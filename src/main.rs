// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! camera-app: the panel's camera, a harry app without a screen of its own. It draws a USB
//! webcam into two zones of the panel (smarthome-app) over Wayland, as harry's multi-process
//! design has it: its looper is a zone client (libhar_platform_android_client), one display
//! per zone, and the panel shows each in the node that names the zone's tag.
//!
//! ```text
//! webcam ─V4L2─► camera thread ─pictures─► pictures.rs (slots) ─► looper ─har_zone─► panel
//!                  ▲      └─state─► reducer ─► presenter ─► UI model ─┘   #zone/camera (tile)
//!   panel ─"live"/"still"─► control socket ◄─"on"/"off"─┘              #zone/camera-live
//! ```
//!
//! The tile shows a still every few seconds; the full view, while the panel shows it, live
//! video. Without a camera both show the offline state, and the camera is looked for again.

use crate::actions::CameraAction;
use crate::presenter::UiModelPresenter;
use crate::resource_loader::ResourceManager;
use crate::state::State;
use crate::ui_event_listener::UiListener;
use crate::ui_model::UiModel;
use har_monitoring::default_performance_monitor::DefaultPerformanceMonitor;
use har_platform_api::commlib::api::DocumentSwitch;
use har_platform_api::commlib::responder::{ChannelResponder, ResponseMonitorThread};
use har_platform_api::graphics::FontInfo;
use har_platform_api::looper::{create_resource_manager_client, DisplayId, Looper, LooperOptions};
use harry::reducer::{Actions, DefaultReducer};
use harry_app_utils::document_loader::{LocalDiskFigmaLoader, ShutdownOnGenericErrorResponseHandler};
use harry_app_utils::layout_helper::HarLayoutHelperManager;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

mod actions;
mod camera;
mod capture;
mod config;
mod control;
mod format;
mod jpeg;
mod meter;
mod picture;
mod pictures;
mod presenter;
mod protocol;
mod resource_loader;
mod state;
mod status;
mod ui_event_listener;
mod ui_model;

/// How often the panel's socket is looked at: for the panel at the start, for a new panel after.
const PANEL_POLL: Duration = Duration::from_secs(1);

fn main() {
    har_log::init_logger("camera-app");
    log::set_max_level(log::LevelFilter::Info);
    let args = config::args();

    if args.probe {
        let devices = match &args.device {
            Some(d) => vec![d.clone()],
            None => capture::uvc_devices(),
        };
        print!("{}", capture::probe(&devices));
        return;
    }
    let Some(max) = config::parse_size(&args.max_size) else {
        eprintln!("--max-size {}: expected WxH", args.max_size);
        std::process::exit(2);
    };

    // The panel serves the zones. Started before it (at boot, or by init again after the panel
    // went away), the app waits until its socket answers: a looper without the panel aborts.
    let wayland = args.wayland_socket();
    let panel = wait_for_panel(&wayland);

    // The control socket first: a second camera app ends here, before it binds the zones (the
    // first one's zones would leave their planes when it unbinds them).
    let (modes_tx, modes_rx) = channel();
    let control = args.control_socket();
    let board = match control::start(&control, modes_tx) {
        Ok(board) => board,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            log::error!("camera: {} is served already ({e}): ending", control.display());
            std::process::exit(1);
        }
        Err(e) => {
            log::error!("camera: no control socket at {} ({e}): stills only, the panel hears nothing", control.display());
            control::Board::none()
        }
    };

    let displays = config::displays(&wayland);
    let performance_monitor = Arc::new(DefaultPerformanceMonitor::new().expect("Error creating performance monitor."));
    // No side effects: the reducer still needs a channel for them.
    let (side_effects_tx, side_effects_rx) = channel::<()>();
    std::thread::spawn(move || for _ in side_effects_rx {});

    let display_root_node_map = displays.iter().map(|d| (d.zone.display(), d.zone.root().to_string())).collect();
    let (actions_tx, actions_rx) = channel();
    let mut layout_helper = HarLayoutHelperManager::new().expect("Failed to create layout helper");
    for (family, weight, data) in resource_loader::FONTS {
        // SAFETY: OpenType fonts are supported.
        unsafe { layout_helper.load_font(vec![FontInfo::new(family, *weight)], data) }
            .expect("Failed to load a font into the layout helper");
    }
    let reducer: DefaultReducer<State, CameraAction, UiModel, (), _, _> = DefaultReducer::new(
        State::default(),
        UiModel::new(display_root_node_map),
        side_effects_tx,
        displays.iter().map(|d| d.zone.display()).collect::<Vec<DisplayId>>(),
        actions_tx,
        actions_rx,
        None,
        Some(performance_monitor),
        Some(layout_helper),
    );
    let actions = reducer.action_tx();
    let (looper_tx, looper_rx) = channel();
    // the zones' pictures: a slot each in the renderers' storage, new pixels through the looper
    let storage = har_platform_api::resource_manager::ResourceManager::new();
    let pictures = pictures::Pictures::new(&storage, create_resource_manager_client(looper_tx.clone()))
        .expect("Cannot make the pictures' slots");

    log::info!("camera: the design from {}", args.data_path);
    let reducer_thread =
        reducer.execute::<UiModelPresenter, _>(looper_tx.clone(), LocalDiskFigmaLoader::new(args.data_path.clone()));
    let (responder, doc_load_response_rx) = ChannelResponder::create_with_receiver();
    actions
        .send(Actions::SwapDocument(DocumentSwitch { document_id: args.document_id.clone() }, responder))
        .expect("Cannot send document load action");
    // Exit if the document cannot be loaded.
    let _document_load_monitor =
        ResponseMonitorThread::new(doc_load_response_rx, ShutdownOnGenericErrorResponseHandler::new(actions.clone(), looper_tx))
            .spawn();

    // The zones: one Wayland connection to the panel each, bound to the zone's tag.
    let mut looper = create_looper(&displays, storage);
    looper.set_ui_event_listener(UiListener::new(actions.clone())).expect("Cannot set the UI event listener");

    camera::spawn(
        camera::Settings {
            device: args.device.clone(),
            max,
            live_fps: args.live_fps,
            still_interval: Duration::from_secs(args.still_interval),
        },
        modes_rx,
        pictures,
        actions,
        board,
    );
    watch_panel(wayland, panel);

    log::info!("camera: started, zones {} and {}", config::Zone::Tile.tag(), config::Zone::Live.tag());
    looper.start(looper_rx).expect("Cannot start UI thread");
    reducer_thread.join().expect("Error reported on the reducer thread.");
}

/// Waits until the panel answers on its socket (a file left by a panel that stopped refuses the
/// connection); the socket's inode, to tell a new panel by.
fn wait_for_panel(socket: &Path) -> u64 {
    let mut logged = false;
    loop {
        // the probe's connection ends at once: the panel sees a client come and go
        let answered = std::os::unix::net::UnixStream::connect(socket).map(drop);
        match answered.and_then(|()| std::fs::metadata(socket)) {
            Ok(m) => {
                if logged {
                    log::info!("camera: the panel answers on {}", socket.display());
                }
                return m.ino();
            }
            Err(e) => {
                if !logged {
                    log::info!("camera: waiting for the panel's socket {} ({e})", socket.display());
                    logged = true;
                }
                std::thread::sleep(PANEL_POLL);
            }
        }
    }
}

/// harry's zone client does not connect again to a panel that went away (the zones stay
/// bound to a closed connection): when the panel's socket is gone or another one, the app ends
/// and its service starts it again, connecting to the new panel.
fn watch_panel(socket: std::path::PathBuf, ino: u64) {
    std::thread::Builder::new()
        .name("camera-panel".to_string())
        .spawn(move || loop {
            std::thread::sleep(PANEL_POLL);
            let now = std::fs::metadata(&socket).map(|m| m.ino()).ok();
            if now != Some(ino) {
                log::warn!("camera: the panel's socket {} is gone or new: ending, to connect again", socket.display());
                std::process::exit(0);
            }
        })
        .expect("spawn the panel watch");
}

fn create_looper(
    displays: &[config::Display],
    storage: har_platform_api::resource_manager::ResourceManager,
) -> impl Looper<ResourceManager> {
    let mut options = displays.iter().map(|d| {
        let (width, height) = d.zone.size();
        LooperOptions {
            display_id: Some(d.zone.display()),
            display_params: d.display_params.clone(),
            width,
            height,
            // Figma px == zone px
            dpi: 160f32,
            root_node_id: d.zone.root().to_string(),
            take_screenshots: false,
            driverui_health_node_id: String::new(),
        }
    });
    let first = options.next().expect("At least one zone");
    let mut looper =
        harry_app_platform_specific::create_looper(first, ResourceManager { storage }).expect("Cannot connect to the panel's zones");
    options.for_each(|o| {
        let id = o.display_id;
        looper.add_display(o).unwrap_or_else(|e| panic!("Cannot add the zone of display {id:?}: {e:?}"));
    });
    looper
}

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

//! The command line, the panel's two zones and where the sockets are.

use clap::Parser as _;
use clap_derive::Parser;
use har_platform_api::looper::DisplayId;
use std::collections::HashMap;
use std::path::PathBuf;

/// The panel's runtime directory when the environment names none (a run from adb).
const DEFAULT_RUNTIME_DIR: &str = "/data/local/tmp";
/// The panel's Wayland socket in its runtime directory (smarthome-app config.rs).
const WAYLAND_SOCKET: &str = "har-display";
/// The control socket in the same directory (control.rs).
const CONTROL_SOCKET: &str = "har-camera";
/// The render node the zones' buffers are allocated on.
const RENDER_NODE: &str = "/dev/dri/renderD128";

#[derive(Parser, Debug)]
pub struct CommandLineArgs {
    /// The camera's device. Unset: the first UVC capture device (/sys/class/video4linux).
    #[arg(long)]
    pub device: Option<PathBuf>,
    /// Directory with the document (.dcf).
    #[arg(short, long, default_value = "/data/local/tmp/camera-app")]
    pub data_path: String,
    /// The design's file in data_path (`<name>.dcf`; a name without the extension is completed).
    #[arg(short = 'i', long, default_value = "camera.dcf")]
    pub document_id: String,
    /// Seconds between two stills in the tile.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=600))]
    pub still_interval: u64,
    /// Frames a second of the full view, at most: the camera's fastest rate up to it.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=60))]
    pub live_fps: u32,
    /// The largest frame taken from the camera, "WxH".
    #[arg(long, default_value = "1280x720")]
    pub max_size: String,
    /// The panel's Wayland socket. Unset: har-display in $XDG_RUNTIME_DIR, else /data/local/tmp.
    #[arg(long)]
    pub wayland_socket: Option<PathBuf>,
    /// The socket the panel connects to (live/still, on/off). Unset: har-camera next to the
    /// Wayland socket.
    #[arg(long)]
    pub control_socket: Option<PathBuf>,
    /// Print the UVC devices with their formats and exit.
    #[arg(long)]
    pub probe: bool,
}

pub fn args() -> CommandLineArgs {
    let mut args = CommandLineArgs::parse();
    // the file name harry opens: <name>.dcf, also when the name came without the extension
    args.document_id = everyware_ui::design::document_file(&args.document_id);
    args
}

pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DEFAULT_RUNTIME_DIR))
}

impl CommandLineArgs {
    pub fn wayland_socket(&self) -> PathBuf {
        self.wayland_socket.clone().unwrap_or_else(|| runtime_dir().join(WAYLAND_SOCKET))
    }

    pub fn control_socket(&self) -> PathBuf {
        self.control_socket.clone().unwrap_or_else(|| self.wayland_socket().with_file_name(CONTROL_SOCKET))
    }
}

/// A zone of the panel this app draws: a display of its looper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// The camera tile of the dashboard: a still every few seconds.
    Tile,
    /// The full view the panel opens from the tile: live video.
    Live,
}

impl Zone {
    pub const ALL: [Zone; 2] = [Zone::Tile, Zone::Live];

    /// The zone tag the panel's node names (har_zone): the contract with smarthome-app.
    pub fn tag(self) -> &'static str {
        match self {
            Zone::Tile => "#zone/camera",
            Zone::Live => "#zone/camera-live",
        }
    }

    /// The root frame of the zone in this app's design (design/camera-patch.js).
    pub fn root(self) -> &'static str {
        match self {
            Zone::Tile => "#camera-tile",
            Zone::Live => "#camera-live",
        }
    }

    /// The zone's size: the panel's node (#camera/feed is 904 x 360) and this app's root frame.
    /// The panel configures the size of a shown node when the zone binds; a node hidden then
    /// (the full view) gets this size.
    pub fn size(self) -> (u32, u32) {
        match self {
            Zone::Tile => (904, 360),
            Zone::Live => (1920, 1080),
        }
    }

    pub fn display(self) -> DisplayId {
        match self {
            Zone::Tile => DisplayId::new(1),
            Zone::Live => DisplayId::new(2),
        }
    }
}

#[derive(Debug)]
pub struct Display {
    pub zone: Zone,
    pub display_params: HashMap<String, String>,
}

/// One display per zone, each a Wayland surface of the panel (the looper of
/// libhar_platform_android_client): the zone's tag, the panel's socket, the render node.
pub fn displays(wayland_socket: &std::path::Path) -> Vec<Display> {
    Zone::ALL
        .iter()
        .map(|&zone| {
            let mut display_params = HashMap::new();
            display_params.insert("zone_tag".to_string(), zone.tag().to_string());
            display_params.insert("wayland_socket".to_string(), wayland_socket.to_string_lossy().into_owned());
            display_params.insert("render_node".to_string(), RENDER_NODE.to_string());
            Display { zone, display_params }
        })
        .collect()
}

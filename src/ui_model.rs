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

//! The UI model: one field per layer of #camera-tile and #camera-live the app writes (design:
//! design/camera-patch.js). The names are the contract with the design: rename both or neither.

use customizations::render_customizations::RenderCustomization;
use customizations::ImageContentData;
use everyware_ui::model::prelude::*;

/// A camera picture for a plain frame of the design, or none (the frame's design fill).
pub type Picture = Option<ImageContentData>;

/// A picture on its frame; without one the frame keeps its fill (dark, under the offline
/// state). harry stretches a picture over the frame: camera.rs cuts it to the frame's aspect.
#[derive(Clone, Debug, Default)]
pub struct PictureCustomizer {}

impl Customizer<Picture> for PictureCustomizer {
    fn format(&mut self, picture: Picture) -> CustomizerOutput {
        match picture {
            Some(image) => image.into(),
            None => CustomizerOutput::new_finished(RenderCustomization::Display(DisplayState::Visible)),
        }
    }
}

everyware_ui::ui_model! {
    // the tile (#zone/camera): the still, the time it was taken, the offline state over it
    tile_picture: Picture = "#camera/tile/picture" ["PictureCustomizer"] = None;
    tile_time_chip: DisplayState = "#camera/tile/time-chip" = DisplayState::Gone;
    tile_time: String = "#camera/tile/time";
    tile_offline: DisplayState = "#camera/tile/offline";
    tile_offline_title: String = "#camera/tile/offline/title";
    tile_offline_detail: String = "#camera/tile/offline/detail";
    // the full view (#zone/camera-live): live video, the offline state over it
    live_picture: Picture = "#camera/live/picture" ["PictureCustomizer"] = None;
    live_offline: DisplayState = "#camera/live/offline";
    live_offline_title: String = "#camera/live/offline/title";
    live_offline_detail: String = "#camera/live/offline/detail";
}

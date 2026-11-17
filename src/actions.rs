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

//! What the camera thread tells the screen. Only changes the zones' frames need: a picture
//! that is new in its zone, the time of a still, the camera's state. A frame of the same size
//! goes to the looper directly (camera.rs) and never through here.

use crate::config::Zone;
use crate::state::State;
use crate::status::Status;
use customizations::ImageContentData;
use harry::side_effects::SideEffects;
use harry::state::StateAction;

#[derive(Clone, Debug, PartialEq)]
pub enum CameraAction {
    /// The picture of a zone, new there (or none: the zone shows its design fill).
    Picture(Zone, Option<ImageContentData>),
    /// A still was shown in the tile, at this local time ("HH:MM:SS").
    Still(String),
    /// The camera's state changed.
    Status(Status),
}

impl StateAction<State, ()> for CameraAction {
    fn apply(&self, state: &mut State, _side_effects: &mut Vec<SideEffects<()>>) {
        match self {
            CameraAction::Picture(Zone::Tile, p) => state.tile = p.clone(),
            CameraAction::Picture(Zone::Live, p) => state.live = p.clone(),
            CameraAction::Still(time) => state.still_at = time.clone(),
            CameraAction::Status(s) => state.status = s.clone(),
        }
    }
}

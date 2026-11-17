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

//! The looper's events. The zones take no touch: the panel's buttons over them (open, close)
//! are the panel's, and the app subscribes to none of the zone's touches (zone_input). Only
//! the drawn frames matter: the reducer paces its frames by them (harry's FramePacing).

use crate::actions::CameraAction;
use har_platform_api::looper::{DisplayId, HandleEventStatus, UiEventListener};
use har_platform_api::user_input::UserInputEvent;
use harry::reducer::Actions;
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct UiListener {
    actions: Sender<Actions<CameraAction>>,
}

impl UiListener {
    pub fn new(actions: Sender<Actions<CameraAction>>) -> Self {
        Self { actions }
    }
}

impl UiEventListener for UiListener {
    fn handle_event(&mut self, _event: UserInputEvent) -> HandleEventStatus {
        HandleEventStatus::NotConsumed
    }

    fn frame_presented(&mut self, _display_id: DisplayId) {
        let _ = self.actions.send(Actions::FramePresented);
    }
}

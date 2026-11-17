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

//! State -> UI model, after every action.

use crate::state::State;
use crate::ui_model::UiModel;
use customizations::render_customizations::DisplayState;
use harry::presentation::Presenter;

pub struct UiModelPresenter;

impl Presenter<State, UiModel> for UiModelPresenter {
    fn present(model: &mut UiModel, state: &State) {
        let shown = |b: bool| if b { DisplayState::Visible } else { DisplayState::Gone };
        let title = state.status.title();
        let offline = title.is_some();
        let (title, detail) = (title.unwrap_or_default().to_string(), state.status.detail().to_string());

        model.tile_picture = state.tile.clone();
        model.tile_time = state.still_at.clone();
        model.tile_time_chip = shown(!offline && !state.still_at.is_empty());
        model.tile_offline = shown(offline);
        model.tile_offline_title = title.clone();
        model.tile_offline_detail = detail.clone();

        model.live_picture = state.live.clone();
        model.live_offline = shown(offline);
        model.live_offline_title = title;
        model.live_offline_detail = detail;
    }
}

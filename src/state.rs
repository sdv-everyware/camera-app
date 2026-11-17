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

//! Everything the zones show: the pictures, the time of the tile's still, the camera's state.

use crate::status::Status;
use customizations::ImageContentData;

pub struct State {
    pub status: Status,
    /// The tile's picture, once the first still came.
    pub tile: Option<ImageContentData>,
    /// The full view's picture while live.
    pub live: Option<ImageContentData>,
    /// When the tile's still was taken, "HH:MM:SS"; empty before the first.
    pub still_at: String,
}

impl Default for State {
    fn default() -> Self {
        State { status: Status::Starting, tile: None, live: None, still_at: String::new() }
    }
}

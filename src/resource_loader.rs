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

//! The renderer's resources: the pictures' storage (pictures.rs, where the camera frames go)
//! and the fonts, which the layout helper that measures text gets too (main.rs).

use har_platform_api::error::Error;
use har_platform_api::graphics::{FontInfo, Renderer};
use har_platform_api::looper::RendererHelper;
use har_platform_api::resource_manager::ResourceManager as Storage;

const INTER_REGULAR: &[u8] = include_bytes!("../data/inter_regular.otf");
const INTER_BOLD: &[u8] = include_bytes!("../data/inter_bold.otf");

pub const FONTS: &[(&str, u16, &[u8])] =
    &[("Inter", 400, INTER_REGULAR), ("Inter", 500, INTER_REGULAR), ("Inter", 600, INTER_BOLD)];

/// What every renderer gets: the pictures' storage and the fonts.
pub struct ResourceManager {
    /// The camera's pictures; a clone shares the storage, so both displays' renderers and
    /// pictures.rs see the same slots.
    pub storage: Storage,
}

impl RendererHelper for ResourceManager {
    fn initialize_renderer(&self, renderer: &mut dyn Renderer) -> Result<(), Error> {
        renderer.replace_raw_image_storage(self.storage.clone());
        for (family, weight, data) in FONTS {
            // SAFETY: OpenType fonts are supported.
            unsafe { renderer.load_font(vec![FontInfo::new(family, *weight)], data)? };
        }
        Ok(())
    }
}

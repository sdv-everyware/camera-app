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

//! The zones' pictures, as harry's reference app keeps its camera preview (reference/harry-app
//! main.rs): one slot per zone in a raw image storage of the app's own, which the renderers of
//! both displays share (resource_loader.rs), and new pixels for a slot through the looper's
//! resource client. The looper puts them into the storage and redraws the displays that show
//! the slot, uploading the texture at the picture's own size; the reducer is not involved.
//!
//! At most one picture per zone is on its way to the looper: the next one goes when the storage
//! holds the last one sent. A looper that falls behind (a slow panel) gets fewer frames, not a
//! queue of 3.7 MB frames. The pixels of a picture the storage let go of come back as the
//! buffer of a later frame (`spare`): a live frame allocates nothing.

use crate::config::Zone;
use customizations::ImageContentData;
use har_platform_api::looper::ResourceManagerClient;
use har_platform_api::resource_manager::ResourceManager;
use har_types::resources::{RawImageData, ResourceToken};
use std::sync::Arc;

/// The full view's picture while it is closed: one pixel of the design's dark, stretched.
const DARK: [u8; 4] = [0x1d, 0x27, 0x33, 0xff];

struct Slot {
    token: ResourceToken,
    /// The pixels of the last picture sent, until the storage holds them.
    sent: Option<Arc<Vec<u8>>>,
    /// The size the zone's frame was told (its ImageContentData).
    told: Option<(u32, u32)>,
    /// Pictures sent before the last one, until the storage and the renderers let go of them.
    retired: Vec<Arc<Vec<u8>>>,
}

/// Pictures kept for their buffers at most: the storage holds one, the looper one on its way.
const RETIRED: usize = 3;

pub struct Pictures {
    storage: ResourceManager,
    client: ResourceManagerClient,
    slots: [Slot; 2],
}

impl Pictures {
    /// A slot per zone in `storage` (the renderers' storage: a clone shares it).
    pub fn new(storage: &ResourceManager, client: ResourceManagerClient) -> Result<Pictures, String> {
        let mut storage = storage.clone();
        let mut slot = || -> Result<Slot, String> {
            Ok(Slot { token: storage.add_resource(RawImageData::default())?, sent: None, told: None, retired: Vec::new() })
        };
        let slots = [slot()?, slot()?];
        Ok(Pictures { storage, client, slots })
    }

    /// Whether a new picture of `zone` may go: the looper took the last one.
    pub fn ready(&self, zone: Zone) -> bool {
        let slot = &self.slots[zone as usize];
        match &slot.sent {
            None => true,
            Some(sent) => self.storage.get_resource(slot.token).is_some_and(|r| Arc::ptr_eq(&r.data, sent)),
        }
    }

    /// The buffer of an earlier picture of `zone` that nobody holds any more, for the next frame.
    pub fn spare(&mut self, zone: Zone) -> Option<Vec<u8>> {
        let retired = &mut self.slots[zone as usize].retired;
        let i = retired.iter().position(|a| Arc::strong_count(a) == 1)?;
        Arc::try_unwrap(retired.swap_remove(i)).ok()
    }

    /// Sends a picture of `zone`. Some(content) when the zone's frame must be told (its first
    /// picture, another size): the caller hands it to the reducer.
    pub fn send(&mut self, zone: Zone, rgba: Vec<u8>, width: u32, height: u32) -> Result<Option<ImageContentData>, String> {
        let data = Arc::new(rgba);
        let slot = &mut self.slots[zone as usize];
        self.client.update(slot.token, RawImageData::from_arc_data(data.clone(), width, height)).map_err(|e| format!("{e:?}"))?;
        if let Some(old) = slot.sent.replace(data) {
            slot.retired.push(old);
            if slot.retired.len() > RETIRED {
                slot.retired.remove(0);
            }
        }
        if slot.told == Some((width, height)) {
            return Ok(None);
        }
        slot.told = Some((width, height));
        Ok(Some(ImageContentData::new(slot.token, width, height)))
    }

    /// The full view closed: its slot gets one dark pixel, so that it opens dark, not on the
    /// last frame of the time before. Sent whether or not the looper took the last frame.
    pub fn darken(&mut self, zone: Zone) -> Result<Option<ImageContentData>, String> {
        self.send(zone, DARK.to_vec(), 1, 1)
    }
}

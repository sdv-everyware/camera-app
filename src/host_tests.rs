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

//! The modules that need neither the camera nor the panel, as host tests
//! (`m camera_app_host_test`): the pictures of a clip and its pace, the choice of the format and
//! its rates, the cut to a zone's aspect and YUYV, the control socket (a socket in the temp
//! directory) with its lines and the live demand, the still schedule, broken frames, the offline
//! words and the live view's meter.

#[allow(dead_code)]
mod clip;
#[allow(dead_code)]
mod control;
#[allow(dead_code)]
mod format;
#[allow(dead_code)]
mod meter;
#[allow(dead_code)]
mod picture;
#[allow(dead_code)]
mod protocol;
#[allow(dead_code)]
mod status;

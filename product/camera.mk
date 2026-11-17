# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# The panel's camera (camera-app, README.md) in a product: a board product with a USB camera
# inherits this file (device-rb3). The image gets camera_app and its design on the vendor
# partition and its service in /vendor/etc/init/everyware/, a directory the board's root init
# imports (an image without androidboot.init_rc reads /vendor/etc/init on its own, not the
# directory: such a board imports it). The board's policy gives camera_app its domain
# everyware_camera.
PRODUCT_PACKAGES += \
    camera_app \
    camera_app_dcf
PRODUCT_COPY_FILES += \
    packages/services/display_safety/utils/camera-app/product/everyware_camera.rc:$(TARGET_COPY_OUT_VENDOR)/etc/init/everyware/everyware_camera.rc

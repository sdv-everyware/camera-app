#!/bin/bash
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

# This app's design, end to end: camera-patch.js on ui-lib's library-base.fig (a document with
# only the icon sources: the zones use no Library component), then data/camera.dcf with both
# roots (fig-to-dcf.sh, the previous one kept next to it). Usage, from anywhere:
#
#   design/build.sh                     # -> design/camera.fig and data/camera.dcf
#
# design/camera.fig is the result, in git: the zones as a designer opens them in Figma.
# The tools are everyware's design/figma (EVERYWARE_FIGMA) and scripts/fig-to-dcf.sh, the base
# ui-lib's (EVERYWARE_UI), all found in the tree by default. Needs node, unzip, zip and the
# tree's importer (cargo).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
TOP=$(cd "$HERE/../../../../../.." && pwd)
FIGMA=${EVERYWARE_FIGMA:-$TOP/vendor/sdv-everyware/everyware/design/figma}
UI=${EVERYWARE_UI:-$TOP/vendor/sdv-everyware/ui-lib}
FIG="$HERE/camera.fig"
node "$FIGMA/roundtrip.js" "$UI/design/library-base.fig" "$FIG" "$HERE/camera-patch.js"
echo "built $FIG"
DCF_ROOT='#camera-tile' DCF_NODES='#camera-live' \
DCF_CHECK='#camera-live #camera/tile/picture #camera/tile/time-chip #camera/tile/time #camera/tile/offline #camera/tile/offline/title #camera/tile/offline/detail #camera/live/picture #camera/live/offline #camera/live/offline/title #camera/live/offline/detail' \
    "$TOP/vendor/sdv-everyware/everyware/scripts/fig-to-dcf.sh" "$FIG" "$HERE/../data/camera.dcf"
# the last line: the file to give the user for a review
echo "review in Figma (File > Import): $FIG"

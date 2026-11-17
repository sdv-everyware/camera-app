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

// camera-patch.js: the two zones of camera-app, each a root frame the size of the panel's node
// that shows it. The camera's picture fills a plain frame (harry draws a picture only there);
// over it the time of the still (the tile) and the offline state (both). The panel draws its
// own parts over the zones (the LIVE chip at the top left, the caption and the open button at
// the bottom of the tile), so the tile keeps the top left and the bottom free. The app
// (src/ui_model.rs) drives the #camera/* names.
//
//   design/build.sh     # this patch on ui-lib's library-base.fig, then data/camera.dcf

// not the Library's palette: the dark of a camera picture that is not there yet
const DARK = [0x1d / 255, 0x27 / 255, 0x33 / 255];
const WHITE = [1, 1, 1];
// the zones: the panel's #camera/feed (904 x 360) and its full view (the whole screen)
const TILE = { w: 904, h: 360 }, LIVE = { w: 1920, h: 1080 };

// roundtrip.js hands over fig-helpers.js (the node authoring) as the second argument
module.exports = function (msg, { FONT, authoring }) {
  const A = authoring(msg, 'camera');
  const { nodes, frame, rect, ellipse, text, base, at, log } = A;
  const page = nodes.find(n => n.type === 'CANVAS' && n.name === 'Page 1');
  if (!page) throw new Error('no Page 1: build on ui-lib design/library-base.fig');
  for (const name of ['#camera-tile', '#camera-live']) {
    if (nodes.some(n => n.name === name)) throw new Error(`${name} exists already`);
  }
  const solid = rgb => [{ type: 'SOLID', color: { r: rgb[0], g: rgb[1], b: rgb[2], a: 1 }, opacity: 1, visible: true, blendMode: 'NORMAL' }];

  // A camera with a slash through it, `s` wide, centred at (cx, cy): filled shapes only (the
  // body, a dark lens, the slash cut out of the body by a dark bar under a white one).
  const cameraOff = (parent, cx, cy, s) => {
    const g = frame(parent, 'Camera off', cx - s / 2, cy - s / 2, s, s);
    const bw = s * 0.62, bh = s * 0.44, bx = s * 0.08, by = (s - bh) / 2;
    rect(g, 'body', bx, by, bw, bh, WHITE, s * 0.08);
    // the lens of a video camera: a wedge to the right of the body
    base(g, 'ROUNDED_RECTANGLE', 'lens', 0, 0, s * 0.2, s * 0.2, { fillPaints: solid(WHITE), cornerRadius: s * 0.03,
      transform: at(bx + bw + s * 0.1, s / 2 - s * 0.142, 45) });
    const len = s * 1.0, th = s * 0.07, angle = 45;
    const bar = (name, rgb, thick) => {
      const r = angle * Math.PI / 180;
      const ox = s / 2 - (Math.cos(r) * len / 2 - Math.sin(r) * thick / 2), oy = s / 2 - (Math.sin(r) * len / 2 + Math.cos(r) * thick / 2);
      base(g, 'ROUNDED_RECTANGLE', name, 0, 0, len, thick, { fillPaints: solid(rgb), cornerRadius: thick / 2, transform: at(ox, oy, angle) });
    };
    bar('cut', DARK, th * 3);
    bar('slash', WHITE, th);
    return g;
  };

  // The offline state of a zone: a dark cover with the icon, the headline and the line under it.
  const offline = (root, prefix, w, h, top, icon, title, detail) => {
    const o = frame(root, `${prefix}/offline`, 0, 0, w, h, { fillPaints: solid(DARK) });
    cameraOff(o, w / 2, top + icon / 2, icon);
    text(o, `${prefix}/offline/title`, 0, top + icon + icon * 0.3, w, title * 1.3, 'Camera offline', title, FONT.semibold, WHITE, 'CENTER');
    text(o, `${prefix}/offline/detail`, 0, top + icon + icon * 0.3 + title * 1.6, w, detail * 1.3, 'No camera is connected', detail,
      FONT.regular, WHITE, 'CENTER', 'TOP', { opacity: 0.72 });
    return o;
  };

  // ---- the tile (#zone/camera): the still, its time at the top right, the offline state
  const tile = frame(page, '#camera-tile', 0, 0, TILE.w, TILE.h, { fillPaints: solid(DARK), frameMaskDisabled: false });
  frame(tile, '#camera/tile/picture', 0, 0, TILE.w, TILE.h, { fillPaints: solid(DARK) });
  // the panel's LIVE chip is 74 x 34 at (24, 24): the time chip mirrors it at the top right
  const chip = frame(tile, '#camera/tile/time-chip', TILE.w - 24 - 104, 24, 104, 34);
  rect(chip, 'bg', 0, 0, 104, 34, [0, 0, 0], 17, { opacity: 0.45 });
  text(chip, '#camera/tile/time', 0, 0, 104, 34, '11:28:20', 15, FONT.medium, WHITE, 'CENTER', 'CENTER');
  // above the panel's caption, which starts 273 px down the tile
  offline(tile, '#camera/tile', TILE.w, TILE.h, 64, 64, 24, 16);
  log('#camera-tile 904 x 360: #camera/tile/picture, /time-chip, /time, /offline (title, detail)');

  // ---- the full view (#zone/camera-live): the live picture, the offline state
  const live = frame(page, '#camera-live', 0, TILE.h + 120, LIVE.w, LIVE.h, { fillPaints: solid(DARK), frameMaskDisabled: false });
  frame(live, '#camera/live/picture', 0, 0, LIVE.w, LIVE.h, { fillPaints: solid(DARK) });
  offline(live, '#camera/live', LIVE.w, LIVE.h, 380, 112, 40, 24);
  log('#camera-live 1920 x 1080: #camera/live/picture, /offline (title, detail)');
};

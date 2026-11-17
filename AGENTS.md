# camera-app: the guide for coding agents

The panel's camera: a harry app without a screen of its own that draws a USB webcam into two
zones of the panel (the tile and the full view) over Wayland. Its own git repository in the tree
at `packages/services/display_safety/utils/camera-app`. The project's guide (the repositories,
every skill, the rules for every change) is `vendor/sdv-everyware/everyware/AGENTS.md`: read it
first.

## What is where

| Path | What |
|---|---|
| `src/main.rs` | the looper as a zone client (one display per zone), the reducer, the threads |
| `src/config.rs` | the command line, the zones (tag, root, size: the contract with the panel), the sockets |
| `src/camera.rs` | the camera thread: open, stream, stills and live, the offline state |
| `src/pictures.rs` | the zones' picture slots in the renderers' storage, one frame in flight per zone |
| `src/capture.rs` | V4L2 through v4l2r: the UVC device, its formats, the buffers, the newest frame |
| `src/format.rs`, `src/picture.rs` | which format and rates; the cut to a zone's aspect, YUYV to RGBA |
| `src/jpeg.rs`, `src/jpeg_shim.c` | MJPEG to RGBA with libjpeg-turbo (its error jumps stay in C) |
| `src/control.rs`, `src/protocol.rs` | the control socket: live/still from the panel, on/off to it |
| `src/status.rs` | the camera's state in the zones' words; the still schedule; broken frames |
| `src/state.rs`, `actions.rs`, `presenter.rs`, `ui_model.rs` | harry's flow; the `#camera/*` names |
| `design/camera-patch.js`, `design/build.sh` | the two roots, `design/camera.fig`, `data/camera.dcf` |
| `product/camera.mk`, `product/everyware_camera.rc` | camera_app, its design and its service in a board's image (README, How it starts) |

The other side is smarthome-app (`src/camera.rs`, `design/camera-patch.js`): the nodes that show
`#zone/camera` and `#zone/camera-live`, the LIVE chips and the control socket's client. The
domain everyware_camera is device-rb3's (`sepolicy/vendor/everyware_camera.te`). The zones' tags and sizes and the socket's lines are
shared: change both sides.

## Rules

- A camera frame goes to the looper only (`src/pictures.rs`), never through the reducer: a
  reducer frame per camera frame is a full pre-render.
- Logic that needs neither the camera nor the panel is a module with host tests
  (`src/host_tests.rs`, `m camera_app_host_test`).
- A design's names are a contract with `src/ui_model.rs`: change both in one commit.
- On the board: say what each command does and whether it changes state; the panel is init's.

A change lands through its own branch, a review by another agent and the user's word
(`vendor/sdv-everyware/everyware/AGENTS.md`, "How a change lands"; `scripts/feature.sh` there).
The hooks in `scripts/git-hooks/` refuse a commit on `main`, by another author, with something
private in it or with an AI attribution.

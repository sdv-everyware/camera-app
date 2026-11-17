# camera-app

The smart home panel's camera: `camera_app` shows a USB webcam in the panel's camera tile and,
when the panel opens it, live in a full view. It is a harry app without a screen of its own:
its looper is a Wayland zone client (`libharry_app_platform_specific_client`), and the panel
(`smarthome-app`, the HAR master) shows its two displays in the nodes that name their zones.

```
 webcam ─V4L2 (MJPEG/YUYV)─► camera thread ─RGBA─► pictures.rs (a slot per zone) ─► looper (2 displays)
                               │                                            │ har_zone, dmabuf
                               └─state─► reducer ─► presenter ─► UI model ──┘
                                                                            ▼
 panel (smarthome_app)  #camera/feed ◄─── #zone/camera       (904 x 360, a still every 2 s)
                        full view    ◄─── #zone/camera-live  (1920 x 1080, live while shown)
   tile's ↗ / close ──"live" / "still"──► control socket har-camera ──► camera thread
   LIVE chip       ◄──"on" / "off"──────┘
```

- **The tile**: a still every `--still-interval` seconds (2), cut to the tile's aspect (the
  middle of the picture, never stretched), with the time it was taken at the top right. The
  stream runs at the camera's slowest rate from 5 frames a second.
- **The full view**: while the panel says `live`, every frame goes to `#zone/camera-live`, at
  the camera's fastest rate up to `--live-fps` (30). When the panel says `still` or closes its
  connection, the camera goes back to stills and the full view to its dark fill.
- **No camera**: both zones show "Camera offline" with the reason (none connected, busy, no
  permission, no pictures, format not supported), the panel hears `off`, and the camera is
  looked for every 2 s: a replugged camera comes back by itself.

A camera frame never goes through the reducer: the camera thread sends its pixels to the slot of
its zone (`src/pictures.rs`, as harry's reference app does with its camera preview:
`ResourceManagerClient::update`), and the looper redraws only the display that shows the slot.
At most one frame per zone is on its way to the looper; a slow panel gets fewer frames. The
reducer runs for a picture's first size, a still's time and a change of the state. Closing the
full view puts one dark pixel into its slot, so it opens dark next time, not on an old frame.

## The camera

`camera_app` takes the first UVC device that captures video (`/sys/class/video4linux/video*`
with the driver `uvcvideo`; a UVC camera's metadata node and the SoC's codecs are skipped), or
`--device`. Of its formats it takes the largest picture up to `--max-size` (1280x720) that
comes at 15 frames a second or more, MJPEG before YUYV (`src/format.rs`). MJPEG is decoded with
libjpeg-turbo (`src/jpeg_shim.c`, the tree's external/libjpeg-turbo), YUYV converted in
`src/picture.rs`. A frame the driver or the decoder marks as broken is dropped. The camera keeps
its own exposure: in a dim room this webcam sends 25 frames a second where it offers 30 (its
"exposure, auto priority"), and the live view shows each of them. The thread that decodes asks
the scheduler for half a core's capacity (uclamp.min, `sched_setattr`): on a little core at its
idle clock a 720p frame takes 25 ms, on a big one 5 to 10.

```bash
adb shell /data/local/tmp/camera-app/camera_app --probe    # the UVC devices and their formats
```

The camera's node is not fixed: on the RB3 the SoC's codec takes `video0` and `video1` after a
cold boot and the webcam comes as `video2` (its metadata node `video3`). The app looks the camera
up again every 2 s while it has none, so a camera plugged in later is picked up.

Without a webcam: `--device /dev/videoN` takes any V4L2 capture node that offers MJPEG or YUYV,
for example a v4l2loopback device that ffmpeg feeds with a clip (`-f v4l2 -pix_fmt yuyv422`); the
search without `--device` takes UVC devices only. The RB3's kernel has neither v4l2loopback nor
vivid, so there the test needs the webcam; harry's own emulated camera
(`reference/camera/emulated-camera`) serves harry's camera API, which camera_app does not use
(below, "Why not harry's camera API").

A limitation of the RB3, not of the app: its USB controller `a800000` does not see a device
plugged in after boot. The device appears once the controller is probed again (as root:
`echo a8f8800.usb > /sys/bus/platform/drivers/dwc3-qcom/unbind`, then the same into `bind`),
and then the app takes it.

## Why not harry's camera API

harry has a camera path of its own: `har_platform_api::camera` (`ICameraManager`,
`ICameraDevice`) with the V4L2 implementation in `reference/platforms/har-platform-android/src/
camera/implementation.rs`; its frames reach a scene the same way camera_app's do, as pixels of a
resource token (`ResourceManagerClient::update`, an image node). camera_app uses that last part
(`src/pictures.rs`) and the same V4L2 crate (v4l2r), but captures by itself, because the
implementation cannot do what the panel needs without a change of harry:

- no MJPEG: it converts AB24, AR24, BA24, RX24, XR24, NV12, RGB3 and YUYV only, and keeps the
  device's current format (a UVC camera's first, MJPEG on this webcam, whose YUYV ends at 800x480);
- a fixed 640 x 360 picture, whatever the camera and the zone;
- an unplugged camera ends the process: `dqbuf(..).expect(..)` in the capture thread, and Soong
  builds with `panic=abort`;
- the frame rate is a sleep after each frame (the stream runs at the camera's rate, the driver's
  queue makes the pictures late), not `VIDIOC_S_PARM`; no still rate;
- the devices are listed once, by opening every `/dev/video*` (the SoC's codecs too), so a camera
  plugged in later is not seen until the list is empty.

The changes harry would need, small and upstreamable: MJPEG (libjpeg-turbo) and the format's
choice in `start`, the dequeue's errors as stream events, the rate by `VIDIOC_S_PARM`, the newest
frame instead of a queue, the devices by capability and again on request. With them camera_app
could take its frames from `ICameraDevice`.

## The control socket

`camera_app` listens on `har-camera` next to the panel's Wayland socket (`$XDG_RUNTIME_DIR`,
without one `/data/local/tmp`; `--control-socket`), a unix stream socket, rw for its user and
group. One word a line (`src/protocol.rs`): the panel writes `live` or `still`, the app writes
`on` or `off` at the connection's start and at every change. A connection that asked for `live`
keeps the camera live until it says `still` or closes. Any other line ends the connection.

```bash
# by hand, as root on the board (toybox nc speaks unix sockets with -U)
adb shell 'echo live | nc -U /data/vendor/everyware/run/har-camera'
```

## The panel

The panel's side is smarthome-app's `src/camera.rs` and `design/camera-patch.js`: the tile's
`#camera/feed` shows zone `#zone/camera` (`--wayland-zones`), the full view `#camera-view` shows
zone `#zone/camera-live`, the LIVE chips follow `on`/`off`, and the panel connects to the control
socket. When the panel goes away (its Wayland socket is gone or new), `camera_app` ends: harry's
zone client does not connect again, and its service starts it again (How it starts).

## The design

`design/camera-patch.js` draws the two roots, `#camera-tile` (904 x 360) and `#camera-live`
(1920 x 1080): a plain frame for the picture, the time chip, the offline state. Their sizes are
the panel's nodes. A zone client keeps the size of its first configure (a later one is only
logged), and a zone hidden when it binds (the full view) gets no configure and renders the size
asked for (`config.rs`, `Zone::size`). harry clean/cut2 draws a zone with the node's size as the
source rect, so there a buffer of another size lands 1:1 in the corner; super-harry 0960c70
(feature/zone-scale, in the RB3's image) scales it into the node. With the sizes equal both
show the same.

The picture frames are plain rectangles (radius 0): harry draws a picture or a zone only on a
plain frame (the display list's RoundRect arm has neither, `generation.rs`), so a rounded corner
comes from a clipping parent, as the panel's tile does.

```bash
packages/services/display_safety/utils/camera-app/design/build.sh   # design/camera.fig, data/camera.dcf
```

## Build

```bash
# from the tree's top (bash: env.sh uses BASH_SOURCE)
bash -c 'source device/rb3/sdv_media_db845c/tools/env.sh >/dev/null && m camera_app camera_app_host_test'
out_rb3/host/linux-x86/nativetest64/camera_app_host_test/camera_app_host_test
```

## How it starts

A board with a USB camera inherits `product/camera.mk` (device-rb3 does): the image gets
`camera_app` (`/vendor/bin`), its design (`/vendor/etc/camera-app/camera.dcf`) and the service
`everyware_camera` (`product/everyware_camera.rc`, installed in `/vendor/etc/init/everyware/`, a
directory the board's root init imports). init starts it on `init.svc.smarthome=running`, as root
with the group `camera` (`/dev/video*`) and `XDG_RUNTIME_DIR=/data/vendor/everyware/run`: the
panel's Wayland socket there is root's alone. `camera_app` waits until that socket answers, and
ends when the panel goes away; init starts it again.

It runs in the SELinux domain `everyware_camera` (device-rb3 `sepolicy/vendor/everyware_camera.te`):
`/dev/video*` and sysfs to find the camera, the GPU's render node, its design, the panel's Wayland
socket, its control socket in the runtime directory (labeled `everyware_camera_socket`, which the
panel may connect to) and `setsched` for its decode thread; no network. The panel's other
processes and their domains: everyware's `docs/concepts/9-starting-the-app.md`.

## Run (RB3, from adb)

The service runs it in the image (`adb logcat -s camera-app`; `stop everyware_camera`, `start
everyware_camera` as root). A build of your own runs by hand as root after `stop
everyware_camera` (two camera apps refuse each other):

```bash
adb root
adb shell stop everyware_camera
adb shell mkdir -p /data/local/tmp/camera-app
adb push out_rb3/target/product/sdv_media_db845c/vendor/bin/camera_app /data/local/tmp/camera-app/
adb push packages/services/display_safety/utils/camera-app/data/camera.dcf /data/local/tmp/camera-app/
adb shell 'cd /data/local/tmp/camera-app; XDG_RUNTIME_DIR=/data/vendor/everyware/run \
    LD_LIBRARY_PATH=/system/lib64:/vendor/lib64 nohup ./camera_app > camera-app.log 2>&1 &'
adb logcat -s camera-app
```

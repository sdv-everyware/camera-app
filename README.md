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
 panel (smarthome_app)  #camera/feed ◄─── #zone/camera       (904 x 360, a still every 5 s)
                        full view    ◄─── #zone/camera-live  (1920 x 1080, live while shown)
   tile's ↗ / close ──"live" / "still"──► control socket har-camera ──► camera thread
   LIVE chip       ◄──"on" / "off"──────┘
```

- **The tile**: a still every `--still-interval` seconds (5), cut to the tile's aspect (the
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
`src/picture.rs`. A frame the driver or the decoder marks as broken is dropped.

```bash
adb shell /data/local/tmp/camera-app/camera_app --probe    # the UVC devices and their formats
```

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

The panel's side is not in this repository. It needs: the tile's `#camera/feed` as zone
`#zone/camera` (`--wayland-zones`, there today), a full view with a frame showing zone
`#zone/camera-live`, the LIVE chip bound to `on`/`off`, and a connection to the control socket.
When the panel goes away (its Wayland socket is gone or new), `camera_app` ends: harry's zone
client does not connect again, and its service starts it again.

## The design

`design/camera-patch.js` draws the two roots, `#camera-tile` (904 x 360) and `#camera-live`
(1920 x 1080): a plain frame for the picture, the time chip, the offline state. Their sizes are
the panel's nodes and must stay so: harry draws a zone with the node's size as the source rect
(a buffer of another size lands 1:1 in the corner), and a zone client keeps the size of its
first configure (a later one is only logged). A zone hidden when it binds (the full view) gets
no configure and renders the size asked for (`config.rs`, `Zone::size`).

```bash
packages/services/display_safety/utils/camera-app/design/build.sh   # design/camera.fig, data/camera.dcf
```

## Build

```bash
# from the tree's top (bash: env.sh uses BASH_SOURCE)
bash -c 'source device/rb3/sdv_media_db845c/tools/env.sh >/dev/null && m camera_app camera_app_host_test'
out_rb3/host/linux-x86/nativetest64/camera_app_host_test/camera_app_host_test
```

## Run (RB3, from adb)

The panel runs as root with `--wayland-zones`; its runtime directory
`/data/vendor/everyware/run` is root's, so `camera_app` runs as root too. Nothing starts it in
the image yet: a service and a SELinux domain for it are a proposal, not made.

```bash
adb root
adb shell mkdir -p /data/local/tmp/camera-app
adb push out_rb3/target/product/sdv_media_db845c/system/bin/camera_app /data/local/tmp/camera-app/
adb push packages/services/display_safety/utils/camera-app/data/camera.dcf /data/local/tmp/camera-app/
adb shell 'cd /data/local/tmp/camera-app; XDG_RUNTIME_DIR=/data/vendor/everyware/run \
    LD_LIBRARY_PATH=/system/lib64:/vendor/lib64 nohup ./camera_app > camera-app.log 2>&1 &'
adb logcat -s camera-app
```

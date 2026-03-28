# ADR-011: Local Display Mode

## Status
Accepted

## Context
WayRay is designed as a remote thin client compositor, but as OpenIndiana's flagship Wayland experience it must also work as a **local desktop compositor** -- apps and display on the same machine. The "but can I just use it locally?" question is day-one feedback.

### The illumos Display Problem

Getting pixels on screen on illumos is constrained:

| Path | Status on illumos |
|------|-------------------|
| DRM/KMS | Intel Gen2-7 only (ancient). No AMD, no modern Intel. |
| illumos `/dev/fb0` (fbio) | **Works.** UEFI GOP framebuffer via `gfxp_bitmap` driver. Userspace mmap + write pixels directly. Resolution fixed at boot. |
| illumos VIS console ops | Kernel-only (`VIS_CONSDISPLAY` etc. check `FKIOCTL`). Not for userspace rendering. |
| X11 + illumosfb DDX | Works on UEFI GOP systems. Uses `/dev/fb0` directly. See [xf86-video-illumosfb](https://github.com/LuminousMonkey/xf86-video-illumosfb). |
| X11 + VESA DDX | Works on any GPU via VBE BIOS calls. CPU-rendered. |
| X11 + i915 DDX | Works on Intel Gen2-7 with DRI acceleration. |
| X11 + NVIDIA proprietary | Works with specific driver versions. |
| Mesa llvmpipe | Software OpenGL available everywhere. |

**Two universal display paths exist on illumos:**
1. **`/dev/fb0` bare-metal** -- direct framebuffer access on UEFI GOP systems (no X11 needed)
2. **X11** -- works everywhere including legacy BIOS via VESA DDX

### illumos `/dev/fb0` Details

The `gfxp_bitmap` kernel driver (backing the `vgatext` DDI driver) exposes the UEFI GOP framebuffer via the classic SunOS `fbio(4I)` interface:

```c
fd = open("/dev/fb0", O_RDWR);
ioctl(fd, VIS_GETIDENTIFIER, &ident);   // -> "illumos_fb"
ioctl(fd, FBIOGATTR, &attr);            // -> struct fbgattr with:
                                         //    resolution, depth, size
                                         //    pitch + RGB masks via gfxfb_info
                                         //    in sattr.dev_specific[]
buf = mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);
ioctl(fd, KDSETMODE, KD_GRAPHICS);      // take over from console
// write pixels directly to buf (write-combining memory)
// use non-temporal stores (SSE2/AVX2/AVX-512) for performance
ioctl(fd, KDSETMODE, KD_TEXT);           // release back to console
```

**Constraints:**
- UEFI GOP only (legacy BIOS falls back to VGA text mode)
- Resolution fixed at boot time (no mode switching -- whatever GOP configured)
- Write-combining memory mapping requires non-temporal stores for performance
- `struct gfxfb_info` (pitch, RGB layout) smuggled through `fbsattr.dev_specific[8]`

**Source references:**
- `illumos-gate/usr/src/uts/common/sys/fbio.h` -- ioctl definitions, `gfxfb_info`
- `illumos-gate/usr/src/uts/i86pc/io/gfx_private/gfxp_bitmap.c` -- bitmap FB backend
- `illumos-gate/usr/src/uts/i86pc/io/gfx_private/gfxp_fb.c` -- ioctl dispatch
- `illumos-gate/usr/src/uts/intel/io/vgatext/vgatext.c` -- DDI driver creating `/dev/fb0`

### Smithay Backend Landscape

| Backend | Requirements | Works on illumos? |
|---------|-------------|-------------------|
| `backend_drm` | DRM/KMS + GBM + libseat | Only Intel Gen2-7 |
| `backend_x11` | X11 + DRM node + GBM | Only with DRM (rare) |
| `backend_winit` | winit + EGL | Needs winit illumos patches + Mesa |
| Custom fbio | `/dev/fb0` + UEFI GOP | Yes -- bare metal, no X11 needed |
| Custom X11 SHM | X11 + MIT-SHM extension | Yes -- universal fallback |

## Decision

### Four-tier local display architecture:

### Tier 0: Bare-Metal Framebuffer Backend (illumos `/dev/fb0`, Linux `/dev/fb0`)

Direct framebuffer access -- WayRay as the **sole display server**, no X11 underneath.

On illumos (UEFI GOP systems):
1. Open `/dev/fb0`, verify `VIS_GETIDENTIFIER` returns `"illumos_fb"`
2. Query geometry via `FBIOGATTR` (resolution, depth, pitch, RGB layout from `gfxfb_info`)
3. `mmap()` the framebuffer (write-combining memory)
4. `KDSETMODE` → `KD_GRAPHICS` to take over from console
5. Render with `PixmanRenderer` into CPU buffer
6. Copy damaged regions to framebuffer using non-temporal stores (SSE2/AVX2/AVX-512)
7. Input from `/dev/kbd` + `/dev/mouse` (illumos STREAMS input devices)

On Linux:
1. Open `/dev/fb0`, query via `FBIOGET_VSCREENINFO` / `FBIOGET_FSCREENINFO`
2. `mmap()` the framebuffer
3. Render and blit same as above
4. Input via libinput or evdev

```
Wayland apps → Smithay compositor → PixmanRenderer → CPU buffer
    → non-temporal memcpy to /dev/fb0 → pixels on screen
```

**Constraints:** Resolution fixed at boot (UEFI GOP / VESA BIOS). No VSync (tearing possible). No hardware acceleration. Requires UEFI on illumos.

**Performance note:** `xf86-video-illumosfb` demonstrates that SIMD non-temporal stores are essential for write-combining memory. The backend must use `_mm_stream_si128` (SSE2), `_mm256_stream_si256` (AVX2), or `_mm512_stream_si512` (AVX-512) with runtime detection via `getisax(2)` on illumos or CPUID on Linux. Rust's `std::arch` intrinsics provide these.

### Tier 1: Custom X11 SHM Backend (Portable Fallback)

A custom Smithay backend that:
1. Opens an X11 connection via `x11rb` (pure Rust XCB bindings)
2. Creates a window (fullscreen or windowed for development)
3. Renders with `PixmanRenderer` into CPU buffers
4. Presents via `XShmPutImage` (MIT-SHM extension) -- zero DRM/EGL/GBM dependency
5. Receives keyboard/mouse input from X11 events
6. Maps X11 input events to Smithay's input types

This works on **every illumos system with X11**, regardless of GPU or BIOS type. Even `xf86-video-vesa` works. Also useful for development (run WayRay in a window on your existing desktop).

```
Wayland apps → Smithay compositor → PixmanRenderer → CPU buffer
    → XShmPutImage → X11 window on screen
```

### Tier 2: Loopback Optimization (Local Server+Client)

When wrsrvd and wrclient run on the same machine, skip encoding entirely:

1. Server renders to shared memory ring buffer (`shm_open` + `mmap`)
2. Client reads framebuffers directly from shared memory
3. Only damage regions communicated via small control channel
4. Client presents via Tier 0 (fbdev) or Tier 1 (X11 SHM)

```
Wayland apps → Smithay compositor → PixmanRenderer → shared memory
    → wrclient (local) → fbdev or X11 SHM → screen
```

Performance: sub-millisecond frame latency (vs 5-30ms with encode/decode), near-zero CPU overhead for transport, pixel-perfect quality.

### Tier 3: DRM Backend (Linux, Accelerated illumos)

On Linux or illumos with a supported DRM GPU (Intel Gen2-7):
- Use Smithay's standard `backend_drm` + `GlesRenderer`
- Direct scanout, hardware compositing, VSync
- Feature-gated: `local-drm`

## Backend Selection Logic

```
if cfg!(feature = "local-drm") && drm_device_available() {
    // Tier 3: Direct DRM/KMS (best performance)
    use DrmBackend + GlesRenderer
} else if local_mode && fbdev_available() {
    // Tier 0: Bare-metal framebuffer (no X11 needed)
    use FbdevBackend + PixmanRenderer
} else if local_mode && x11_available() {
    // Tier 1: X11 SHM (fallback, also good for development)
    use X11ShmBackend + PixmanRenderer
} else {
    // Remote mode (default)
    use HeadlessBackend + PixmanRenderer + QUIC transport
}
```

## Mode Summary

| Mode | Backend | Renderer | Transport | Use Case |
|------|---------|----------|-----------|----------|
| **Remote** | Headless | Pixman/GLES | QUIC (encode+decode) | Thin client (primary) |
| **Local fbdev** | illumos fbio / Linux fbdev | Pixman | Non-temporal memcpy to `/dev/fb0` | Bare-metal workstation (UEFI) |
| **Local X11** | X11 SHM | Pixman | XShmPutImage | Development / legacy BIOS / fallback |
| **Local Loopback** | Headless | Pixman | Shared memory | Co-located server+client |
| **Local DRM** | DRM/KMS | GLES | Direct scanout | Linux / accelerated GPU |

## Implementation: Framebuffer Backend (Tier 0)

```rust
struct FbdevBackend {
    fd: RawFd,
    buffer: *mut u8,           // mmap'd framebuffer (write-combining)
    shadow: Vec<u8>,           // CPU-cached shadow buffer for rendering
    width: u32,
    height: u32,
    depth: u32,
    pitch: u32,                // bytes per scanline
    rgb_layout: RgbLayout,     // mask/position from gfxfb_info
    size: usize,
}

impl FbdevBackend {
    fn open_illumos() -> Result<Self> {
        let fd = open("/dev/fb0", O_RDWR)?;
        // Verify identity
        let ident = vis_getidentifier(fd)?;
        assert_eq!(ident.name, "illumos_fb");
        // Query geometry
        let attr = fbiogattr(fd)?;
        let gfxfb = gfxfb_info_from_dev_specific(&attr.sattr.dev_specific);
        // mmap framebuffer
        let buffer = mmap(fd, attr.fbtype.fb_size, PROT_READ | PROT_WRITE, MAP_SHARED)?;
        // Take over from console
        kdsetmode(fd, KD_GRAPHICS)?;
        // ...
    }

    fn present_damage(&self, damage: &[Rectangle]) {
        // For each damage rect: copy from shadow to FB
        // using non-temporal stores for write-combining memory
        for rect in damage {
            streaming_copy_rect(&self.shadow, self.buffer, rect, self.pitch);
        }
    }
}

/// SIMD non-temporal copy (runtime-selected)
fn streaming_copy_rect(src: &[u8], dst: *mut u8, rect: &Rectangle, pitch: u32) {
    // SSE2:   _mm_stream_si128
    // AVX2:   _mm256_stream_si256
    // AVX-512: _mm512_stream_si512
    // Selected at runtime via std::is_x86_feature_detected!()
    // (or getisax(2) on illumos)
}
```

On Linux, a similar struct uses `FBIOGET_VSCREENINFO` / `FBIOGET_FSCREENINFO` instead of `FBIOGATTR`.

Input on bare metal:
- illumos: read from `/dev/kbd` (keyboard) and `/dev/mouse` (mouse) via STREAMS ioctls
- Linux: libinput or raw evdev (`/dev/input/event*`)

## Implementation: X11 SHM Backend (Tier 1)

```rust
struct X11ShmBackend {
    conn: x11rb::rust_connection::RustConnection,
    window: x11rb::protocol::xproto::Window,
    shm_seg: x11rb::protocol::shm::Seg,
    buffer: *mut u8,  // mmap'd SHM region
    width: u32,
    height: u32,
    // Input state
    keyboard_state: xkb::State,
    pointer_position: (f64, f64),
}

impl X11ShmBackend {
    /// Present a rendered frame to the X11 window
    fn present(&self, damage: &[Rectangle]) {
        // XShmPutImage for each damage rectangle
        // (or full frame if damage covers most of screen)
    }

    /// Pump X11 events and convert to Smithay InputEvents
    fn process_input(&mut self) -> Vec<InputEvent> {
        // KeyPress/KeyRelease -> KeyboardKeyEvent
        // MotionNotify -> PointerMotionAbsoluteEvent
        // ButtonPress/ButtonRelease -> PointerButtonEvent
        // etc.
    }
}
```

The backend integrates into calloop by registering the X11 connection fd as an event source.

## Rationale

- **`/dev/fb0` enables bare-metal on illumos**: No X11 dependency, WayRay as sole display server. Proven by xf86-video-illumosfb consuming the same `fbio(4I)` interface.
- **PixmanRenderer is fast enough**: For a desktop compositor on a workstation CPU, software compositing handles typical desktop loads well. Browsers and media do their own GPU rendering internally.
- **Same compositor, different output**: The Smithay compositor core is identical in local and remote modes. Only the output backend changes. This avoids maintaining two compositor codepaths.
- **cocoa-way validates this**: Smithay on macOS works by rendering headless and presenting to a native window. Same pattern, different native window system.
- **Loopback optimization is easy**: Once local X11 works, adding shared-memory passthrough for co-located server+client is incremental

## Consequences

- Must write and maintain two custom backends: fbdev and X11 SHM (neither in upstream Smithay)
- Fbdev backend requires SIMD non-temporal store implementation (Rust `std::arch` intrinsics)
- Fbdev resolution is fixed at boot; no mode switching. Users must configure UEFI GOP resolution in firmware settings.
- Fbdev has no VSync; tearing is possible. Mitigate with damage-based partial updates.
- Fbdev input on illumos needs custom `/dev/kbd` + `/dev/mouse` STREAMS reader
- X11 SHM backend adds `x11rb` as a dependency (already pure Rust, minimal)
- Input mapping from X11 events to Smithay types requires careful keysym/keycode handling
- Fullscreen mode needs proper X11 EWMH hints (`_NET_WM_STATE_FULLSCREEN`)
- Multi-monitor in fbdev mode requires multiple `/dev/fb*` devices or single large GOP framebuffer
- Multi-monitor in X11 SHM mode depends on Xorg's RANDR configuration
- On Linux, users will prefer the DRM backend; fbdev/X11 SHM are primarily for illumos

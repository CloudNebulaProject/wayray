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
| Linux framebuffer (`/dev/fb0`) | Does not exist. No `vesafb`/`simplefb` equivalent. |
| illumos VIS (`/dev/fbs/*`) | Kernel-only ioctls (`FKIOCTL` check). Unusable from userspace. |
| X11 + VESA DDX | Works on any GPU. CPU-rendered but functional. |
| X11 + i915 DDX | Works on Intel Gen2-7 with DRI acceleration. |
| X11 + NVIDIA proprietary | Works with specific driver versions. |
| Mesa llvmpipe | Software OpenGL available everywhere. |

**X11 is the universal display path on illumos.** Every illumos workstation has a working X11 session, even if it's VESA-only.

### Smithay Backend Landscape

| Backend | Requirements | Works on illumos? |
|---------|-------------|-------------------|
| `backend_drm` | DRM/KMS + GBM + libseat | Only Intel Gen2-7 |
| `backend_x11` | X11 + DRM node + GBM | Only with DRM (rare) |
| `backend_winit` | winit + EGL | Needs winit illumos patches + Mesa |
| Custom X11 SHM | X11 + MIT-SHM extension | Yes -- universal |

## Decision

### Three-tier local display architecture:

### Tier 1: Custom X11 SHM Backend (Primary, Portable)

A custom Smithay backend that:
1. Opens an X11 connection via `x11rb` (pure Rust XCB bindings)
2. Creates a window (fullscreen or windowed for development)
3. Renders with `PixmanRenderer` into CPU buffers
4. Presents via `XShmPutImage` (MIT-SHM extension) -- zero DRM/EGL/GBM dependency
5. Receives keyboard/mouse input from X11 events
6. Maps X11 input events to Smithay's input types

This works on **every illumos system with X11**, regardless of GPU. Even `xf86-video-vesa` works because we only need X11 SHM pixmap blitting.

```
Wayland apps → Smithay compositor → PixmanRenderer → CPU buffer
    → XShmPutImage → X11 window on screen
```

### Tier 2: Loopback Optimization (Local Server+Client)

When wayray-server and wayray-client run on the same machine, skip encoding entirely:

1. Server renders to shared memory ring buffer (`shm_open` + `mmap`)
2. Client reads framebuffers directly from shared memory
3. Only damage regions communicated via small control channel
4. Client presents to X11 via SHM pixmaps

```
Wayland apps → Smithay compositor → PixmanRenderer → shared memory
    → wayray-client (local) → XShmPutImage → screen
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
    // Tier 3: Direct DRM/KMS
    use DrmBackend + GlesRenderer
} else if local_mode && x11_available() {
    // Tier 1: X11 SHM
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
| **Local X11** | X11 SHM | Pixman | XShmPutImage (direct) | illumos workstation |
| **Local Loopback** | Headless | Pixman | Shared memory | Same-machine optimization |
| **Local DRM** | DRM/KMS | GLES | Direct scanout | Linux / accelerated GPU |

## Implementation: X11 SHM Backend

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

- **X11 SHM is universal on illumos**: No GPU requirements, works with VESA
- **PixmanRenderer is fast enough**: For a desktop compositor on a workstation CPU, software compositing handles typical desktop loads well. Browsers and media do their own GPU rendering internally.
- **Same compositor, different output**: The Smithay compositor core is identical in local and remote modes. Only the output backend changes. This avoids maintaining two compositor codepaths.
- **cocoa-way validates this**: Smithay on macOS works by rendering headless and presenting to a native window. Same pattern, different native window system.
- **Loopback optimization is easy**: Once local X11 works, adding shared-memory passthrough for co-located server+client is incremental

## Consequences

- Must write and maintain a custom X11 SHM backend (not in upstream Smithay)
- X11 SHM backend adds `x11rb` as a dependency (already pure Rust, minimal)
- Performance ceiling on unaccelerated VESA: adequate for desktop, not for gaming
- Input mapping from X11 events to Smithay types requires careful keysym/keycode handling
- Fullscreen mode needs proper X11 EWMH hints (`_NET_WM_STATE_FULLSCREEN`)
- Multi-monitor in X11 SHM mode depends on Xorg's RANDR configuration
- On Linux, users will prefer the DRM backend; X11 SHM is primarily for illumos

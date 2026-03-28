# ADR-008: illumos as Primary Target Platform

## Status
Accepted

## Context
WayRay is a spiritual successor to SunRay, which ran on Solaris. illumos (the open-source Solaris fork) is a natural target platform alongside Linux. However, the standard Wayland compositor stack (DRM/KMS, libinput, udev, libseat, PipeWire) is deeply Linux-specific.

## Research Findings

### What works on illumos
- **Rust**: Tier 2 with host tools (`x86_64-unknown-illumos`), full std support
- **tokio/mio**: Native event port support (`port_create`/`port_associate`), correct and performant
- **quinn (QUIC)**: Pure Rust on tokio, fully portable
- **rustls**: Pure Rust TLS, no platform dependencies
- **Wayland protocol core**: Unix domain sockets + `SCM_RIGHTS` fd passing + `shm_open` all available
- **PixmanRenderer**: Pure software, fully portable
- **PulseAudio**: Available on illumos (default audio on OpenIndiana)

### What does NOT work on illumos
- **DRM/KMS**: Only ancient Intel (pre-Gen8). AMD/modern Intel unsupported. Not needed for headless.
- **libinput**: Depends on Linux evdev. illumos uses STREAMS-based `/dev/kbd` and `/dev/mouse`.
- **udev**: Linux-specific. illumos uses `sysevent` for device notifications.
- **libseat/logind**: Linux-specific session management. Irrelevant for headless compositor.
- **PipeWire**: Not ported. PulseAudio available as alternative.
- **memfd_create**: Linux-specific since kernel 3.17. Must fall back to `shm_open` + `mmap`.
- **GBM**: Mesa's buffer manager, Linux-specific. Not needed without DRM.
- **USB/IP**: Linux kernel module, no illumos equivalent.

### Precedent
**cocoa-way** (macOS Wayland compositor on Smithay) proves the architecture: use Smithay with `default-features = false`, software rendering, custom input backend, no DRM/libinput/udev.

## Decision
**Target illumos as a first-class platform via headless-first compositor architecture.**

WayRay's server is inherently headless -- it renders to a virtual framebuffer for network transmission, not to a physical display. This means the Linux-specific backends (DRM, libinput, udev, libseat) are optional features for local display, not core requirements.

### Platform Abstraction Strategy

```
smithay (default-features = false)
  + wayland_frontend     (portable: Unix sockets + shm)
  + renderer_pixman      (portable: pure software)
  + renderer_gl          (optional: when EGL available)
  + desktop              (portable: Space, Window, helpers)
  + xwayland             (portable: X11 socket protocol)

Platform-specific features (feature flags):
  + backend_drm          (Linux only)
  + backend_libinput     (Linux only)
  + backend_udev         (Linux only)
  + backend_session_libseat (Linux only)
  + backend_winit        (Linux + illumos dev mode)
```

### Required Portability Work

| Component | Approach |
|-----------|----------|
| Shared memory | `shm_open` fallback when `memfd_create` unavailable |
| Input (server-side) | Network input from clients (primary), `/dev/kbd`+`/dev/mouse` (illumos local) |
| Audio | Trait-based backend: PipeWire (Linux), PulseAudio (illumos/Linux) |
| Device hotplug | `sysevent` on illumos, `udev` on Linux |
| USB forwarding | Userspace protocol over QUIC (not kernel USB/IP) |

## Rationale
- SunRay ran on Solaris; WayRay running on illumos honors that heritage
- Headless-first architecture makes illumos support natural, not an afterthought
- The critical path (Wayland protocol, rendering, networking) is already portable
- Forces clean separation of platform-specific backends behind traits
- illumos Zones provide excellent session isolation (better than Linux namespaces)

## Consequences
- Must compile Smithay with `default-features = false` and select features carefully
- Need `cfg(target_os)` gates for platform-specific code paths
- `memfd_create` fallback adds minor complexity to shared memory handling
- PulseAudio support required alongside PipeWire for audio
- Cannot rely on Linux-specific crates without feature-gating them
- CI must test on both Linux and illumos (illumos CI runner or cross-compilation)

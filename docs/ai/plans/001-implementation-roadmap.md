# Implementation Roadmap

## Phase 0: Foundation (Weeks 1-2)

### 0.1 Project Structure
- Set up Cargo workspace with four crates: `wayray-server`, `wayray-client`, `wayray-protocol`, `wayray-ctl`
- Configure shared dependencies, feature flags, CI (Linux + illumos)
- Set up tracing/logging infrastructure with miette error handling
- Smithay with `default-features = false` + portable features only in core
- Platform-specific backends behind `cfg(target_os)` + feature flags

### 0.2 Minimal Compositor (Server)
- Implement a minimal Smithay compositor based on Smallvil patterns
- Support: wl_compositor, xdg_shell, wl_shm, wl_seat, wl_output
- Use Winit backend for development/testing (works on both Linux and illumos)
- Use `shm_open` with `memfd_create` fallback for shared memory portability
- Verify basic Wayland clients (foot terminal, weston-info) can connect and render

### 0.3 Framebuffer Capture
- Implement ExportMem-based framebuffer capture after each render pass
- Integrate OutputDamageTracker for efficient dirty region tracking
- Benchmark: capture latency, memory bandwidth

## Phase 1: Network Protocol (Weeks 3-5)

### 1.1 Protocol Definition
- Define WayRay wire protocol in wayray-protocol crate
- Message types: FrameUpdate, InputEvent, SessionControl, AudioChunk, USBData
- Serialization: serde + bincode or postcard for low overhead
- Version negotiation and capability exchange

### 1.2 QUIC Transport Layer
- Implement QUIC server (quinn) in wayray-server
- Implement QUIC client (quinn) in wayray-client
- Stream mapping:
  - Stream 0: Control channel (session mgmt, capabilities)
  - Stream 1: Display channel (frame updates, damage regions)
  - Stream 2: Input channel (keyboard, mouse, touch)
  - Stream 3: Audio channel (Opus frames)
  - Stream 4+: USB device channels (one per device)
- Connection handling: TLS certificates, authentication

### 1.3 Frame Encoding Pipeline
- Implement frame differencing (XOR diff against previous frame)
- Implement region-based compression (zstd for lossless regions)
- Implement content-adaptive encoding:
  - Static regions: lossless zstd diff
  - Video regions: H.264 via ffmpeg-next or VAAPI
  - Text regions: lossless PNG-style encoding
- Damage rectangle merging and optimization

## Phase 2: Client Viewer (Weeks 5-7)

### 2.1 Display Client
- Implement wayray-client as a standalone application
- Use winit + wgpu for cross-platform display
- Frame decoding pipeline: receive -> decompress -> decode -> upload to GPU -> display
- Double-buffered rendering with VSync

### 2.2 Input Capture & Forwarding
- Capture keyboard events (with proper keymap forwarding via xkb)
- Capture mouse events (motion, buttons, scroll)
- Capture touch events
- Serialize and send over QUIC input stream
- Handle keyboard grab/release for compositor key passthrough

### 2.3 Cursor Handling
- Server-side cursor rendering (simplest)
- Client-side cursor rendering with cursor image forwarding (lower latency)
- Cursor shape protocol support

## Phase 2.5: Pluggable Window Management (Weeks 7-8)

### 2.5.1 WM Protocol Definition
- Define `wayray_wm_manager_v1` Wayland protocol XML
- Define `wayray_wm_window_v1`, `wayray_wm_seat_v1`, `wayray_wm_workspace_v1`
- Implement two-phase transaction model (manage + render sequences)
- Generate Rust bindings via wayland-scanner

### 2.5.2 WM Protocol Server (in compositor)
- Implement WM global in wayray-server
- Window lifecycle events (new, closed, properties)
- Manage phase: receive policy decisions, send configures
- Render phase: apply positions/z-order atomically
- Keybinding registration and dispatch via seat interface

### 2.5.3 Built-in Floating WM
- Default WM active when no external WM is connected
- Basic floating behavior: centered new windows, focus-follows-click
- Keyboard shortcuts: Alt+F4 close, Alt+Tab cycle, Super+Arrow snap
- Yields to external WM on connect

### 2.5.4 Example Tiling WM
- Ship a reference tiling WM as a separate binary (`wayray-wm-tiling`)
- Demonstrates the protocol for third-party WM developers
- Basic BSP tiling with keyboard-driven focus

## Phase 3: Session Management (Weeks 8-11)

### 3.1 Session Persistence
- Session state machine: Created -> Active -> Suspended -> Resumed -> Destroyed
- Session storage: in-memory with optional persistence (SeaORM + SQLite)
- Session timeout and cleanup policies

### 3.2 Authentication
- Token-based session identification (smart card ID, badge, or software token)
- PAM integration for user authentication
- Session-token binding in session store

### 3.3 Hot-Desking (Session Mobility)
- Token insertion triggers session lookup across server pool
- Session reconnection: rebind existing session to new client endpoint
- Session disconnect: unbind from client, keep session running
- Sub-second reconnection target (< 500ms)

### 3.4 Multi-Server Support
- Server discovery protocol (mDNS or custom)
- Session registry: which sessions live on which servers
- Cross-server session redirect
- Load balancing for new session placement

## Phase 4: Audio & Peripherals (Weeks 10-13)

### 4.1 Audio Forwarding
- Trait-based audio backend: PipeWire (Linux), PulseAudio (illumos/Linux fallback)
- PipeWire integration on server side for audio capture (Linux)
- Opus encoding for low-latency audio streaming
- Audio stream over dedicated QUIC stream
- Playback synchronization with display frames
- Microphone input forwarding (bidirectional audio)

### 4.2 USB Device Forwarding
- Userspace USB forwarding protocol over QUIC (not kernel USB/IP, for illumos portability)
- Consider usbredir as wire format or design custom
- Device hotplug detection on client (udev on Linux, sysevent on illumos)
- Device attach/detach over QUIC channels
- Security: device class filtering (allow/deny policies)

### 4.3 Clipboard Synchronization
- Intercept wl_data_device on server
- Forward clipboard content types and data over control channel
- Handle large clipboard entries (images) efficiently
- Security: optional clipboard direction restrictions

## Phase 5: Production Hardening (Weeks 14-17)

### 5.1 Platform-Specific Backends
- Linux: DRM/KMS backend for running wayray-server on hardware (optional, feature-gated)
- Linux: Multi-GPU support via MultiRenderer
- Linux: Session management via logind/libseat
- illumos: Custom input backend for `/dev/kbd` + `/dev/mouse` (local console use)
- illumos: Zones integration for session isolation
- CI: Test matrix for both Linux and illumos

### 5.2 XWayland Support
- Integrate Smithay's XWayland module for X11 application compatibility
- Handle X11 clipboard integration

### 5.3 Performance Optimization
- Adaptive bitrate based on network conditions
- Hardware encoding path (VAAPI, NVENC)
- Zero-copy frame capture via DMA-BUF export to encoder
- Client-side frame interpolation for network jitter compensation

### 5.4 Security
- TLS 1.3 for all QUIC connections (mandatory)
- Certificate-based mutual authentication
- Session encryption at rest
- Audit logging for session lifecycle events
- AppArmor/seccomp profiles for server process

## Phase 6: Management & Operations (Weeks 16-20)

### 6.1 Administration
- CLI tool: `wayray-ctl` for server/session management
- REST API for external integration
- Session monitoring: active sessions, resource usage, network stats

### 6.2 Multi-Tenancy
- User session isolation (namespaces, cgroups)
- Resource quotas (CPU, memory, GPU per session)
- Fair scheduling across sessions

### 6.3 High Availability
- Server failover groups
- Session state replication for seamless failover
- Health checking and automatic server removal

## Milestones

| Milestone | Phase | Description |
|-----------|-------|-------------|
| M0 | 0 | Wayland clients render in local compositor |
| M1 | 1 | Remote viewer sees compositor output over network |
| M2 | 2 | Interactive remote session (display + input) |
| M2.5 | 2.5 | External WM can control window layout via protocol |
| M3 | 3 | Session persists across client disconnects, hot-desking works |
| M4 | 4 | Audio and USB forwarding functional |
| M5 | 5 | Production-ready with platform backends, XWayland, illumos CI |
| M6 | 6 | Multi-server deployment with HA and management tools |

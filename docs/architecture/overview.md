# WayRay Architecture Overview

## Vision

WayRay is a modern reimagining of Oracle/Sun's SunRay thin client system, built as a native Wayland compositor. Where SunRay used proprietary hardware and the ALP protocol over UDP, WayRay uses commodity hardware, the Wayland protocol for local application rendering, and QUIC for network transport.

## System Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     WayRay Server                        │
│                                                          │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐              │
│  │ App 1    │  │ App 2    │  │ App N    │  Wayland     │
│  │ (foot)   │  │ (firefox)│  │ (...)    │  Clients     │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘              │
│       │              │              │                    │
│       └──────────────┼──────────────┘                    │
│                      │ Wayland Protocol                  │
│                      v                                   │
│  ┌───────────────────────────────────────┐              │
│  │         Smithay Compositor            │              │
│  │                                       │              │
│  │  ┌─────────┐  ┌──────────┐           │              │
│  │  │ XDG     │  │ Layer    │  Protocol │              │
│  │  │ Shell   │  │ Shell    │  Handlers │              │
│  │  └─────────┘  └──────────┘           │              │
│  │                                       │              │
│  │  ┌─────────┐  ┌──────────┐           │              │
│  │  │ Seat    │  │ Output   │           │              │
│  │  │ (input) │  │ Manager  │           │              │
│  │  └─────────┘  └──────────┘           │              │
│  │                                       │              │
│  │  ┌─────────────────────────┐         │              │
│  │  │ Renderer                │         │              │
│  │  │ (Pixman or GLES)       │         │              │
│  │  │                         │         │              │
│  │  │ OutputDamageTracker     │         │              │
│  │  │ ExportMem (framebuffer) │         │              │
│  │  └─────────────────────────┘         │              │
│  └───────────────┬───────────────────────┘              │
│                  │                                       │
│                  v                                       │
│  ┌───────────────────────────────────────┐              │
│  │         Frame Encoder                 │              │
│  │                                       │              │
│  │  Damage Regions -> Tile Classification│              │
│  │  -> Lossless (zstd) / Lossy (H.264)  │              │
│  └───────────────┬───────────────────────┘              │
│                  │                                       │
│  ┌───────────────┴───────────────────────┐              │
│  │         Session Manager               │              │
│  │                                       │              │
│  │  Token -> Session mapping             │              │
│  │  Session lifecycle                    │              │
│  │  Multi-server coordination            │              │
│  └───────────────┬───────────────────────┘              │
│                  │                                       │
│  ┌───────────────┴───────────────────────┐              │
│  │         QUIC Transport (quinn)        │              │
│  │                                       │              │
│  │  Stream 0: Control                    │              │
│  │  Stream 1: Display (frames)           │              │
│  │  Stream 2: Input (keyboard/mouse)     │              │
│  │  Stream 3: Audio (Opus)               │              │
│  │  Stream 4+: USB devices               │              │
│  └───────────────┬───────────────────────┘              │
└──────────────────┼───────────────────────────────────────┘
                   │
                   │  QUIC / TLS 1.3
                   │  (Network)
                   │
┌──────────────────┼───────────────────────────────────────┐
│                  │           WayRay Client                │
│  ┌───────────────┴───────────────────────┐              │
│  │         QUIC Transport (quinn)        │              │
│  └───────────────┬───────────────────────┘              │
│                  │                                       │
│  ┌───────────┬───┴──────┬────────────┬───────────┐     │
│  │           │          │            │           │     │
│  v           v          v            v           v     │
│ ┌─────┐  ┌──────┐  ┌────────┐  ┌────────┐  ┌─────┐  │
│ │Ctrl │  │Frame │  │ Input  │  │ Audio  │  │ USB │  │
│ │     │  │Decode│  │Capture │  │Play/Cap│  │Fwd  │  │
│ └─────┘  └──┬───┘  └───┬────┘  └────────┘  └─────┘  │
│             │          │                              │
│             v          │                              │
│  ┌──────────────┐      │                              │
│  │ wgpu/winit   │      │                              │
│  │ Display      │<─────┘ (cursor, HiDPI)             │
│  └──────────────┘                                     │
│                                                       │
│  ┌──────────────┐                                     │
│  │ Token        │  Smart Card / NFC / Software Token  │
│  │ Provider     │                                     │
│  └──────────────┘                                     │
└───────────────────────────────────────────────────────┘
```

## Core Principles

### 1. Stateless Client
The WayRay client stores no user data, no session state, and no application state. If the client device is lost or destroyed, nothing is lost. All state lives on the server.

### 2. Session Mobility
A session is bound to a token, not a device. Insert your token into any WayRay client on the network and your desktop follows you -- all windows, all applications, exact state preserved.

### 3. Server-Side Rendering
All Wayland clients run on the server. The compositor renders everything server-side and transmits compressed frames to the client. The client is purely a decoder + display + input capture device.

### 4. Adaptive Encoding
Frame encoding adapts to content type and network conditions:
- Text/UI: lossless compression (perfect rendering)
- Video: lossy hardware encoding (H.264/AV1)
- Low bandwidth: aggressive compression, lower framerate
- LAN: minimal compression, high framerate

### 5. Security by Default
- All network traffic encrypted via TLS 1.3 (QUIC mandates it)
- No data at rest on client devices
- Session authentication via PAM
- USB device policies for access control

## Data Flow

### Display (Server -> Client)
1. Wayland applications commit surface updates
2. Smithay compositor renders all surfaces into output framebuffer
3. OutputDamageTracker identifies changed regions since last frame
4. Frame encoder processes damaged regions (tile-based, content-adaptive)
5. Encoded frame sent over QUIC display stream
6. Client decodes frame regions, uploads to GPU, displays

### Input (Client -> Server)
1. Client captures keyboard/mouse/touch events
2. Events serialized with timestamps and keymaps
3. Sent over QUIC input stream (highest priority)
4. Server deserializes and injects into Smithay Seat
5. Seat forwards to focused Wayland client

### Audio (Bidirectional)
1. Server applications output audio to PipeWire virtual sink
2. Audio frames encoded to Opus, sent over QUIC audio stream
3. Client decodes Opus, plays through local audio device
4. Client captures microphone, encodes to Opus, sends to server
5. Server feeds decoded audio into PipeWire virtual source

## Scope Boundary: What WayRay Owns

WayRay is a compositor, not a desktop environment or login system.

**WayRay owns:**
- Wayland compositor (rendering, protocol, encoding, transport)
- Session lifecycle (active, suspended, resumed, destroyed)
- Token-to-session binding
- Pluggable WM protocol

**WayRay does NOT own:**
- User authentication (PAM, LDAP, Kerberos)
- User environment (home dirs, mounts, shell, env vars)
- Desktop environment (WayRay cannot host GNOME/KDE -- they are compositors)

**The desktop experience** is composed from independent Wayland clients:
- Window manager via `wayray_wm_manager_v1` protocol (floating, tiling, custom)
- Panel/taskbar (waybar, etc.) via `wlr-layer-shell`
- App launcher (fuzzel, wofi) via `wlr-layer-shell`
- Notification daemon (mako, fnott) via `wlr-layer-shell`
- Any Wayland or X11 application

**Login flow** is handled by an external session launcher (like greetd for Sway):
1. Token arrives, no session exists
2. Session launcher creates user environment (PAM, mounts)
3. Greeter (a Wayland client) authenticates the user
4. Session launcher starts user's configured session (WM, panel, apps)

## Comparison with SunRay

| Feature | SunRay | WayRay |
|---------|--------|--------|
| Display Protocol | ALP (proprietary, UDP) | WayRay Protocol (QUIC) |
| Compositor | X server (Xnewt) | Wayland (Smithay) |
| Client Hardware | Purpose-built DTU | Any Linux/macOS/Windows device |
| Session Mobility | Smart card | Pluggable tokens (smart card, NFC, software) |
| Login Screen | dtlogin (X11 client) | wrlogin (Wayland client) |
| Desktop | CDE/GNOME on X11 | Composable: pluggable WM + panel + launcher |
| Audio | Custom ALP channel | Opus over QUIC |
| USB Forwarding | Custom | Userspace over QUIC |
| Encryption | Optional ALP encryption | Mandatory TLS 1.3 |
| Multi-server | Failover groups | Distributed session registry |
| Platform | Solaris | illumos + Linux |
| Client OS | Proprietary RTOS | Native application |

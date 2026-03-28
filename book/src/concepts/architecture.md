# Architecture

WayRay follows a classic thin client architecture with a modern twist: the server is a full Wayland compositor, and the network protocol is QUIC.

## The Big Picture

```
┌──────────────────────┐         QUIC/TLS 1.3        ┌──────────────────────┐
│    WayRay Server     │ ◄──────────────────────────► │    WayRay Client     │
│                      │                              │                      │
│  Wayland Compositor  │  Display frames ──────────►  │  Frame Decoder       │
│  (Smithay)           │                              │  + Display (wgpu)    │
│                      │  ◄────────── Input events    │                      │
│  Applications run    │                              │  Keyboard/Mouse      │
│  here (foot, firefox │  Audio (Opus) ◄──────────►   │  capture             │
│  vscode, etc.)       │                              │                      │
│                      │  USB data ◄──────────────►   │  USB forwarding      │
└──────────────────────┘                              └──────────────────────┘
```

## Server Architecture

The server has four major subsystems:

### 1. Compositor (Smithay)
The heart of WayRay. This is a standard Wayland compositor that:
- Accepts client connections via the Wayland protocol
- Manages window placement, focus, and decoration
- Handles input distribution to focused windows
- Renders all surfaces into a combined framebuffer

Unlike a desktop compositor (which renders to a monitor), WayRay's compositor renders to a virtual framebuffer for network transmission.

### 2. Frame Encoder
Takes the rendered framebuffer and produces compressed data for transmission:
- **Damage tracking**: Only processes regions that changed since the last frame
- **Content classification**: Identifies text, UI, and video regions
- **Adaptive encoding**: Lossless for text/UI, lossy for video, based on content and bandwidth
- **Tile-based**: Divides the frame into tiles for parallel processing

### 3. Session Manager
Manages the lifecycle of user sessions:
- Token-based session identity
- Session creation, suspension, and resumption
- Multi-server session routing for hot-desking
- Authentication via PAM

### 4. Network Layer (QUIC)
Multiplexed transport with independent streams for each data type:
- Display, input, audio, and USB on separate streams
- Loss on one stream doesn't block others
- Built-in encryption (TLS 1.3)
- Connection migration and 0-RTT resumption

## Client Architecture

The client is intentionally simple -- a "dumb terminal" that:

1. **Connects** to a server with a session token
2. **Receives** encoded frame updates
3. **Decodes** and **displays** them via wgpu
4. **Captures** input events and sends them to the server
5. **Plays** audio received from the server
6. **Forwards** USB devices attached to the client

No application logic runs on the client. If the client crashes or is replaced, nothing is lost.

## Why This Architecture?

### Centralized Management
All applications, data, and configuration live on servers. IT manages servers, not thousands of desktops.

### Security
Client devices store nothing. A stolen thin client reveals no data. All traffic is encrypted.

### Session Mobility
Sessions are server-side state. Any client can display any session. The token is the key.

### Resource Efficiency
Powerful server hardware is shared among users. Client devices can be minimal -- even a Raspberry Pi.

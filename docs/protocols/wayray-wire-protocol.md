# WayRay Wire Protocol Specification

## Overview

The WayRay wire protocol defines communication between WayRay server and client over QUIC. All messages are serialized with postcard (compact binary, no-std compatible) and transmitted over typed QUIC streams.

## Protocol Version

Protocol version is exchanged during the initial handshake on the control stream. Both sides must agree on a compatible version before proceeding.

```
ProtocolVersion {
    major: u16,  // Breaking changes
    minor: u16,  // Additive features
    patch: u16,  // Bug fixes
}
```

## QUIC Stream Layout

### Stream 0: Control (Bidirectional, Reliable)
Session management, capability negotiation, configuration.

### Stream 1: Display (Server -> Client, Semi-Reliable)
Frame updates. Old undelivered frames may be skipped in favor of newer ones.

### Stream 2: Input (Client -> Server, Reliable)
Keyboard, mouse, touch, tablet input events.

### Stream 3: Audio (Bidirectional, Semi-Reliable)
Opus-encoded audio frames. Tolerant of packet loss.

### Stream 4+: USB (Bidirectional, Reliable)
One stream per USB device. USB/IP or usbredir encapsulated data.

## Message Format

Every message on the wire is framed as:

```
┌──────────┬──────────┬──────────────────┐
│ type: u8 │ len: u32 │ payload: [u8]    │
└──────────┴──────────┴──────────────────┘
```

- `type`: Message type discriminant
- `len`: Payload length in bytes (little-endian)
- `payload`: Postcard-serialized message body

## Control Messages (Stream 0)

### ClientHello
Client -> Server. Sent immediately after QUIC connection established.

```rust
ClientHello {
    protocol_version: ProtocolVersion,
    token: SessionToken,            // Session identity
    client_capabilities: ClientCaps,
    display_info: Vec<DisplayInfo>, // Client's physical displays
}

ClientCaps {
    max_width: u32,
    max_height: u32,
    supports_h264: bool,
    supports_av1: bool,
    supports_opus: bool,
    supports_usb: bool,
    preferred_refresh_rate: u32,
}

DisplayInfo {
    width: u32,
    height: u32,
    scale_factor: f64,
    refresh_rate: u32,
}
```

### ServerHello
Server -> Client. Response to ClientHello.

```rust
ServerHello {
    protocol_version: ProtocolVersion,
    session_id: Uuid,
    session_state: SessionState,  // New, Resumed
    server_capabilities: ServerCaps,
    output_config: OutputConfig,  // Assigned resolution/refresh
    keymap: Vec<u8>,              // XKB keymap for input
}
```

### SessionSuspend / SessionResume
Bidirectional. Lifecycle transitions.

### Ping / Pong
Bidirectional. Latency measurement and keepalive.

## Display Messages (Stream 1)

### FrameUpdate
Server -> Client. Contains one or more encoded regions.

```rust
FrameUpdate {
    sequence: u64,
    timestamp_us: u64,
    cursor: Option<CursorUpdate>,
    regions: Vec<EncodedRegion>,
}

EncodedRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    encoding: RegionEncoding,
    data: Vec<u8>,
}

enum RegionEncoding {
    Raw,           // Uncompressed RGBA
    Zstd,          // Zstd-compressed diff against previous frame
    Jpeg,          // JPEG for photographic content
    H264,          // H.264 NAL units
    Av1,           // AV1 OBU
}

CursorUpdate {
    x: i32,
    y: i32,
    shape: Option<CursorShape>,  // Only sent when shape changes
}

CursorShape {
    width: u32,
    height: u32,
    hotspot_x: u32,
    hotspot_y: u32,
    pixels: Vec<u8>,  // RGBA
}
```

### FrameAck
Client -> Server. Acknowledges frame receipt for flow control.

```rust
FrameAck {
    sequence: u64,
    render_time_us: u64,  // How long client took to decode + render
}
```

## Input Messages (Stream 2)

### KeyboardEvent

```rust
KeyboardEvent {
    timestamp_us: u64,
    keycode: u32,
    state: KeyState,       // Pressed, Released
    modifiers: Modifiers,  // Current modifier state
}
```

### PointerMotion

```rust
PointerMotion {
    timestamp_us: u64,
    x: f64,  // Absolute position in output coordinates
    y: f64,
}
```

### PointerButton

```rust
PointerButton {
    timestamp_us: u64,
    button: u32,
    state: ButtonState,
}
```

### PointerAxis

```rust
PointerAxis {
    timestamp_us: u64,
    axis: Axis,         // Horizontal, Vertical
    value: f64,
    discrete: Option<i32>,
}
```

### TouchEvent

```rust
TouchEvent {
    timestamp_us: u64,
    touch_type: TouchType,  // Down, Up, Motion, Frame, Cancel
    id: i32,
    x: f64,
    y: f64,
}
```

## Audio Messages (Stream 3)

### AudioFrame

```rust
AudioFrame {
    timestamp_us: u64,
    channels: u8,
    sample_rate: u32,
    codec: AudioCodec,     // Opus
    data: Vec<u8>,         // Encoded audio data
}
```

### AudioConfig
Sent during session setup to negotiate audio parameters.

```rust
AudioConfig {
    sample_rate: u32,      // 48000
    channels: u8,          // 2 (stereo)
    frame_duration_ms: u8, // 20
    codec: AudioCodec,
}
```

## USB Messages (Stream 4+)

### USBDeviceAttach

```rust
USBDeviceAttach {
    device_id: u32,
    vendor_id: u16,
    product_id: u16,
    device_class: u8,
    device_subclass: u8,
    device_protocol: u8,
    description: String,
}
```

### USBDeviceDetach

```rust
USBDeviceDetach {
    device_id: u32,
}
```

### USBData
Encapsulated USB/IP or usbredir protocol data.

```rust
USBData {
    device_id: u32,
    data: Vec<u8>,
}
```

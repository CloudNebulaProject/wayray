# Phase 1: Remote Display Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a working remote display pipeline — headless wrsrvd on Linux sends compositor frames over QUIC to wrclient on macOS, which displays them in a native window with input forwarding.

**Architecture:** Seven tasks building bottom-up: wire protocol → frame encoding → headless backend → QUIC transport → client display → input forwarding → integration. Each task produces testable output independently.

**Tech Stack:** Smithay (PixmanRenderer), quinn (QUIC), postcard (serialization), zstd (compression), winit + wgpu (client display), rcgen (TLS certs)

**Spec:** `docs/ai/specs/2026-04-07-phase1-remote-display-design.md`

**Important conventions:**
- Before every commit: `cargo fmt --all && cargo clippy --workspace` — fix all warnings, no dead code
- Use context7 MCP tool for library API lookups when unsure about Smithay, quinn, wgpu, or winit APIs
- Consult Smithay's Smallvil/Anvil examples for compositor patterns

---

## File Structure

```
crates/
├── wayray-protocol/src/
│   ├── lib.rs              # Re-exports, protocol version constant
│   ├── messages.rs         # All message types with serde derives
│   └── codec.rs            # Length-prefixed framing (encode/decode)
│
├── wrsrvd/src/
│   ├── main.rs             # CLI parsing, backend selection, startup
│   ├── state.rs            # WayRay compositor state (minor changes)
│   ├── errors.rs           # Error types (add network errors)
│   ├── handlers/           # Wayland protocol handlers (unchanged)
│   ├── backend/
│   │   ├── mod.rs          # Backend enum + shared interface
│   │   ├── headless.rs     # PixmanRenderer + calloop timer render loop
│   │   └── winit.rs        # Existing Winit backend (moved from main.rs)
│   ├── (render.rs deleted — logic moved into backend/headless.rs and backend/winit.rs)
│   ├── encoder.rs          # XOR diff + zstd compression
│   └── network.rs          # QUIC server, frame sending, input receiving
│
├── wrclient/src/
│   ├── main.rs             # CLI args, QUIC connect, main loop
│   ├── decoder.rs          # zstd decompress + XOR apply
│   ├── display.rs          # winit window + wgpu texture rendering
│   ├── input.rs            # Keyboard/mouse capture → protocol messages
│   └── network.rs          # QUIC client, frame receiving, input sending
│
└── wradm/src/
    └── main.rs             # Unchanged
```

---

## Task 1: Wire Protocol Messages

**Files:**
- Modify: `crates/wayray-protocol/Cargo.toml`
- Rewrite: `crates/wayray-protocol/src/lib.rs`
- Create: `crates/wayray-protocol/src/messages.rs`
- Create: `crates/wayray-protocol/src/codec.rs`

This task fills in the `wayray-protocol` crate with message types and a length-prefixed codec. Everything is pure Rust with serde — no network, no Smithay.

- [ ] **Step 1: Add dependencies to wayray-protocol**

Add to `crates/wayray-protocol/Cargo.toml`:
```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
postcard = { version = "1", features = ["alloc"] }
thiserror.workspace = true

[dev-dependencies]
proptest = "1"
```

Add `postcard` and `serde` to workspace dependencies in root `Cargo.toml`.

- [ ] **Step 2: Create message types**

`crates/wayray-protocol/src/messages.rs`:

Define these types with `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]`:

**Control messages:**
```rust
pub struct ClientHello {
    pub version: u32,
    pub capabilities: Vec<String>,
}

pub struct ServerHello {
    pub version: u32,
    pub session_id: u64,
    pub output_width: u32,
    pub output_height: u32,
}

pub struct Ping { pub timestamp: u64 }
pub struct Pong { pub timestamp: u64 }
pub struct FrameAck { pub sequence: u64 }
```

**Display messages:**
```rust
pub struct DamageRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,  // zstd-compressed XOR diff
}

pub struct FrameUpdate {
    pub sequence: u64,
    pub regions: Vec<DamageRegion>,
}
```

**Input messages:**
```rust
pub enum KeyState { Pressed, Released }
pub enum ButtonState { Pressed, Released }
pub enum Axis { Horizontal, Vertical }

pub struct KeyboardEvent { pub keycode: u32, pub state: KeyState, pub time: u32 }
pub struct PointerMotion { pub x: f64, pub y: f64, pub time: u32 }
pub struct PointerButton { pub button: u32, pub state: ButtonState, pub time: u32 }
pub struct PointerAxis { pub axis: Axis, pub value: f64, pub time: u32 }
```

**Top-level enums for each channel:**
```rust
pub enum ControlMessage {
    ClientHello(ClientHello),
    ServerHello(ServerHello),
    Ping(Ping),
    Pong(Pong),
    FrameAck(FrameAck),
}

pub enum DisplayMessage {
    FrameUpdate(FrameUpdate),
}

pub enum InputMessage {
    Keyboard(KeyboardEvent),
    PointerMotion(PointerMotion),
    PointerButton(PointerButton),
    PointerAxis(PointerAxis),
}
```

- [ ] **Step 3: Create the codec**

`crates/wayray-protocol/src/codec.rs`:

Length-prefixed framing: 4-byte LE length + postcard payload.

```rust
use serde::{Serialize, de::DeserializeOwned};

/// Encode a message to a length-prefixed byte vector.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    let payload = postcard::to_allocvec(msg)?;
    let len = (payload.len() as u32).to_le_bytes();
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Decode a message from a byte slice (payload only, after length prefix).
pub fn decode<T: DeserializeOwned>(data: &[u8]) -> Result<T, CodecError> {
    Ok(postcard::from_bytes(data)?)
}

/// Read a length prefix from a byte slice, returning (length, remaining).
pub fn read_length_prefix(data: &[u8]) -> Option<(u32, &[u8])> {
    if data.len() < 4 { return None; }
    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    Some((len, &data[4..]))
}
```

Define `CodecError` wrapping postcard errors.

- [ ] **Step 4: Update lib.rs**

```rust
pub mod messages;
pub mod codec;
pub const PROTOCOL_VERSION: u32 = 1;
```

- [ ] **Step 5: Write tests**

Test round-trip serialization for each message type. Test the codec encode/decode cycle. Test edge cases (empty regions, max values).

```rust
#[test]
fn test_control_message_roundtrip() {
    let msg = ControlMessage::ClientHello(ClientHello { version: 1 });
    let encoded = codec::encode(&msg).unwrap();
    let (len, payload) = codec::read_length_prefix(&encoded).unwrap();
    let decoded: ControlMessage = codec::decode(&payload[..len as usize]).unwrap();
    assert_eq!(decoded, msg);
}
```

- [ ] **Step 6: Verify**

```bash
cargo test -p wayray-protocol
cargo fmt --all && cargo clippy --workspace
```

- [ ] **Step 7: Commit**

```bash
git add crates/ tests/ Cargo.toml Cargo.lock && git commit -m "Implement wire protocol messages and codec in wayray-protocol

Message types for control, display, and input channels.
Length-prefixed postcard serialization with encode/decode codec."
```

---

## Task 2: Frame Encoder and Decoder

**Files:**
- Create: `crates/wrsrvd/src/encoder.rs`
- Create: `crates/wrclient/src/decoder.rs`
- Modify: `crates/wrsrvd/Cargo.toml` (add zstd)
- Modify: `crates/wrclient/Cargo.toml` (add zstd, wayray-protocol)

Pure functions — no network, no compositor. Takes pixel buffers in, produces encoded/decoded data out. Fully testable in isolation.

- [ ] **Step 1: Add zstd dependency**

Add `zstd = "0.13"` to workspace dependencies and to both wrsrvd and wrclient.

- [ ] **Step 2: Implement the encoder**

`crates/wrsrvd/src/encoder.rs`:

```rust
use wayray_protocol::messages::DamageRegion;

/// Compute XOR diff between two ARGB8888 framebuffers.
/// Returns a new buffer where unchanged pixels are zero.
pub fn xor_diff(current: &[u8], previous: &[u8]) -> Vec<u8> {
    assert_eq!(current.len(), previous.len());
    current.iter().zip(previous.iter()).map(|(a, b)| a ^ b).collect()
}

/// Extract a rectangular region from a framebuffer and compress it.
/// `stride` is the number of bytes per row in the full framebuffer.
pub fn encode_region(
    diff: &[u8],
    stride: usize,
    x: u32, y: u32, width: u32, height: u32,
) -> DamageRegion {
    let bpp = 4; // ARGB8888
    let region_stride = width as usize * bpp;
    let mut region_data = Vec::with_capacity(region_stride * height as usize);

    for row in 0..height as usize {
        let src_offset = (y as usize + row) * stride + x as usize * bpp;
        region_data.extend_from_slice(&diff[src_offset..src_offset + region_stride]);
    }

    let compressed = zstd::encode_all(region_data.as_slice(), 1).expect("zstd encode failed");

    DamageRegion { x, y, width, height, data: compressed }
}
```

- [ ] **Step 3: Implement the decoder**

`crates/wrclient/src/decoder.rs`:

```rust
use wayray_protocol::messages::DamageRegion;

/// Apply a compressed XOR diff region onto a framebuffer.
/// Modifies `framebuffer` in place.
pub fn apply_region(
    framebuffer: &mut [u8],
    stride: usize,
    region: &DamageRegion,
) {
    let bpp = 4;
    let decompressed = zstd::decode_all(region.data.as_slice()).expect("zstd decode failed");
    let region_stride = region.width as usize * bpp;

    for row in 0..region.height as usize {
        let dst_offset = (region.y as usize + row) * stride + region.x as usize * bpp;
        let src_offset = row * region_stride;
        let dst_row = &mut framebuffer[dst_offset..dst_offset + region_stride];
        let src_row = &decompressed[src_offset..src_offset + region_stride];
        // XOR apply
        for (d, s) in dst_row.iter_mut().zip(src_row.iter()) {
            *d ^= s;
        }
    }
}
```

- [ ] **Step 4: Write tests**

Test that encoding then decoding a known buffer produces the original.
Test with a fully unchanged frame (all zeros after XOR, compresses to almost nothing).
Test with a single changed pixel.
Test with a subregion of the framebuffer.

```rust
#[test]
fn test_encode_decode_roundtrip() {
    let width = 100u32;
    let height = 100u32;
    let stride = width as usize * 4;
    let prev = vec![0u8; stride * height as usize];
    let mut curr = prev.clone();
    // Change a 10x10 region at (5,5)
    for y in 5..15 {
        for x in 5..15 {
            let offset = y * stride + x * 4;
            curr[offset..offset+4].copy_from_slice(&[255, 0, 0, 255]);
        }
    }
    let diff = encoder::xor_diff(&curr, &prev);
    let region = encoder::encode_region(&diff, stride, 5, 5, 10, 10);
    let mut reconstructed = prev.clone();
    decoder::apply_region(&mut reconstructed, stride, &region);
    assert_eq!(&reconstructed[..], &curr[..]);
}
```

Note: This test requires both encoder and decoder. Put it in `tests/encoding_roundtrip.rs` at the workspace root.

- [ ] **Step 5: Verify**

```bash
cargo test --workspace
cargo fmt --all && cargo clippy --workspace
```

- [ ] **Step 6: Commit**

```bash
git add crates/ tests/ Cargo.toml Cargo.lock && git commit -m "Implement XOR diff + zstd frame encoder and decoder

Encoder in wrsrvd: xor_diff + encode_region (per damage rect).
Decoder in wrclient: apply_region (decompress + XOR apply).
Round-trip integration test verifies correctness."
```

---

## Task 3: Headless Backend with PixmanRenderer

**Files:**
- Create: `crates/wrsrvd/src/backend/mod.rs`
- Create: `crates/wrsrvd/src/backend/headless.rs`
- Create: `crates/wrsrvd/src/backend/winit.rs`
- Modify: `crates/wrsrvd/src/main.rs` (backend selection, major refactor)
- Delete: `crates/wrsrvd/src/render.rs` (logic absorbed into each backend module)
- Modify: `crates/wrsrvd/src/state.rs` (minor: remove GlesRenderer-specific code)
- Modify: `crates/wrsrvd/Cargo.toml` (add `renderer_pixman` feature)

This is the most complex task — it restructures the server to support multiple backends and adds the headless PixmanRenderer path.

- [ ] **Step 1: Add renderer_pixman to Smithay features**

In `crates/wrsrvd/Cargo.toml`, add `renderer_pixman` to the Smithay features list:
```toml
smithay = { version = "0.7", default-features = false, features = [
    "wayland_frontend",
    "desktop",
    "renderer_gl",
    "renderer_pixman",
    "backend_winit",
] }
```

Verify it compiles: `cargo build -p wrsrvd`

- [ ] **Step 2: Create backend module**

`crates/wrsrvd/src/backend/mod.rs`:

Define the interface that both backends must provide. This is not a trait (Smithay's renderer types differ) but a module structure where `main.rs` dispatches based on CLI args.

```rust
pub mod headless;
pub mod winit;
```

- [ ] **Step 3: Implement headless backend**

`crates/wrsrvd/src/backend/headless.rs`:

This module:
- Creates a `PixmanRenderer`
- Creates an in-memory render target using `renderer.create_buffer()`
- Sets up a calloop timer to drive the render loop
- After each render, provides raw pixel data for the encoder

Key Smithay APIs to use:
- `PixmanRenderer::new()` — create the renderer
- `renderer.create_buffer(format, size)` — create a framebuffer image
- `renderer.bind(&mut buffer)` — bind for rendering
- `smithay::desktop::space::render_output()` — render the space
- `OutputDamageTracker::from_output()` — track damage
- Access pixels via the pixman Image's `data()` pointer after rendering

The render loop should:
1. Call `render_output` with the PixmanRenderer
2. Read the pixel data from the buffer (it's already in CPU RAM)
3. Pass pixels + damage regions to a callback/channel for the encoder

Consult Smithay's Anvil example for PixmanRenderer usage patterns. The key difference from the Winit backend: no `backend.bind()` / `backend.submit()` — instead you bind the pixman buffer directly and read pixels after render.

Note: The exact PixmanRenderer API may require experimentation. Smithay 0.7's PixmanRenderer can render to an in-memory `Image`. Use context7 and `cargo doc -p smithay --open` to explore the API.

- [ ] **Step 4: Move existing Winit code to backend/winit.rs**

Move the Winit-specific code from `main.rs` and `render.rs` into `backend/winit.rs`. The Winit backend should be a self-contained module that:
- Initializes the Winit backend
- Runs the event loop
- Handles rendering and frame capture

Keep the existing behavior identical — this is a refactor, not a behavior change.

- [ ] **Step 5: Update main.rs for backend selection**

Add a CLI argument (use `std::env::args()` — no need for clap yet):
```
wrsrvd                  → headless backend (default)
wrsrvd --backend winit  → Winit backend (dev/debug)
```

`main.rs` parses the arg and dispatches to the appropriate backend module.

- [ ] **Step 6: Verify headless backend starts**

```bash
cargo run --bin wrsrvd
```

Expected: Compositor starts without a display server, creates a Wayland socket, accepts client connections. No window opens. Log shows PixmanRenderer initialization and render loop ticking.

Test with:
```bash
WAYLAND_DISPLAY=<socket> weston-info
```

Expected: `weston-info` connects and lists compositor globals.

- [ ] **Step 7: Verify Winit backend still works**

```bash
cargo run --bin wrsrvd -- --backend winit
```

Expected: Same behavior as before (window opens, clients render). This requires a display server.

- [ ] **Step 8: Verify**

```bash
cargo fmt --all && cargo clippy --workspace
```

- [ ] **Step 9: Commit**

```bash
git add crates/ tests/ Cargo.toml Cargo.lock && git commit -m "Add headless backend with PixmanRenderer

Default backend renders to in-memory buffer without display server.
Winit backend moved to backend/winit.rs, selectable via --backend flag.
Headless backend uses PixmanRenderer for pure CPU compositing."
```

---

## Task 4: QUIC Transport

**Files:**
- Create: `crates/wrsrvd/src/network.rs`
- Create: `crates/wrclient/src/network.rs`
- Modify: `crates/wrsrvd/Cargo.toml` (add quinn, rustls, rcgen, tokio)
- Modify: `crates/wrclient/Cargo.toml` (add quinn, rustls, tokio)
- Modify: root `Cargo.toml` (workspace deps)

QUIC server in wrsrvd, client in wrclient. Self-signed TLS certs. Three logical channels (control, display, input).

**Important:** quinn is async (tokio-based). The compositor uses calloop (sync). The network layer runs in a separate tokio runtime thread, communicating with the compositor via channels (`std::sync::mpsc` or `crossbeam`).

- [ ] **Step 1: Add dependencies**

Add to workspace `Cargo.toml`:
```toml
quinn = "0.11"
rustls = { version = "0.23", default-features = false, features = ["ring", "std"] }
rcgen = "0.13"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "sync"] }
```

Add these to both wrsrvd and wrclient `Cargo.toml`.

- [ ] **Step 2: Implement TLS cert generation**

Add a helper function (can go in `wayray-protocol` or a shared utility) that generates a self-signed certificate and key, saves them to `~/.config/wayray/cert.pem` and `~/.config/wayray/key.pem`, and loads them on subsequent runs.

Use `rcgen` to generate the cert. Include `localhost`, `127.0.0.1`, and the machine's hostname as SANs.

- [ ] **Step 3: Implement QUIC server (wrsrvd)**

`crates/wrsrvd/src/network.rs`:

The server:
1. Loads/generates TLS cert
2. Creates a quinn `Endpoint` bound to `0.0.0.0:4433`
3. Accepts one connection
4. Opens channels:
   - Accepts a bidirectional stream from client (control)
   - Opens a unidirectional stream to client (display)
   - Accepts a unidirectional stream from client (input)
5. Reads `ClientHello`, sends `ServerHello` with output dimensions
6. Runs a send loop: reads encoded frames from a channel, sends as `FrameUpdate` messages on the display stream
7. Runs a receive loop: reads `InputMessage` from the input stream, forwards to compositor via channel

**Threading model:**
```
Main thread (calloop):       Network thread (tokio):
  compositor render loop       quinn server
  → encoder produces frame   ← frame_tx channel →   send on display stream
  ← input events             ← input_rx channel ←   recv from input stream
```

Use `std::sync::mpsc::channel()` for frame data (compositor→network) and input events (network→compositor). The calloop side uses `calloop::channel::Channel` to wake the event loop when input arrives.

- [ ] **Step 4: Implement QUIC client (wrclient)**

`crates/wrclient/src/network.rs`:

The client:
1. Connects to server address with `danger_accept_any_cert` for now
2. Opens channels:
   - Opens a bidirectional stream (control)
   - Accepts a unidirectional stream from server (display)
   - Opens a unidirectional stream to server (input)
3. Sends `ClientHello`, receives `ServerHello`
4. Runs a receive loop: reads `FrameUpdate` from display stream, forwards to decoder
5. Runs a send loop: reads input events from channel, sends on input stream

- [ ] **Step 5: Write a basic connection test**

Integration test that starts a QUIC server, connects a client, exchanges hello messages, and disconnects cleanly.

```rust
#[tokio::test]
async fn test_quic_hello_exchange() {
    // Start server in background
    // Connect client
    // Send ClientHello, receive ServerHello
    // Verify version and output dimensions
    // Clean disconnect
}
```

- [ ] **Step 6: Verify**

```bash
cargo test --workspace
cargo fmt --all && cargo clippy --workspace
```

- [ ] **Step 7: Commit**

```bash
git add crates/ tests/ Cargo.toml Cargo.lock && git commit -m "Add QUIC transport layer with quinn

Server: accepts connection, opens control/display/input channels.
Client: connects, exchanges hello, ready for frame/input streams.
Self-signed TLS certs generated on first run."
```

---

## Task 5: Client Display (winit + wgpu)

**Files:**
- Rewrite: `crates/wrclient/src/main.rs`
- Create: `crates/wrclient/src/display.rs`
- Modify: `crates/wrclient/Cargo.toml` (add winit, wgpu, env_logger)

The client viewer: creates a native window and renders received frames as a GPU texture.

- [ ] **Step 1: Add display dependencies**

Add to wrclient `Cargo.toml`:
```toml
winit = "0.30"
wgpu = "24"
env_logger = "0.11"
```

(Check crates.io for current versions — these may need adjustment.)

- [ ] **Step 2: Implement the display module**

`crates/wrclient/src/display.rs`:

This module:
1. Creates a winit `Window` sized to match `ServerHello`'s output dimensions
2. Creates a wgpu `Device`, `Queue`, and `Surface` for the window
3. Maintains an ARGB8888 pixel buffer in CPU RAM (the "local framebuffer")
4. On each frame update: applies decoded regions to the local framebuffer, uploads it to a wgpu `Texture`, renders a fullscreen quad

**Key wgpu pattern:**
- Create a texture matching the output size
- On update: `queue.write_texture()` to upload pixel data
- Render pass: bind the texture, draw a fullscreen triangle/quad
- Use a simple vertex+fragment shader that samples the texture

Consult wgpu examples (particularly the "texture" example) for the render pipeline setup. The shader is trivial — just sample a texture at UV coordinates.

- [ ] **Step 3: Implement main.rs**

`crates/wrclient/src/main.rs`:

```
Usage: wrclient <host>:<port>
```

Main flow:
1. Parse server address from args
2. Start tokio runtime in a background thread
3. Connect QUIC client, exchange hello
4. Create display window at `ServerHello` dimensions
5. Run winit event loop:
   - On `RedrawRequested`: check for new decoded frames, upload and render
   - On keyboard/mouse events: serialize and send via input channel
   - On `CloseRequested`: disconnect and exit

- [ ] **Step 4: Test locally**

Build the client on your Mac:
```bash
cargo build --bin wrclient
```

Run it pointing at a dummy server (or just verify the window opens):
```bash
cargo run --bin wrclient -- 127.0.0.1:4433
```

Expected: Window opens (may show black until server connects). Verify it doesn't crash.

- [ ] **Step 5: Verify**

```bash
cargo fmt --all && cargo clippy --workspace
```

- [ ] **Step 6: Commit**

```bash
git add crates/ tests/ Cargo.toml Cargo.lock && git commit -m "Implement client display with winit + wgpu

Native window renders received frames as GPU texture.
Sized from ServerHello output dimensions."
```

---

## Task 6: Input Forwarding

**Files:**
- Create: `crates/wrclient/src/input.rs`
- Modify: `crates/wrclient/src/main.rs` (capture events in winit loop)
- Modify: `crates/wrsrvd/src/state.rs` (add network input injection)

Client captures winit input events, serializes as protocol messages, sends over QUIC. Server receives and injects into the Smithay seat.

- [ ] **Step 1: Implement client input capture**

`crates/wrclient/src/input.rs`:

Convert winit `WindowEvent` variants to protocol `InputMessage`:
- `WindowEvent::KeyboardInput { event, .. }` → `InputMessage::Keyboard`
- `WindowEvent::CursorMoved { position, .. }` → `InputMessage::PointerMotion`
- `WindowEvent::MouseInput { button, state, .. }` → `InputMessage::PointerButton`
- `WindowEvent::MouseWheel { delta, .. }` → `InputMessage::PointerAxis`

The conversion functions should return `Option<InputMessage>` (some events we don't care about).

- [ ] **Step 2: Implement server input injection**

In `crates/wrsrvd/src/state.rs`, add a method:

```rust
impl WayRay {
    pub fn inject_network_input(&mut self, msg: InputMessage) {
        match msg {
            InputMessage::Keyboard(ev) => {
                let serial = SERIAL_COUNTER.next_serial();
                let keyboard = self.seat.get_keyboard().unwrap();
                let state = match ev.state {
                    KeyState::Pressed => smithay::backend::input::KeyState::Pressed,
                    KeyState::Released => smithay::backend::input::KeyState::Released,
                };
                keyboard.input::<(), _>(
                    self, ev.keycode, state, serial, ev.time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputMessage::PointerMotion(ev) => {
                // Similar to process_input_event's absolute motion handler
                // Transform (ev.x, ev.y) to output coordinates
                // Find surface under pointer, call pointer.motion()
            }
            InputMessage::PointerButton(ev) => {
                // Similar to process_input_event's button handler
                // Click-to-focus + forward button event
            }
            InputMessage::PointerAxis(ev) => {
                // Similar to process_input_event's axis handler
            }
        }
    }
}
```

This reuses the same patterns from the existing `process_input_event` method but takes protocol messages instead of backend input events.

- [ ] **Step 3: Wire input into the calloop event loop**

In the server's main loop, check the input channel from the network thread. When input arrives, call `inject_network_input`. Use `calloop::channel::Channel` so the calloop event loop wakes up when input is available.

- [ ] **Step 4: Verify**

```bash
cargo fmt --all && cargo clippy --workspace
```

- [ ] **Step 5: Commit**

```bash
git add crates/ tests/ Cargo.toml Cargo.lock && git commit -m "Add input forwarding from client to server

Client captures winit keyboard/mouse events, serializes as protocol
messages, sends over QUIC input channel.
Server receives and injects into Smithay seat."
```

---

## Task 7: End-to-End Integration

**Files:**
- Modify: `crates/wrsrvd/src/main.rs` (wire encoder + network into headless render loop)
- Modify: `crates/wrsrvd/src/backend/headless.rs` (add frame capture + encode + send)
- Modify: `crates/wrclient/src/main.rs` (wire network + decoder + display)

This task connects all the pieces: headless render → encode → QUIC → decode → display.

- [ ] **Step 1: Wire server pipeline**

In the headless backend's render loop, after each render:
1. Get the pixel buffer from the PixmanRenderer's render target
2. Call `encoder::xor_diff()` against the previous frame
3. For each damage rectangle from the damage tracker, call `encoder::encode_region()`
4. Package as `FrameUpdate` and send via the frame channel to the network thread
5. Store the current frame as the new "previous frame"

- [ ] **Step 2: Wire client pipeline**

In wrclient's main loop:
1. Network thread receives `FrameUpdate`, sends to main thread via channel
2. Main thread processes each `DamageRegion` through `decoder::apply_region()`
3. Upload updated framebuffer to wgpu texture
4. Request redraw

- [ ] **Step 3: Add the default QUIC port**

Server listens on `0.0.0.0:4433`. Client connects to `<host>:4433`. Make the port configurable via CLI arg or environment variable.

- [ ] **Step 4: Test end-to-end on Linux**

On the Linux host:
```bash
cargo run --bin wrsrvd &
WAYLAND_DISPLAY=<socket> foot &
```

On your Mac (after building wrclient):
```bash
cargo run --bin wrclient -- <linux-host-ip>:4433
```

Expected: The wrclient window on your Mac shows the foot terminal running on the Linux server. Typing in the wrclient window should produce text in foot.

- [ ] **Step 5: Fix any issues found during testing**

Common issues to watch for:
- Byte order mismatches (ARGB vs BGRA)
- Stride/alignment differences between PixmanRenderer and wgpu texture
- QUIC stream ordering and framing
- Input coordinate mapping (client window size vs server output size)

- [ ] **Step 6: Final cleanup**

```bash
cargo fmt --all && cargo clippy --workspace
cargo test --workspace
```

Fix all warnings and dead code.

- [ ] **Step 7: Commit**

```bash
git add crates/ tests/ Cargo.toml Cargo.lock && git commit -m "Wire end-to-end remote display pipeline

Headless render → XOR diff + zstd → QUIC → decompress + apply → wgpu display.
Input forwarding: winit capture → QUIC → Smithay seat injection.
Phase 1 complete: remote desktop works over the network."
```

---

## Notes for the Implementer

### Smithay PixmanRenderer API

The PixmanRenderer API in Smithay 0.7 may require experimentation. Key points:
- `PixmanRenderer::new()` — creates a software renderer
- `renderer.create_buffer(Fourcc::Argb8888, size)` — creates a render target
- After rendering, the pixel data is in the pixman Image's memory — accessible via unsafe pointer access
- The `ExportMem` trait IS implemented for PixmanRenderer and can be used as a fallback
- Consult Smithay's Anvil example (it has a PixmanRenderer path)

### quinn async / calloop sync bridge

quinn requires tokio, but the compositor uses calloop. The bridge:
1. Spawn a `std::thread` running a tokio runtime
2. The tokio runtime runs the QUIC server/client
3. Communication between threads uses `std::sync::mpsc` channels
4. On the calloop side, use `calloop::channel::Channel` as an event source so the event loop wakes when network data arrives

### wgpu texture upload

The key wgpu API for uploading pixel data:
```rust
queue.write_texture(
    texture.as_image_copy(),
    &pixel_data,
    wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(width * 4),
        rows_per_image: Some(height),
    },
    texture_size,
);
```

### Cross-compilation

wrclient needs to build on macOS. All its dependencies (winit, wgpu, quinn) are cross-platform. No special setup needed — `cargo build --bin wrclient` on macOS should work.

### What's Next (Phase 2.5)

With the remote display pipeline working, Phase 2.5 adds:
- Pluggable window management protocol
- Built-in floating WM (windows currently stack at 0,0)
- Session management foundations

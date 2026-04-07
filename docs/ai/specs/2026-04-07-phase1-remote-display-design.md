# Phase 1: Remote Display Pipeline — Design Spec

## Goal

Build a working remote display pipeline: `wrsrvd` runs headless on a
Linux server, composites Wayland clients with PixmanRenderer, encodes
frame diffs with zstd, and sends them over QUIC to `wrclient` running
on macOS, which displays them in a native Cocoa window. Input flows
back from client to server.

This combines the roadmap's Phase 1 (protocol + transport + encoding)
and Phase 2 (client viewer) into a single deliverable, producing an
end-to-end working system.

## Context

Phase 0 built a working Smithay compositor with Winit backend, but
visual testing requires a display server — which is broken under UTM's
virtualized GPU. The solution: make the server headless (no display
server needed) and move the visual output to the client over the
network. This is the actual WayRay production architecture.

## Architecture

```
wrsrvd (Linux, headless)              wrclient (macOS)
─────────────────────                 ──────────────────
Wayland clients render
  → PixmanRenderer composites
  → OutputDamageTracker finds dirty rects
  → XOR diff against previous frame
  → zstd compress per region
  → QUIC Stream 1 (display)    →→→    Receive FrameUpdate
                                      → zstd decompress
                                      → XOR apply to local framebuffer
                                      → Upload to GPU texture (wgpu)
                                      → Display in winit/Cocoa window

  ← QUIC Stream 2 (input)     ←←←    Capture keyboard/mouse (winit)
  → Inject into Smithay seat          → Serialize as protocol messages
```

## Deliverables

### 1. Wire Protocol (`wayray-protocol` crate)

Message types serialized with `postcard` (compact binary, serde-based,
no-std compatible).

**Control messages (QUIC Stream 0 — bidirectional):**

| Message | Fields | Direction |
|---------|--------|-----------|
| `ClientHello` | `version: u32`, `capabilities: Vec<String>` | client→server |
| `ServerHello` | `version: u32`, `session_id: u64`, `output_width: u32`, `output_height: u32` | server→client |
| `Ping` | `timestamp: u64` | either |
| `Pong` | `timestamp: u64` | either |

**Display messages (QUIC Stream 1 — server→client, unidirectional):**

| Message | Fields |
|---------|--------|
| `FrameUpdate` | `sequence: u64`, `regions: Vec<DamageRegion>` |
| `DamageRegion` | `x: u32`, `y: u32`, `width: u32`, `height: u32`, `data: Vec<u8>` (zstd-compressed XOR diff) |

**Input messages (QUIC Stream 2 — client→server, unidirectional):**

| Message | Fields |
|---------|--------|
| `KeyboardEvent` | `keycode: u32`, `state: KeyState`, `time: u32` |
| `PointerMotion` | `x: f64`, `y: f64`, `time: u32` |
| `PointerButton` | `button: u32`, `state: ButtonState`, `time: u32` |
| `PointerAxis` | `axis: Axis`, `value: f64`, `time: u32` |

**Flow control:** Client sends `FrameAck { sequence: u64 }` on Stream 0
after processing each frame. Server limits in-flight frames to prevent
buffer bloat.

**Serialization:** Each message is length-prefixed (4-byte little-endian
length, then postcard-encoded payload) for framing on QUIC streams.

### 2. QUIC Transport

**Server (wrsrvd):** quinn-based QUIC listener. Accepts one client at a
time (multi-client comes with session management in Phase 3).

**Client (wrclient):** quinn-based QUIC connection to the server.

**TLS:** Self-signed certificates generated at first run, cached to
`~/.config/wayray/`. Client uses `danger_accept_any_cert` for now —
trust-on-first-use and proper PKI deferred to Phase 3.

**Stream model (logical channels, not literal QUIC stream IDs):**
- Control channel: Bidirectional stream — hello, ping/pong, frame ack
- Display channel: Unidirectional server→client stream — frame updates
- Input channel: Unidirectional client→server stream — input events

Each channel opens its own QUIC stream at connection time. QUIC assigns
the actual stream IDs based on directionality and open order.

### 3. Headless Backend (wrsrvd)

Replace the Winit backend as the default with a headless backend using
PixmanRenderer.

**PixmanRenderer path:**
- Create `PixmanRenderer` (no GPU, no EGL, no display server)
- Allocate an in-memory framebuffer as the render target
- Render with `OutputDamageTracker` for damage tracking
- Read pixels directly from RAM (no `ExportMem` needed)
- Drive the render loop on a calloop timer (render-on-damage, cap 60fps)

**Backend selection via CLI:**
- `wrsrvd` — headless (default)
- `wrsrvd --backend winit` — Winit window (dev/debug, requires display)

**Winit backend stays** as-is in a separate module, feature-gated or
behind the CLI flag.

### 4. Frame Encoding (Tier 1)

Simplest viable encoding — optimize later.

**Encoding (server):**
1. After render, get the new framebuffer (ARGB8888 pixels in RAM)
2. XOR against the previous frame to produce a diff
3. For each damage rectangle from OutputDamageTracker:
   - Extract the XOR diff region
   - Compress with zstd (level 1 — fast)
   - Package as a `DamageRegion` in the `FrameUpdate` message
4. Store current frame as the new "previous frame"

**Decoding (client):**
1. Receive `FrameUpdate`
2. For each `DamageRegion`:
   - Decompress with zstd
   - XOR-apply onto the local framebuffer copy at the given position
3. Upload the updated framebuffer to a GPU texture
4. Display

**Why XOR + zstd:** XOR diff makes unchanged pixels zero. zstd compresses
runs of zeros extremely well. For typical desktop content (mostly static
with small changes), this gives 10-100x compression with minimal CPU.

### 5. Client Viewer (wrclient)

Native application using winit + wgpu.

**Window:** winit creates a platform-native window (Cocoa on macOS).
Sized to match the server's output dimensions from `ServerHello`.

**Rendering:** wgpu renders a fullscreen textured quad. Each frame,
the updated pixel buffer is uploaded to a GPU texture and drawn.
Double-buffered with vsync.

**Input capture:** winit captures keyboard and mouse events from the
native window. Events are serialized as protocol messages and sent
over QUIC Stream 2.

**CLI:**
```
wrclient <host>:<port>
```

## Dependencies

**New workspace dependencies (`Cargo.toml`):**

| Crate | Purpose |
|-------|---------|
| `quinn` | QUIC transport |
| `rustls` | TLS for QUIC |
| `rcgen` | Self-signed certificate generation |
| `postcard` + `serde` | Wire protocol serialization |
| `zstd` | Frame compression |
| `winit` | Window creation (wrclient) |
| `wgpu` | GPU rendering (wrclient) |

**Smithay feature changes (wrsrvd):**
- Add: `renderer_pixman` (headless rendering)
- Keep: `wayland_frontend`, `desktop`, `renderer_gl`, `backend_winit`
  (behind feature flag for dev mode)

## Module Structure

```
crates/
├── wayray-protocol/src/
│   ├── lib.rs          # Re-exports
│   ├── messages.rs     # All message types + serde derives
│   ├── codec.rs        # Length-prefixed framing (encode/decode)
│   └── version.rs      # Protocol version constant
│
├── wrsrvd/src/
│   ├── main.rs         # CLI arg parsing, backend selection
│   ├── state.rs        # WayRay compositor state (unchanged)
│   ├── handlers/       # Wayland protocol handlers (unchanged)
│   ├── backend/
│   │   ├── mod.rs      # Backend trait/enum
│   │   ├── headless.rs # PixmanRenderer + in-memory framebuffer
│   │   └── winit.rs    # Existing Winit backend (moved from main.rs)
│   ├── render.rs       # Render logic (adapted for both backends)
│   ├── encoder.rs      # XOR diff + zstd compression
│   └── network/
│       ├── mod.rs      # Re-exports
│       └── server.rs   # QUIC server, frame sending, input receiving
│
├── wrclient/src/
│   ├── main.rs         # CLI args, connect, main loop
│   ├── decoder.rs      # zstd decompress + XOR apply
│   ├── display.rs      # winit window + wgpu rendering
│   ├── input.rs        # Keyboard/mouse capture + serialization
│   └── network/
│       ├── mod.rs      # Re-exports
│       └── client.rs   # QUIC client, frame receiving, input sending
│
└── wradm/src/
    └── main.rs         # Unchanged (placeholder)
```

## Success Criteria

1. `wrsrvd` starts headless on the Linux host, no display server needed
2. `wrclient` connects from macOS over the local network
3. Launch a Wayland client (foot, weston-terminal) into wrsrvd
4. The client window appears on the Mac in the wrclient window
5. Typing and clicking in the wrclient window reaches the Wayland client
6. Frame updates are visible in real-time

## Design Notes

**Cursor:** Rendered server-side into the framebuffer for Phase 1
(simplest). No separate cursor channel or client-side cursor.

**Input injection:** Network input events are injected into the Smithay
seat by calling the seat's keyboard/pointer methods directly (same
pattern as the existing Winit input handler in `state.rs`). No custom
`InputBackend` needed.

**Protocol evolution:** The message types defined here are Phase 1
subsets. They will evolve toward the full wire protocol spec
(`docs/protocols/wayray-wire-protocol.md`) in later phases.

**PixmanRenderer pixel access:** Verify early that PixmanRenderer's
framebuffer can be read directly from RAM. If Smithay requires
`ExportMem` even for Pixman, adapt accordingly — Pixman's `ExportMem`
is a simple memcpy (no GPU readback).

## Out of Scope

- Audio forwarding (Phase 4)
- USB forwarding (Phase 4)
- Session management / hot-desking (Phase 3)
- Content-adaptive encoding (Tier 2/3 — future optimization)
- Hardware video encoding (H.264/AV1 — future optimization)
- Multi-client support (Phase 3)
- Proper TLS certificate validation (Phase 3)
- Window management protocol (Phase 2.5)

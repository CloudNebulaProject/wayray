# ADR-016: GPU Pipeline Future-Proofing (Cloud Gaming Path)

## Status
Accepted

## Context
WayRay's current design prioritizes the no-GPU headless path (PixmanRenderer + CPU encoding). This is correct for the primary thin client use case. However, a cloud gaming scenario -- GPU-rendered games streamed to thin clients -- is a natural future extension. We must ensure today's design decisions don't close that door.

### The Cloud Gaming Pipeline

```
Current (CPU, works in zones):
  App → wl_shm (CPU) → Pixman composite (CPU) → memcpy → CPU encode → QUIC
  Latency: ~15-30ms encode
  Quality: good for desktop, too slow for gaming

Future (GPU, zero-copy):
  Game → DMA-BUF (GPU) → GLES composite (GPU) → DMA-BUF → VAAPI/NVENC (GPU) → QUIC
  Latency: ~3-5ms encode
  Quality: 1080p60 or 4K60 with hardware encoding
  CPU: barely involved
```

The zero-copy GPU path means pixels never touch CPU memory: the game renders to a DMA-BUF, the compositor composites into another DMA-BUF, and the hardware encoder reads that DMA-BUF directly. This is how Sunshine/Moonlight, GeForce NOW, and Steam Remote Play achieve sub-20ms motion-to-photon latency.

### What Could Block This

If we make any of these mistakes now, GPU encoding becomes a painful retrofit later:

| Mistake | Why it blocks GPU path |
|---------|----------------------|
| Encoder takes only `&[u8]` pixel data | Forces GPU→CPU readback, kills zero-copy |
| Frame pipeline is synchronous | Can't pipeline render→encode→send |
| Damage tracking assumes CPU-accessible buffers | Can't diff GPU-resident frames |
| Frame cadence is damage-only | Gaming needs consistent 60/120fps |
| Compositor hardcodes PixmanRenderer | Can't composite on GPU |

## Decision

### Design constraints to preserve the GPU path:

### 1. Frame Source Abstraction

The encoder must accept both CPU buffers and GPU buffer handles:

```rust
/// A frame ready for encoding. Can be CPU pixels or a GPU buffer handle.
enum FrameSource<'a> {
    /// CPU-accessible pixel data (from PixmanRenderer / ExportMem)
    Cpu {
        data: &'a [u8],
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    },

    /// GPU buffer handle (from GlesRenderer / DRM)
    /// Encoder can import this directly without CPU copy
    DmaBuf {
        fd: OwnedFd,
        width: u32,
        height: u32,
        stride: u32,
        format: DrmFourcc,
        modifier: DrmModifier,
    },
}
```

A CPU-based encoder (zstd diff, software x264) processes `FrameSource::Cpu`.
A hardware encoder (VAAPI, NVENC) prefers `FrameSource::DmaBuf` (zero-copy) but can also accept `FrameSource::Cpu` (upload first, slower).

### 2. Encoder Trait

```rust
trait FrameEncoder: Send {
    /// What frame sources this encoder supports
    fn supported_sources(&self) -> &[FrameSourceType];

    /// Encode a frame, returning encoded data for transmission.
    /// `damage` hints which regions changed (encoder may ignore if doing full-frame).
    fn encode(
        &mut self,
        source: FrameSource<'_>,
        damage: &[Rectangle],
        frame_seq: u64,
    ) -> Result<EncodedFrame>;

    /// Signal that encoding parameters should adapt
    /// (e.g., client reported decode time too high)
    fn adapt(&mut self, feedback: EncoderFeedback);
}

enum FrameSourceType {
    Cpu,
    DmaBuf,
}
```

Planned encoder implementations:

| Encoder | Accepts | Use Case |
|---------|---------|----------|
| `ZstdDiffEncoder` | Cpu | Desktop, lossless text, LAN |
| `SoftwareH264Encoder` (x264) | Cpu | Desktop, lossy, WAN |
| `VaapiH264Encoder` | DmaBuf (preferred), Cpu | GPU-accelerated, gaming |
| `NvencH264Encoder` | DmaBuf (preferred), Cpu | NVIDIA GPU, gaming |
| `VaapiAv1Encoder` | DmaBuf (preferred), Cpu | Future, better quality |

### 3. Pipelined Frame Submission

The render→encode→send pipeline must be **asynchronous and pipelined**, not synchronous:

```
Synchronous (WRONG -- blocks on encode):
  Frame N:   [render]──[encode]──[send]
  Frame N+1:                            [render]──[encode]──[send]

Pipelined (RIGHT -- overlap work):
  Frame N:   [render]──[encode]──[send]
  Frame N+1:        [render]──[encode]──[send]
  Frame N+2:             [render]──[encode]──[send]
```

Implementation: the compositor submits frames to an encoding channel. The encoder runs on a separate thread (or GPU async queue). Encoded frames are submitted to the QUIC sender. Each stage can work on a different frame simultaneously.

```rust
// Compositor render loop (calloop)
fn on_frame_rendered(&mut self, frame: FrameSource, damage: Vec<Rectangle>) {
    // Non-blocking: submit to encoder channel
    self.encoder_tx.send(EncodeRequest { frame, damage, seq: self.next_seq() });
}

// Encoder thread
fn encoder_loop(rx: Receiver<EncodeRequest>, tx: Sender<EncodedFrame>) {
    for req in rx {
        let encoded = self.encoder.encode(req.frame, &req.damage, req.seq)?;
        tx.send(encoded);  // Submit to network sender
    }
}
```

### 4. Adaptive Frame Cadence

Two scheduling modes, selectable at runtime:

```rust
enum FrameCadence {
    /// Render only when Wayland clients commit new content.
    /// Coalesce rapid commits within a window (e.g., 4ms).
    /// Ideal for desktop: saves CPU when nothing changes.
    OnDamage { coalesce_ms: u32 },

    /// Render at a fixed rate regardless of client commits.
    /// Resubmit the last frame if nothing changed (keeps encoder fed).
    /// Needed for gaming: consistent framerate, no stutter.
    Fixed { fps: u32 },
}
```

The compositor defaults to `OnDamage`. When a surface is detected as "high frame rate" (e.g., commits faster than 30fps consistently), the compositor can switch to `Fixed` mode for that output. This could also be triggered by configuration (`wradm session set-cadence gaming`).

### 5. Damage Tracking Must Support Both Paths

For CPU buffers, we diff pixels directly (XOR previous frame, zstd compress diff).
For GPU buffers, we can't cheaply read pixels for diffing. Instead:

- **Wayland damage regions**: Every `wl_surface.commit()` includes damage hints from the client. These are already tracked by Smithay's `OutputDamageTracker`.
- **Full-frame encoding**: Hardware encoders (H.264, AV1) handle their own temporal redundancy via P-frames and reference frames. They don't need pixel-level diffs from us.

So the damage path forks:

```
CPU path:  pixel diff (XOR + zstd)  → needs pixel access    → FrameSource::Cpu
GPU path:  Wayland damage hints     → no pixel access needed → FrameSource::DmaBuf
           + H.264 temporal coding    encoder handles the rest
```

Both paths are valid. The encoder trait abstracts this -- a zstd encoder uses damage for pixel diff regions, a VAAPI encoder uses damage to decide keyframe frequency or quality.

### 6. GPU Partitioning (Infrastructure, Not WayRay)

For multi-session gaming, the GPU must be shared:

| Technology | What it does | Platform |
|-----------|-------------|----------|
| SR-IOV | Hardware GPU partitioning | Intel (Flex/Arc), some AMD |
| NVIDIA MIG | Multi-Instance GPU slicing | NVIDIA A100/A30/H100 |
| NVIDIA vGPU | Virtual GPU via hypervisor | NVIDIA datacenter GPUs |
| Intel GVT-g | Mediated GPU passthrough | Intel integrated GPUs |
| Virtio-GPU | Paravirtualized GPU for VMs | QEMU/KVM |

This is infrastructure configuration, not compositor code. WayRay just needs to see a `/dev/dri/renderD*` device in its environment. Whether that device is a physical GPU, an SR-IOV VF, a MIG slice, or a vGPU instance is transparent.

**WayRay's responsibility**: detect available GPU, use it if present, fall back gracefully if not. Already covered by ADR-005 (dual renderer).

## What We Build Now vs Later

### Build Now (Phase 0-2)
- `FrameSource` enum with `Cpu` variant only (no DmaBuf yet)
- `FrameEncoder` trait with the full interface (including `supported_sources`)
- `ZstdDiffEncoder` and `SoftwareH264Encoder` (Cpu only)
- Pipelined encode channel (encoder runs on separate thread)
- `FrameCadence::OnDamage` as default
- PixmanRenderer path

### Build Later (when GPU gaming is prioritized)
- Add `FrameSource::DmaBuf` variant
- `VaapiH264Encoder` / `NvencEncoder` (import DMA-BUF, zero-copy encode)
- `FrameCadence::Fixed` mode
- GlesRenderer path with DMA-BUF export to encoder
- Client-side frame interpolation for network jitter compensation
- Input prediction (client-side speculative input processing)

### Never Build (out of scope)
- GPU partitioning infrastructure (SR-IOV, MIG setup)
- Game streaming as a separate product (WayRay is a compositor, not GeForce NOW)
- Client-side GPU rendering (the client stays dumb)

## Rationale

- **Trait-based encoder** with `FrameSource` enum is the critical abstraction. It costs nothing now (we only implement the Cpu variant) but preserves the zero-copy DMA-BUF path for later.
- **Pipelined encoding** improves desktop performance too (compositor doesn't block on encode), so it's not wasted work.
- **Adaptive frame cadence** is a minor addition that enables gaming without disrupting desktop behavior.
- **No premature GPU code**: we don't write VAAPI/NVENC encoders now. We just ensure the interfaces don't preclude them.

## Consequences

- `FrameSource` enum adds one level of indirection to the encoding path (trivial cost)
- Pipelined encoding adds threading complexity (but improves performance)
- Must resist the temptation to simplify the encoder trait to `fn encode(&[u8]) -> Vec<u8>` -- the DmaBuf variant must stay in the design even if unimplemented
- Documentation must explain the GPU future path so contributors don't accidentally close the door

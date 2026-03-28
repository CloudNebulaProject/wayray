# ADR-005: Rendering Strategy

## Status
Accepted

## Context
The WayRay server compositor must render Wayland client surfaces into a framebuffer for network transmission. The server may or may not have a GPU. The rendering path must support efficient framebuffer capture.

## Options Considered

### 1. GlesRenderer only (GPU required)
- Hardware-accelerated compositing
- ExportMem via glReadPixels (GPU -> CPU readback)
- Requires EGL/OpenGL ES 2.0 capable GPU on server
- Can feed DMA-BUF directly to hardware video encoder (zero-copy)

### 2. PixmanRenderer only (CPU software rendering)
- Pure software rendering, no GPU needed
- ExportMem is essentially free (pixels already in CPU memory)
- Lower throughput for complex scenes
- Perfect for headless server deployments (cloud VMs, containers)

### 3. Dual renderer with runtime selection
- PixmanRenderer as default/fallback
- GlesRenderer when GPU is available
- Configuration-driven or auto-detected
- Covers both headless and GPU-equipped deployments

## Decision
**Dual renderer with runtime selection.** PixmanRenderer as the default, GlesRenderer when a GPU is available and the admin opts in.

## Rationale
- Many server deployments are headless VMs or containers without GPUs
- PixmanRenderer gives us zero-overhead framebuffer capture (no GPU readback needed)
- GlesRenderer + DMA-BUF export to VAAPI enables zero-copy hardware encoding
- The Smithay Renderer trait makes both interchangeable
- Niri and cosmic-comp demonstrate this dual-renderer pattern successfully

## Rendering Pipeline

### PixmanRenderer path (default)
```
Wayland clients commit surfaces
  -> PixmanRenderer composites all surfaces into output buffer
  -> OutputDamageTracker identifies changed regions
  -> copy_framebuffer() returns pixels (zero-copy, already in RAM)
  -> Frame encoder processes damaged regions
  -> Transmit to client
```

### GlesRenderer path (GPU available)
```
Wayland clients commit surfaces (may use DMA-BUF)
  -> GlesRenderer composites via OpenGL ES
  -> OutputDamageTracker identifies changed regions
  -> Option A: copy_framebuffer() via glReadPixels -> CPU -> encode
  -> Option B: DMA-BUF export -> VAAPI encoder (zero-copy)
  -> Transmit to client
```

## Frame Cadence
- Render on damage (not fixed rate): only render when clients commit new content
- Cap at client's display refresh rate (sent during capability exchange)
- During suspension (no client connected): stop rendering entirely
- Coalesce rapid commits: batch within ~4ms window to avoid excessive rendering

## Consequences
- PixmanRenderer cannot handle GPU-only client buffers (DMA-BUF with no CPU-accessible format)
  - Mitigation: advertise only SHM formats when running with PixmanRenderer
- glReadPixels is a synchronization point that stalls the GPU pipeline
  - Mitigation: use PBO (Pixel Buffer Objects) for async readback
- Dual renderer adds code complexity; managed via Smithay's trait-based abstraction

# ADR-003: Frame Encoding Strategy

## Status
Accepted

## Context
WayRay must transmit rendered Wayland compositor output from server to client efficiently. The encoding must balance bandwidth, latency, and visual quality. Different regions of the display have different characteristics (static UI vs video vs text) that benefit from different encoding strategies.

## Options Considered

### 1. Full-frame video encoding (H.264/AV1 everything)
- Simple pipeline: render -> encode entire frame -> transmit -> decode
- Good for video-heavy workloads
- Introduces compression artifacts on text and UI elements
- Consistent latency characteristics
- Hardware encoding available (VAAPI, NVENC)

### 2. Differential lossless (like waypipe)
- XOR diff against previous frame, compress with zstd/lz4
- Perfect visual quality
- Very efficient for mostly-static displays
- Bandwidth spikes when large areas change
- No hardware acceleration

### 3. Content-adaptive hybrid (like SPICE)
- Heuristic classification of regions: text, UI, video, image
- Each region encoded with optimal codec
- Best quality-bandwidth tradeoff
- Most complex to implement
- SPICE proved this approach works at scale

### 4. Tile-based encoding
- Divide frame into fixed-size tiles (e.g., 64x64)
- Only encode tiles that changed (via damage tracking)
- Each tile encoded independently (parallel-friendly)
- Mix lossless and lossy per-tile based on content heuristics
- Good balance of complexity and effectiveness

## Decision
**Tile-based encoding with content-adaptive per-tile codec selection**, implemented progressively:

### Stage 1 (MVP): Differential lossless
- Use OutputDamageTracker damage rectangles
- XOR diff against previous frame for damaged regions
- Compress with zstd
- Good enough for development and LAN use

### Stage 2: Tile-based with lossy option
- Divide into 64x64 tiles, only process damaged tiles
- Lossless path: zstd-compressed diff (for text, UI)
- Lossy path: JPEG/WebP for photographic content
- Tile-level quality selection based on content entropy

### Stage 3: Hardware video encoding
- Detect video regions (rapidly changing tiles)
- Route video regions to H.264 encoder (VAAPI/NVENC)
- Keep text/UI regions lossless
- AV1 as alternative for better quality at same bitrate

## Frame Update Message Structure

```
FrameUpdate {
    sequence: u64,
    timestamp: u64,
    full_width: u32,
    full_height: u32,
    regions: Vec<EncodedRegion>,
}

EncodedRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    encoding: Encoding,  // Zstd, Jpeg, H264, Raw
    data: Vec<u8>,
}
```

## Rationale
- Progressive approach: ship MVP quickly, optimize later
- Damage tracking from Smithay gives us changed regions for free
- Tile-based approach is parallelizable (encode tiles on multiple cores)
- Content-adaptive encoding avoids the "text looks blurry" problem of pure video encoding
- Hardware encoding path available when needed for video content
- Lossless path ensures perfect text rendering (critical for terminal/code editors)

## Consequences
- Stage 1 is bandwidth-hungry on large screen changes (acceptable for LAN)
- Need heuristics for tile content classification (can start simple: entropy-based)
- Hardware encoding adds dependency on system GPU/driver support
- Client must handle mixed encodings within a single frame update

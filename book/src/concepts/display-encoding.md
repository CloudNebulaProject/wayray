# Display Encoding

WayRay uses a content-adaptive encoding strategy to efficiently transmit display updates from server to client.

## The Challenge

A 1920x1080 display at 60fps and 32-bit color is approximately 500 MB/s of raw pixel data. Even on a gigabit LAN, this is too much. We need smart compression.

## Damage Tracking

The first optimization: don't send what hasn't changed.

WayRay's Smithay compositor tracks **damage regions** -- the parts of the screen that actually changed between frames. When you type a character in a terminal, only a few pixels change. When a video plays, a rectangular region updates rapidly. When nothing happens, nothing is sent.

This alone reduces bandwidth by 90%+ for typical desktop use.

## Tile-Based Encoding

Damaged regions are divided into 64x64 pixel tiles. Each tile is processed independently, enabling:

- **Parallel encoding** across CPU cores
- **Per-tile codec selection** based on content
- **Efficient caching** -- unchanged tiles are never re-encoded

## Content-Adaptive Compression

Different content benefits from different encoding:

### Text and UI (Lossless)
Terminal text, code editors, menus, and UI elements must be pixel-perfect. These are encoded using:
- XOR diff against the previous frame (most pixels unchanged)
- zstd compression of the diff (excellent on sparse data)
- Result: perfect quality, very low bandwidth for typical changes

### Photographic Content (Lossy)
Photos, image previews, and complex graphics use:
- JPEG or WebP encoding
- Quality tuned to available bandwidth
- Minor artifacts acceptable; huge bandwidth savings

### Video Regions (Hardware Encoding)
Rapidly changing regions (video playback, animations) use:
- H.264 or AV1 video encoding
- Hardware acceleration via VAAPI or NVENC when available
- Optimized for low latency (no B-frames, constant bitrate)
- Detected automatically by tile change frequency

## Adaptive Bitrate

WayRay adjusts encoding quality based on network conditions:

| Condition | Strategy |
|-----------|----------|
| LAN (< 5ms, > 100 Mbps) | Minimal compression, high framerate |
| Good WAN (< 30ms, > 20 Mbps) | Moderate compression, 60fps |
| Poor WAN (> 50ms, < 5 Mbps) | Aggressive compression, reduced framerate |
| Packet loss detected | Increase keyframe frequency, lower quality |

The client reports frame decode times and network statistics, allowing the server to adapt in real-time.

## Frame Pipeline

```
1. Wayland clients commit surface updates
2. Compositor renders all surfaces (Pixman or GLES)
3. Damage tracker identifies changed tiles
4. Each changed tile is classified (text/photo/video)
5. Tiles encoded with appropriate codec
6. Encoded regions assembled into FrameUpdate message
7. Transmitted over QUIC display stream
8. Client receives, decodes, and composites onto display
9. Client sends FrameAck with decode timing
```

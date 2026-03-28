# Remote Display & Thin Client Technologies Research

Comprehensive research for building a SunRay-like thin client system. Covers protocols, capture mechanisms, encoding, networking, and audio/USB forwarding.

---

## Table of Contents

1. [SPICE Protocol](#1-spice-protocol)
2. [RDP (Remote Desktop Protocol)](#2-rdp-remote-desktop-protocol)
3. [VNC / RFB Protocol](#3-vnc--rfb-protocol)
4. [Waypipe](#4-waypipe)
5. [PipeWire Screen Capture](#5-pipewire-screen-capture)
6. [Video Codecs for Remote Display](#6-video-codecs-for-remote-display)
7. [Network Protocols](#7-network-protocols)
8. [Framebuffer Capture Techniques](#8-framebuffer-capture-techniques)
9. [Audio Forwarding](#9-audio-forwarding)
10. [USB/IP](#10-usbip)
11. [Modern Thin Client Projects](#11-modern-thin-client-projects)
12. [Architecture Recommendations](#12-architecture-recommendations-for-a-sunray-like-system)

---

## 1. SPICE Protocol

**SPICE** (Simple Protocol for Independent Computing Environments) is a remote display protocol originally developed by Qumranet (acquired by Red Hat). It is the most architecturally relevant existing protocol for a SunRay-like system.

### Architecture

SPICE has a four-component architecture:
- **Protocol**: Wire format specification for all messages
- **Server** (`libspice-server`): Runs inside the hypervisor/host, directly accesses the virtual GPU framebuffer
- **Client** (`spice-gtk`, `remote-viewer`): Renders display, captures input, handles USB/audio
- **Guest Agent** (`spice-vdagent`): Runs inside the guest VM for clipboard, resolution changes, file transfer

### Channel Architecture

Each SPICE session consists of **multiple independent TCP/TLS connections**, one per channel type:

| Channel | ID | Purpose |
|---|---|---|
| **Main** | 1 | Session management, migration, agent communication |
| **Display** | 2 | Rendering commands, images, video streams |
| **Inputs** | 3 | Keyboard and mouse events |
| **Cursor** | 4 | Pointer shape and position |
| **Playback** | 5 | Audio output (server -> client) |
| **Record** | 6 | Audio input (client -> server) |
| **Smartcard** | 8 | Smartcard passthrough |
| **USB Redir** | 9 | USB device forwarding via usbredir |
| **Port** | 10 | Generic data port |
| **Webdav** | 11 | File sharing via WebDAV |

### Display Channel & Image Compression

The display channel is the most complex. SPICE does **not** just send raw framebuffer pixels. Instead it sends **rendering commands** (draw operations, images, etc.) and tries to **offload rendering to the client GPU**.

**Image compression algorithms** (selectable at runtime):
- **Quic**: Proprietary algorithm based on SFALIC. Optimized for photographic/natural images
- **LZ**: Standard Lempel-Ziv. Good for text/UI content
- **GLZ** (Global LZ): LZ with a **history-based global dictionary** that exploits repeating patterns across images. Critical for WAN performance
- **Auto mode**: Heuristically selects Quic vs. LZ/GLZ per-image based on content type

**Video streaming**: The server **heuristically detects video regions** (rapidly changing rectangular areas) and encodes them as **M-JPEG streams**, dramatically reducing bandwidth for video playback.

**Caching**: Images, palettes, and cursor data are cached on the client side to avoid retransmission.

### Key Design Insights for WayRay

- The multi-channel approach allows independent QoS per data type
- Sending rendering commands rather than raw pixels is more bandwidth-efficient
- The automatic image compression selection based on content type is clever
- GLZ's global dictionary approach is excellent for WAN scenarios
- Video region detection and switching to video codec is a critical optimization

### Sources
- [SPICE Protocol Specification](https://www.spice-space.org/spice-protocol.html)
- [SPICE for Newbies](https://www.spice-space.org/spice-for-newbies.html)
- [SPICE Features](https://www.spice-space.org/features.html)
- [SPICE User Manual](https://www.spice-space.org/spice-user-manual.html)
- [SPICE Protocol PDF](https://www.spice-space.org/static/docs/spice_protocol.pdf)
- [SPICE Wikipedia](https://en.wikipedia.org/wiki/Simple_Protocol_for_Independent_Computing_Environments)

---

## 2. RDP (Remote Desktop Protocol)

**RDP** is Microsoft's proprietary remote desktop protocol, based on the ITU T.120 family of protocols. Default port: TCP/UDP 3389.

### Architecture

RDP uses a **client-server model** with a layered architecture:

1. **Transport Layer**: TCP/IP (traditional) or UDP (for lossy/real-time data)
2. **Security Layer**: TLS/NLA (Network Level Authentication)
3. **Core Protocol**: PDU (Protocol Data Unit) processing, state machine
4. **Virtual Channel System**: Extensible channel framework for features

**Server-side components**:
- `Wdtshare.sys`: RDP driver handling UI transfer, compression, encryption, framing
- `Tdtcp.sys`: Transport driver packaging the protocol onto TCP/IP

### Virtual Channel System

RDP's extensibility comes from its virtual channel architecture:

**Static Virtual Channels (SVC)**:
- Negotiated during connection setup
- Fixed for session lifetime
- Name limited to 8 bytes
- Examples: `RDPSND` (audio), `CLIPRDR` (clipboard), `RDPDR` (device redirection)

**Dynamic Virtual Channels (DVC)**:
- Built on top of the `DRDYNVC` static channel
- Can be opened/closed during a session
- Used for modern features: graphics pipeline, USB redirection, diagnostics
- Microsoft's recommended approach for new development

### Graphics Pipeline

RDP has evolved through several graphics approaches:

1. **GDI Remoting** (original): Send Windows GDI drawing commands
2. **RemoteFX Codec**: Wavelet-based (DWT + RLGR encoding), supports lossless and lossy modes
3. **RemoteFX Progressive Codec**: Progressive rendering for WAN - sends low quality first, refines incrementally
4. **GFX Pipeline** (`MS-RDPEGFX`): Modern graphics extension supporting:
   - AVC/H.264 encoding for video content
   - RemoteFX for non-video content
   - Adaptive selection based on content type and bandwidth

**Note**: RemoteFX vGPU was deprecated in 2020 due to security vulnerabilities; the codec itself lives on in the GFX pipeline.

### FreeRDP

[FreeRDP](https://github.com/FreeRDP/FreeRDP) is the dominant open-source RDP implementation (Apache 2.0 license):
- Written primarily in C (87.8%)
- Clean separation: `libfreerdp` (protocol) vs. client frontends vs. server implementations
- Powers Remmina, GNOME Connections, KRDC, and most Linux RDP clients
- Implements the full virtual channel system including GFX pipeline

### Key Design Insights for WayRay

- The SVC/DVC split is instructive: start with fixed channels, add dynamic ones later
- Progressive rendering is excellent for variable-bandwidth scenarios
- Content-adaptive encoding (H.264 for video, wavelet for desktop) is the modern approach
- FreeRDP's architecture (protocol library separate from client/server) is a good model

### Sources
- [Understanding RDP - Microsoft Learn](https://learn.microsoft.com/en-us/troubleshoot/windows-server/remote/understanding-remote-desktop-protocol)
- [RDP Wikipedia](https://en.wikipedia.org/wiki/Remote_Desktop_Protocol)
- [FreeRDP GitHub](https://github.com/FreeRDP/FreeRDP)
- [MS-RDPEGFX Specification](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0)
- [Graphics Encoding over RDP - Azure](https://learn.microsoft.com/en-us/azure/virtual-desktop/graphics-encoding)
- [RDP Virtual Channels - Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/termserv/terminal-services-virtual-channels)

---

## 3. VNC / RFB Protocol

**VNC** (Virtual Network Computing) uses the **RFB** (Remote Framebuffer) protocol, standardized in [RFC 6143](https://www.rfc-editor.org/rfc/rfc6143.html).

### Architecture

RFB is a **simple, stateless framebuffer protocol**. The fundamental design:

- The display side is based on a single primitive: **"put a rectangle of pixel data at position (x, y)"**
- A sequence of rectangles makes a **framebuffer update**
- The protocol is **client-pull**: the client requests updates, the server sends them
- **Pixel format** is negotiated: 24-bit true color, 16-bit, or 8-bit color-mapped

### Encoding Types

The encoding system is the key to VNC performance. Different encodings trade off bandwidth, client CPU, and server CPU:

| Encoding | Description | Best For |
|---|---|---|
| **Raw** | Uncompressed pixel data, scanline order | Fast LAN, low CPU |
| **CopyRect** | Reference to existing framebuffer region | Window moves, scrolling |
| **RRE** | Rise-and-Run-length Encoding, rectangles of solid color | Simple UIs |
| **Hextile** | 16x16 tile subdivision with RRE within tiles | Fast LAN (low CPU overhead) |
| **Zlib** | Raw data compressed with zlib | Moderate bandwidth savings |
| **Tight** | Intelligent per-rectangle compression selection (zlib, JPEG, indexed color, solid) | Low bandwidth / WAN |
| **ZRLE** | Zlib Run-Length Encoding, combines zlib with palette/RLE | Good all-around |
| **TurboVNC/Tight+JPEG** | Tight with aggressive JPEG for photographic regions | Video content, high FPS |

**Pseudo-encodings** allow clients to advertise extension support (cursor shape, desktop resize, etc.) without changing the core protocol.

### Performance Characteristics

- **Fast LAN**: Hextile or Raw (minimize CPU overhead)
- **WAN/Low bandwidth**: Tight (best compression ratios, especially for mixed content)
- **Photo/Video content**: Tight with JPEG (TurboVNC achieves 4x better performance than ZRLE for images)
- **Scrolling/Window moves**: CopyRect (near-zero bandwidth)

### Key Design Insights for WayRay

- CopyRect-style "reference previous frame data" is extremely efficient for common desktop operations
- Per-rectangle encoding selection (as in Tight) is superior to one-size-fits-all
- RFB's simplicity is both its strength (easy to implement) and weakness (no audio, USB, etc.)
- The client-pull model introduces latency; a push model with damage tracking is better

### Sources
- [RFC 6143 - The Remote Framebuffer Protocol](https://www.rfc-editor.org/rfc/rfc6143.html)
- [RFB Protocol Documentation](https://vncdotool.readthedocs.io/en/0.8.0/rfbproto.html)
- [RFB Protocol Wikipedia](https://en.wikipedia.org/wiki/RFB_protocol)
- [VNC Tight Encoder Comparison](https://www.tightvnc.com/archive/compare.html)
- [TigerVNC RFB Protocol](https://github.com/svn2github/tigervnc/blob/master/rfbproto/rfbproto.rst)

---

## 4. Waypipe

**Waypipe** is a proxy for Wayland clients, analogous to `ssh -X` for X11. It is the most directly relevant existing project for Wayland remote display.

### Architecture

Waypipe operates as a **paired proxy** system:

```
[Remote App] <--Wayland--> [waypipe server] <--socket/SSH--> [waypipe client] <--Wayland--> [Local Compositor]
```

- **Server mode**: Acts as a Wayland compositor stub on the remote side. Wayland apps connect to it as if it were a real compositor.
- **Client mode**: Connects to the local real compositor and forwards surface updates from the remote side.
- **SSH integration**: `waypipe ssh user@host app` sets up the tunnel automatically.

### Buffer Synchronization

This is the key technical innovation:

1. Waypipe keeps a **mirror copy** of each shared memory buffer
2. When a buffer is committed, waypipe **diffs** the current buffer against the mirror
3. Only **changed regions** are transmitted
4. The remote side applies the diff to reconstruct the buffer

### Compression Options

| Method | Use Case | Default |
|---|---|---|
| **none** | High-bandwidth LAN | No |
| **lz4** | General purpose, fast | Yes (default) |
| **zstd** | Low-bandwidth / WAN | No |

Compression ratios: 30x for text-heavy content, down to 1.5x for noisy images.

### Video Encoding (DMA-BUF)

For DMA-BUF buffers (GPU-rendered content), waypipe supports **lossy video encoding**:

- `--video=sw,bpf=120000,h264` (default when `--video` is used)
- **Software encoding** (libx264) or **hardware encoding** (VAAPI)
- With VAAPI on Intel Gen8 iGPU: **80 FPS at 4 MB/s bandwidth**
- Configurable bits-per-frame for quality/bandwidth tradeoff

### Protocol Handling

Waypipe parses the Wayland wire protocol, which is **partially self-describing**. It:
- Intercepts buffer-related messages (wl_shm, wl_buffer, linux-dmabuf)
- Passes through other messages transparently
- Is partially forward-compatible with new Wayland protocols

### Limitations

- Per-application, not whole-desktop
- No built-in audio forwarding
- No USB forwarding
- Performance depends heavily on application rendering patterns
- Latency can be noticeable for interactive use

### Key Design Insights for WayRay

- The diff-based buffer synchronization is very efficient for incremental updates
- VAAPI video encoding for DMA-BUF is the right approach for GPU-rendered content
- Per-application forwarding is limiting; a whole-compositor approach is better for a thin client
- The Wayland protocol's design (buffer passing, damage tracking) is well-suited for remote display

### Sources
- [Waypipe GitHub](https://github.com/neonkore/waypipe)
- [Waypipe Man Page](https://man.archlinux.org/man/extra/waypipe/waypipe.1.en)
- [GSOC 2019 - Waypipe Development Blog](https://mstoeckl.com/notes/gsoc/blog.html)
- [Waypipe DeepWiki](https://deepwiki.com/neonkore/waypipe/2-getting-started)

---

## 5. PipeWire Screen Capture

PipeWire is the modern Linux multimedia framework that unifies audio, video, and screen capture.

### Portal-Based Screen Capture Architecture

On Wayland, screen capture follows a **security-first architecture**:

```
[Application] --> [xdg-desktop-portal (D-Bus)] --> [Portal Backend (compositor-specific)]
                                                         |
                                                    [PipeWire Stream]
                                                         |
                                                  [Application receives frames]
```

**Flow**:
1. Application calls `org.freedesktop.portal.ScreenCast.CreateSession()` via D-Bus
2. Portal presents a permission dialog to the user
3. On approval, `SelectSources()` lets user choose output/window
4. `Start()` creates a PipeWire stream and returns a `pipewire_fd`
5. Application connects to PipeWire using this fd and receives frames

### Buffer Sharing Mechanisms

PipeWire supports two buffer types for screen capture:

**DMA-BUF (preferred)**:
- Zero-copy transfer from compositor GPU memory to consumer
- Buffer stays in GPU VRAM throughout the pipeline
- Ideal for hardware video encoding (capture -> encode without CPU copy)
- Format/modifier negotiation ensures compatibility

**memfd (fallback)**:
- Shared memory file descriptor
- Requires CPU copy from GPU to system memory
- Universal compatibility but higher overhead

### Wayland Capture Protocols

Three generations of capture protocols exist:

1. **wlr-export-dmabuf-unstable-v1** (legacy): Exports entire output as DMA-BUF frames. Simple but no damage tracking.

2. **wlr-screencopy-unstable-v1** (deprecated): More flexible, supports shared memory and DMA-BUF. Has damage tracking via `copy_with_damage`. Being replaced.

3. **ext-image-copy-capture-v1** (current, merged 2024): The new standard protocol:
   - Client specifies which buffer regions need updating
   - Compositor only fills changed regions
   - Supports both output capture and window capture
   - Initial implementations: wlroots, WayVNC, grim

### GNOME's Approach

GNOME/Mutter uses different D-Bus APIs:
- `org.gnome.Mutter.ScreenCast`: Provides PipeWire stream of screen content
- `org.gnome.Mutter.RemoteDesktop`: Provides input injection
- These power `gnome-remote-desktop` which speaks RDP (and VNC)

### Key Design Insights for WayRay

- **ext-image-copy-capture-v1 + PipeWire** is the correct modern capture stack
- DMA-BUF capture -> hardware encode is the zero-copy golden path
- The portal system provides proper security/permission handling
- For a thin client server running its own compositor, you can skip the portal and use the capture protocols directly
- Damage tracking in ext-image-copy-capture-v1 is essential for efficient updates

### Sources
- [XDG Desktop Portal ScreenCast API](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
- [ext-image-copy-capture-v1 Protocol](https://wayland.app/protocols/ext-image-copy-capture-v1)
- [wlr-screencopy-unstable-v1](https://wayland.app/protocols/wlr-screencopy-unstable-v1)
- [wlr-export-dmabuf-unstable-v1](https://wayland.app/protocols/wlr-export-dmabuf-unstable-v1)
- [Wayland Merges New Screen Capture Protocols - Phoronix](https://www.phoronix.com/news/Wayland-Merges-Screen-Capture)
- [PipeWire ArchWiki](https://wiki.archlinux.org/title/PipeWire)
- [Niri Screencasting Implementation](https://deepwiki.com/niri-wm/niri/5.4-screencasting-and-screen-capture)

---

## 6. Video Codecs for Remote Display

### Codec Comparison for Low-Latency Use

| Property | H.264/AVC | H.265/HEVC | AV1 |
|---|---|---|---|
| **Compression efficiency** | Baseline | ~35% better than H.264 | ~50% better than H.264 |
| **Encoding latency** | Lowest | Low | Moderate (improving) |
| **Hardware encode support** | Universal | Widespread | Newer GPUs only |
| **Patent/license** | Licensed (but ubiquitous) | Licensed (complex) | Royalty-free |
| **Screen content coding** | Limited | Better | Best (dedicated tools) |
| **Decode support** | Universal | Nearly universal | Growing rapidly |
| **Best for** | Maximum compatibility | Good quality/bandwidth | Best quality, royalty-free |

### Low-Latency Encoding Considerations

For remote desktop, encoding latency is critical. Key settings:

**Frame structure**:
- **No B-frames**: B-frames require future frames, adding latency
- **No lookahead**: Lookahead improves quality but adds latency
- **No frame reordering**: Frames must be encoded/decoded in order
- **Single slice / low-delay profile**: Minimizes buffering

**Rate control**:
- **CBR (Constant Bit Rate)**: Keeps network queues short and predictable
- **VBR with max bitrate cap**: Better quality but can cause bandwidth spikes
- CBR is generally preferred for remote desktop due to predictable latency

**Intra refresh**:
- Periodic I-frames are large and cause bandwidth spikes
- **Gradual Intra Refresh (GIR)**: Spreads intra-coded blocks across frames, avoiding spikes
- Essential for smooth, low-latency streaming

### AV1 Specific Advantages

AV1 has features specifically useful for remote desktop:
- **Screen Content Coding (SCC)**: Dedicated tools for text, UI elements, and screen captures that dramatically reduce bitrate
- **Temporal Scalability (SVC)**: L1T2 mode (1 spatial layer, 2 temporal layers) allows dropping frames gracefully under bandwidth pressure
- **Film Grain Synthesis**: Can transmit film grain parameters instead of actual grain, saving bandwidth

Chrome's libaom AV1 encoder (speed 10): 12% better quality than VP9 at same bandwidth, 25% faster encoding.

### Hardware Encoding

#### NVIDIA NVENC

- Available on GeForce GTX 600+ and all Quadro/Tesla with Kepler+
- **Video Codec SDK v13.0** (2025): AV1 ultra-high quality mode, comparable to software AV1 encoding
- Latency modes:
  - **Normal Latency**: Default, uses B-frames and lookahead
  - **Low Latency**: No B-frames, no reordering
  - **Ultra Low Latency**: Strict in-order pipeline, minimal frame queuing
- Dedicated hardware encoder block (does not consume CUDA cores)
- Can encode 4K@120fps with sub-frame latency

#### Intel VAAPI (Video Acceleration API)

- Open-source API (`libva`) supported on Intel Gen8+ (Broadwell+)
- Supports H.264, H.265, AV1 (Intel Arc/Gen12+), VP9
- FFmpeg integration: `h264_vaapi`, `hevc_vaapi`, `av1_vaapi`
- Low-power encoding mode available on some platforms
- GStreamer integration via `gstreamer-vaapi`
- Well-suited for always-on server scenarios (low power consumption)

#### AMD AMF/VCN

- Video Core Next (VCN) hardware encoder
- Supports H.264, H.265, AV1 (RDNA 3+)
- AMF (Advanced Media Framework) SDK
- VAAPI support via Mesa `radeonsi` driver
- VCN 4.0+ competitive with NVENC in quality

### Key Design Insights for WayRay

- **Start with H.264** for maximum compatibility, add H.265/AV1 as options
- Use **VAAPI** as the primary encoding API (works across Intel/AMD, open-source)
- Add NVENC support via FFmpeg/GStreamer for NVIDIA GPUs
- **CBR + no B-frames + gradual intra refresh** for lowest latency
- AV1's screen content coding mode is a significant advantage for desktop content
- The **DMA-BUF -> VAAPI encode** path is zero-copy and should be the primary pipeline

### Sources
- [NVIDIA Video Codec SDK](https://developer.nvidia.com/video-codec-sdk)
- [NVENC Application Note](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-application-note/index.html)
- [NVIDIA AV1 Blog Post](https://developer.nvidia.com/blog/improving-video-quality-and-performance-with-av1-and-nvidia-ada-lovelace-architecture/)
- [GPU Video Encoder Evaluation](https://arxiv.org/html/2511.18688v2)
- [VA-API Intel Documentation](https://intel.github.io/libva/)
- [Hardware Video Acceleration ArchWiki](https://wiki.archlinux.org/title/Hardware_video_acceleration)
- [Chrome AV1 Improvements](https://developer.chrome.com/blog/av1)
- [CBR vs VBR for Game Streaming](https://pulsegeek.com/articles/cbr-vs-vbr-for-low-latency-game-streaming/)
- [AV1 SVC in WebRTC](https://w3c.github.io/webrtc-svc/)

---

## 7. Network Protocols

### TCP vs. UDP vs. QUIC for Remote Display

| Property | TCP | UDP | QUIC |
|---|---|---|---|
| **Reliability** | Full (retransmit) | None | Selectable per-stream |
| **Head-of-line blocking** | Yes (single stream) | No | No (multiplexed streams) |
| **Connection setup** | 1-3 RTT (TCP + TLS) | 0 RTT | 0-1 RTT |
| **Congestion control** | Kernel-space, slow to update | Application-managed | User-space, pluggable |
| **NAT/firewall traversal** | Good | Moderate | Moderate (UDP-based) |
| **Encryption** | Optional (TLS) | Optional (DTLS) | Mandatory (TLS 1.3) |

### QUIC Advantages for Remote Display

QUIC is increasingly compelling for remote display:

1. **Stream multiplexing without HOL blocking**: Display, input, audio can be separate QUIC streams. A lost display packet doesn't stall input delivery.
2. **0-RTT connection setup**: Critical for session resumption / hot-desking scenarios
3. **Pluggable congestion control**: Can use algorithms optimized for low-latency interactive traffic (e.g., BBR, COPA)
4. **Connection migration**: Session survives network changes (WiFi -> Ethernet)

### QUIC Challenges

- **Firewall blocking**: Some corporate networks block UDP, forcing TCP fallback. The fallback penalty is severe (full session teardown + TCP reconnect).
- **Library maturity**: QUIC implementations are still maturing. Key libraries:
  - **quinn** (Rust): Well-maintained, async, good for our use case
  - **quiche** (Cloudflare, Rust/C): Production-tested
  - **s2n-quic** (AWS, Rust): High performance
- **CPU overhead**: QUIC's encryption and user-space processing can be higher than kernel TCP

### Media over QUIC (MoQ)

MoQ is an emerging IETF standard (RFC expected 2026) that combines:
- Low-latency interactivity of WebRTC
- Scalability of HLS/DASH
- Built on QUIC/WebTransport

**Architecture**: Publish-subscribe model with tracks, groups, and objects. Sub-250ms latency target.

**Relevance**: MoQ's concepts (prioritized streams, partial reliability, adaptive quality) are directly applicable to remote display, though the protocol itself is focused on media distribution rather than interactive desktop.

**Implementations**: Cloudflare has deployed MoQ relays on their global network. OpenMOQ consortium (Akamai, Cisco, YouTube, etc.) developing open source implementations.

### Adaptive Bitrate for Remote Display

Key strategies:
- **Bandwidth estimation**: Measure RTT and throughput continuously
- **Quality adjustment**: Change encoder bitrate, resolution, or frame rate
- **Frame dropping**: Under extreme congestion, drop non-reference frames
- **Temporal scalability (SVC)**: Encode with multiple temporal layers, drop higher layers under congestion
- **Resolution scaling**: Encode at lower resolution and upscale on client (works well with modern upscaling algorithms)

### Latency Budget

For interactive remote desktop, the target end-to-end latency budget:

| Stage | Target |
|---|---|
| Capture | <1ms (DMA-BUF) |
| Encode | 1-5ms (hardware) |
| Network (LAN) | <1ms |
| Network (WAN) | 10-100ms |
| Decode | 1-3ms (hardware) |
| Render | <1ms |
| **Total (LAN)** | **<10ms** |
| **Total (WAN)** | **15-110ms** |

### Key Design Insights for WayRay

- **Use QUIC as primary transport** with TCP fallback
- Rust has excellent QUIC libraries (quinn)
- Separate QUIC streams for display, input, audio, USB
- Input should be highest priority (lowest latency)
- Implement adaptive bitrate from the start
- Consider SVC temporal layers in the encoder for graceful degradation

### Sources
- [Media Over QUIC IETF Working Group](https://datatracker.ietf.org/group/moq/about/)
- [Cloudflare MoQ Blog](https://blog.cloudflare.com/moq/)
- [Streaming Remote Rendering: QUIC vs WebRTC](https://arxiv.org/html/2505.22132v1)
- [MOQ Protocol Explained - WebRTC.ventures](https://webrtc.ventures/2025/10/moq-protocol-explained-unifying-real-time-and-scalable-streaming/)
- [MoQ - nanocosmos](https://www.nanocosmos.net/blog/media-over-quic-moq/)
- [QUIC Fix for Video Streaming](https://arxiv.org/pdf/1809.10270)

---

## 8. Framebuffer Capture Techniques

### DMA-BUF Export (Zero-Copy)

**DMA-BUF** is the Linux kernel subsystem for sharing buffers between devices (GPU, display, video encoder).

**How it works**:
1. GPU renders frame into a DMA-BUF object (fd-backed GPU memory)
2. The fd is passed to the consumer (encoder, another GPU, etc.)
3. No CPU copy occurs; the buffer stays in GPU memory

**For a Wayland compositor acting as a thin client server**:
```
[Wayland clients] --> [Compositor renders to GPU buffer]
                           |
                    [DMA-BUF export (fd)]
                           |
                    [VAAPI encoder imports fd]
                           |
                    [Encoded bitstream -> network]
```

**Key protocols**:
- `linux-dmabuf-v1`: Clients use this to submit GPU-rendered buffers to the compositor
- `ext-image-copy-capture-v1`: Captures compositor output as DMA-BUF
- DMA-BUF feedback (v4): Tells clients which GPU/format the compositor prefers

### GPU Readback (Fallback)

When DMA-BUF export is not possible:
1. Compositor renders to GPU texture
2. `glReadPixels()` or equivalent copies pixels to CPU memory
3. CPU memory is then compressed/encoded

This is **significantly slower** due to the GPU -> CPU copy and pipeline stall, but universally supported.

### Damage Tracking

**Damage tracking** identifies which regions of the screen changed between frames, avoiding retransmission of unchanged areas.

**Wayland's built-in damage tracking**:
- Each `wl_surface.commit()` includes damage rectangles via `wl_surface.damage()` or `wl_surface.damage_buffer()`
- The compositor knows exactly which surface regions changed

**Compositor-level damage**:
- The compositor tracks which regions of the output changed (due to surface damage, window moves, overlapping windows, etc.)
- `ext-image-copy-capture-v1` supports damage reporting: the compositor tells the capturer which regions changed since the last frame

**For encoding efficiency**:
- With H.264/H.265/AV1: damage regions inform the encoder which macroblocks to mark as changed
- With lossless compression: only changed regions need to be compressed and sent
- With hybrid approach: unchanged regions get zero bits, changed regions get full encoding

### wl-screenrec: Reference Implementation

[wl-screenrec](https://github.com/russelltg/wl-screenrec) is a Rust project demonstrating high-performance Wayland screen recording:
- Uses wlr-screencopy with DMA-BUF
- Hardware encoding via VAAPI
- Zero-copy pipeline (DMA-BUF -> VAAPI -> file)
- Written in Rust, good reference for our implementation

### Key Design Insights for WayRay

- **Own the compositor**: By building/extending a Wayland compositor, we have direct access to all rendering state, damage information, and DMA-BUF handles
- **DMA-BUF -> VAAPI is the critical path**: This zero-copy pipeline should be the primary encoding path
- **Damage tracking reduces encoding work**: Use Wayland's built-in damage tracking to minimize what gets encoded
- **Fallback to GPU readback** for unsupported hardware
- **wl-screenrec** is a good Rust reference for the capture -> encode pipeline

### Sources
- [Linux DMA-BUF Kernel Documentation](https://dri.freedesktop.org/docs/drm/driver-api/dma-buf.html)
- [Linux DMA-BUF Wayland Protocol](https://wayland-book.com/surfaces/dmabuf.html)
- [ext-image-copy-capture-v1](https://wayland.app/protocols/ext-image-copy-capture-v1)
- [wlr-export-dmabuf-unstable-v1](https://wayland.app/protocols/wlr-export-dmabuf-unstable-v1)
- [wl-screenrec GitHub](https://github.com/russelltg/wl-screenrec)
- [OBS Zero-Copy Capture](https://obsproject.com/forum/threads/experimental-zero-copy-screen-capture-on-linux.101262/)
- [GStreamer DMA-BUF Design](https://gstreamer.freedesktop.org/documentation/additional/design/dmabuf.html)

---

## 9. Audio Forwarding

### PipeWire Network Audio

PipeWire provides several mechanisms for network audio:

#### RTP Modules (Recommended)

**`module-rtp-sink`**: Creates a PipeWire sink that sends audio as RTP packets
- Supports raw PCM, Opus encoding
- Configurable latency via `sess.latency.msec` (default: 100ms for network)
- Uses SAP/mDNS for discovery

**`module-rtp-source`**: Creates a PipeWire source that receives RTP packets
- DLL-based clock recovery to handle network jitter
- Configurable ring buffer fill level

**`module-rtp-session`**: Combined send/receive with automatic discovery
- Uses Apple MIDI protocol for low-latency bidirectional MIDI
- Announced via Avahi/mDNS/Bonjour

#### Pulse Tunnel Module

**`module-pulse-tunnel`**: Tunnels audio to/from a remote PulseAudio/PipeWire-Pulse server
- Simpler setup, works over TCP
- Higher latency than RTP approach
- Good for compatibility with existing PulseAudio setups

### Low-Latency Audio Considerations

For remote desktop audio, the targets are:

| Parameter | Target |
|---|---|
| **Codec** | Opus (designed for low latency) |
| **Frame size** | 2.5ms - 10ms (Opus supports down to 2.5ms) |
| **Buffer/Quantum** | As low as 128 samples @ 48kHz (~2.67ms) |
| **Network jitter buffer** | 10-30ms |
| **Total one-way latency** | 15-50ms |

**Opus codec advantages**:
- Designed for both speech and music
- 2.5ms to 60ms frame sizes
- 6 kbps to 510 kbps bitrate range
- Built-in forward error correction (FEC)
- Packet loss concealment (PLC)

### Custom Audio Pipeline for Thin Client

For a purpose-built thin client, the audio pipeline should be:

```
[Server PipeWire] -> [Opus encode] -> [RTP/QUIC] -> [Opus decode] -> [Client audio output]
[Client microphone] -> [Opus encode] -> [RTP/QUIC] -> [Opus decode] -> [Server PipeWire]
```

Key considerations:
- **Clock synchronization**: Client and server audio clocks will drift. Need adaptive resampling or buffer management.
- **Jitter compensation**: Network jitter requires a playout buffer. Adaptive jitter buffer adjusts to network conditions.
- **Echo cancellation**: If microphone and speakers are on the same client device, need AEC.

### Key Design Insights for WayRay

- **Opus over QUIC** is the right approach for a custom thin client
- PipeWire's RTP module is a good starting point but we may want tighter integration
- Clock drift compensation is critical for long-running sessions
- Audio and video synchronization (lip sync) must be maintained
- Forward error correction helps with packet loss without retransmission latency

### Sources
- [PipeWire RTP Session Module](https://docs.pipewire.org/page_module_rtp_session.html)
- [PipeWire RTP Sink](https://docs.pipewire.org/page_module_rtp_sink.html)
- [PipeWire RTP Source](https://docs.pipewire.org/page_module_rtp_source.html)
- [PipeWire Pulse Tunnel](https://docs.pipewire.org/page_module_pulse_tunnel.html)
- [PipeWire/PulseAudio RTP Network Audio Guide (Oct 2025)](https://liotier.medium.com/pipewire-pulseaudio-rtp-network-audio-in-october-2025-a-configuration-guide-to-the-remote-time-e8dc0e20e3b0)
- [PipeWire ArchWiki](https://wiki.archlinux.org/title/PipeWire)
- [PulseAudio Network Setup](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/User/Network/)

---

## 10. USB/IP

### Architecture

USB/IP is a Linux kernel subsystem that shares USB devices over TCP/IP networks.

**Components**:

| Component | Side | Role |
|---|---|---|
| **usbip-core** | Both | Shared protocol and utility code |
| **vhci-hcd** | Client | Virtual Host Controller Interface - presents virtual USB ports to the local USB stack |
| **usbip-host** (stub) | Server | Binds to physical USB devices, encapsulates URBs for network transmission |
| **usbip-vudc** | Server | Virtual USB Device Controller, for USB Gadget-based virtual devices |

### Protocol

**Discovery**: Client sends `OP_REQ_DEVLIST` over TCP, server responds with `OP_REP_DEVLIST` listing exportable devices.

**Attachment**: Client sends `OP_REQ_IMPORT`, server responds with `OP_REP_IMPORT` and begins forwarding URBs.

**Data transfer**: USB Request Blocks (URBs) are encapsulated in TCP packets and forwarded between stub driver and VHCI. The device driver runs entirely on the **client** side.

**Port**: TCP 3240 (default)

### Protocol Flow

```
[USB Device] <-> [Stub Driver (server kernel)]
                      |
                 [TCP/IP Network]
                      |
                [VHCI Driver (client kernel)]
                      |
             [USB Device Driver (client)]
                      |
             [Application (client)]
```

### Kernel Integration

- Merged into mainline Linux since **kernel 3.17**
- Source: `drivers/usb/usbip/` and `tools/usb/usbip/`
- Supports USB 2.0 and USB 3.0 devices
- Windows support via [usbip-win](https://github.com/cezanne/usbip-win)

### Limitations

- **Latency**: TCP round-trip for every URB can add significant latency for isochronous devices (audio, video)
- **Bandwidth**: USB 3.0 bulk transfers work well, but sustained high-bandwidth is limited by network
- **Isochronous transfers**: Not well supported (real-time USB audio/video devices may not work)
- **Security**: No built-in encryption (must tunnel through SSH/VPN)

### Alternatives: SPICE usbredir

SPICE's USB redirection (`usbredir`) is an alternative approach:
- Library: `libusbredir`
- Works at the USB protocol level (like USB/IP)
- Better integration with SPICE's authentication/encryption
- Can be used independently of SPICE

### Key Design Insights for WayRay

- **USB/IP is mature and kernel-integrated** - good baseline
- For a thin client, wrapping USB/IP over QUIC (instead of raw TCP) would add encryption and better congestion handling
- **usbredir** is worth considering as it's designed for remote desktop use cases
- Isochronous USB devices (webcams, audio interfaces) are challenging over network and may need special handling
- Consider selective USB forwarding - only forward devices the user explicitly shares

### Sources
- [USB/IP Kernel Documentation](https://docs.kernel.org/usb/usbip_protocol.html)
- [USB/IP ArchWiki](https://wiki.archlinux.org/title/USB/IP)
- [USB/IP Project](https://usbip.sourceforge.net/)
- [Linux Kernel USB/IP Source](https://github.com/torvalds/linux/tree/master/tools/usb/usbip)
- [USB/IP Tutorial - Linux Magazine](https://www.linux-magazine.com/Issues/2018/208/Tutorial-USB-IP)
- [usbip-win (Windows Support)](https://github.com/cezanne/usbip-win)
- [VirtualHere (Commercial Alternative)](https://www.virtualhere.com/)

---

## 11. Modern Thin Client Projects

### Sun Ray (Historical Reference)

The original Sun Ray (1999-2014) is the gold standard for thin client architecture:

- **Protocol**: Appliance Link Protocol (ALP) over UDP/IP
- **Architecture**: Completely stateless DTUs (Desktop Terminal Units) with zero local storage/OS
- **Session model**: Sessions are independent of physical hardware. Pull your smartcard, insert at another Sun Ray, session follows instantly ("hot desking")
- **Server**: Sun Ray Server Software (SRSS) managed sessions, ran on Solaris/Linux
- **Network**: Standard switched Ethernet, DHCP-based configuration
- **Security**: SSL/TLS encryption with 128-bit ARCFOUR
- **Display**: Rendered entirely on server, compressed framebuffer sent to DTU

**Key Sun Ray concepts to replicate**:
- Instant session mobility (smartcard/badge driven)
- Zero client-side state
- Centralized session management
- Simple, robust network boot

### Wafer (Wayland-Based Thin Client)

[Wafer](https://github.com/lp-programming/Wafer) is the most directly comparable modern project:

- **Goal**: Thin client for Linux server + Linux clients over high-speed LAN
- **Protocol**: Wayland protocol over network
- **Server** ("Mainframe"): Multi-core machine with GBM-capable GPU
- **Design**: Full 3D acceleration on server, minimal CPU on client (Raspberry Pi target)
- **Status**: Proof of concept / early development

### Sunshine + Moonlight

[Sunshine](https://github.com/LizardByte/Sunshine) (server) + [Moonlight](https://moonlight-stream.org/) (client) is the most mature open-source streaming solution:

- **Protocol**: Based on NVIDIA GameStream protocol
- **Encoding**: H.264, H.265, AV1 with NVENC, VAAPI, AMF hardware encoding
- **Performance**: Sub-10ms latency on LAN, up to 120 FPS
- **Clients**: Android, iOS, PC, Mac, Raspberry Pi, Steam Deck, Nintendo Switch, LG webOS
- **Audio**: Full audio streaming with multi-channel support
- **Input**: Mouse, keyboard, gamepad, touchscreen
- **Limitations**: Designed for single-user gaming, not multi-user thin client

### WayVNC

[WayVNC](https://github.com/any1/wayvnc) is a VNC server for wlroots-based Wayland compositors:

- Implements RFB protocol over wlr-screencopy / ext-image-copy-capture
- Supports headless mode (no physical display)
- Authentication: PAM, TLS (VeNCrypt), RSA-AES
- Input: Virtual pointer and keyboard via Wayland protocols
- JSON-IPC for runtime control
- Good reference for Wayland compositor integration

### GNOME Remote Desktop

GNOME's built-in remote desktop solution:

- Speaks **RDP** (primary) and VNC
- Uses PipeWire for screen capture via Mutter's ScreenCast D-Bus API
- Supports headless multi-user sessions (GNOME 46+)
- Input forwarding via Mutter's RemoteDesktop D-Bus API
- Integrated with GDM for remote login
- Active development, improving rapidly

### ThinStation

[ThinStation](https://thinstation.github.io/thinstation/) is a framework for building thin client Linux images:

- Supports Citrix ICA, SPICE, NX, RDP, VMware Horizon
- Boots from network (PXE), USB, or compact flash
- Not a protocol itself, but a client OS/distribution

### openthinclient

[openthinclient](https://openthinclient.com/) is a commercial open-source thin client management platform:

- Based on Debian (latest: Debian 13 "Trixie")
- Manages thin client fleet, user sessions, applications
- Supports multiple VDI protocols
- Version 2603 (2025) includes updated VDI components

### Key Design Insights for WayRay

- **Sunshine/Moonlight** proves that low-latency game streaming is solved; adapt for desktop
- **WayVNC** shows how to integrate with wlroots compositors
- **GNOME Remote Desktop** shows the PipeWire + portal approach
- **Wafer** validates the concept but is early-stage
- **Sun Ray's session mobility** is the killer feature to replicate
- No existing project combines: Wayland-native + multi-user + session mobility + hardware encoding + QUIC transport

### Sources
- [Sun Ray Wikipedia](https://en.wikipedia.org/wiki/Sun_Ray)
- [Sun Ray System Overview - Oracle](https://docs.oracle.com/cd/E19634-01/820-0411/overview.html)
- [Using Sun Ray Thin Clients in 2025](https://catstret.ch/202506/sun-ray-shenanigans/)
- [Wafer GitHub](https://github.com/lp-programming/Wafer)
- [Sunshine GitHub](https://github.com/LizardByte/Sunshine)
- [Moonlight](https://moonlight-stream.org/)
- [WayVNC GitHub](https://github.com/any1/wayvnc)
- [GNOME Remote Desktop Wiki](https://wiki.gnome.org/Projects/Mutter/RemoteDesktop)
- [ThinStation](https://thinstation.github.io/thinstation/)
- [openthinclient](https://openthinclient.com/)

---

## 12. Architecture Recommendations for a SunRay-Like System

Based on all the research above, here is a synthesized architectural recommendation:

### Core Architecture

```
                          ┌─────────────────────────────────────────────┐
                          │              WayRay Server                  │
                          │                                             │
                          │  ┌─────────────────────────────────┐       │
                          │  │   Wayland Compositor (wlroots)  │       │
                          │  │   - Per-user session             │       │
                          │  │   - DMA-BUF output              │       │
                          │  │   - Damage tracking              │       │
                          │  └──────────┬──────────────────────┘       │
                          │             │ DMA-BUF (zero-copy)           │
                          │  ┌──────────▼──────────────────────┐       │
                          │  │   Encoder Pipeline               │       │
                          │  │   - VAAPI H.264/H.265/AV1       │       │
                          │  │   - Damage-aware encoding        │       │
                          │  │   - Adaptive bitrate             │       │
                          │  └──────────┬──────────────────────┘       │
                          │             │ Encoded frames                │
                          │  ┌──────────▼──────────────────────┐       │
                          │  │   Session Manager                │       │
                          │  │   - Multi-user sessions          │       │
                          │  │   - Session migration            │       │
                          │  │   - Authentication               │       │
                          │  └──────────┬──────────────────────┘       │
                          │             │                               │
                          │  ┌──────────▼──────────────────────┐       │
                          │  │   QUIC Transport                 │       │
                          │  │   - Display stream (video)       │       │
                          │  │   - Input stream (low-latency)   │       │
                          │  │   - Audio stream (Opus/RTP)      │       │
                          │  │   - USB stream (usbredir)        │       │
                          │  │   - Control stream               │       │
                          │  └──────────┬──────────────────────┘       │
                          └─────────────┼───────────────────────────────┘
                                        │ QUIC / Network
                          ┌─────────────┼───────────────────────────────┐
                          │             │                               │
                          │  ┌──────────▼──────────────────────┐       │
                          │  │   QUIC Transport                 │       │
                          │  └──────────┬──────────────────────┘       │
                          │             │                               │
                          │  ┌──────────▼──────────────────────┐       │
                          │  │   Decoder (VAAPI/SW)             │       │
                          │  │   + Audio (Opus decode)          │       │
                          │  │   + Input capture                │       │
                          │  │   + USB forwarding               │       │
                          │  └──────────┬──────────────────────┘       │
                          │             │                               │
                          │  ┌──────────▼──────────────────────┐       │
                          │  │   Minimal Wayland Compositor     │       │
                          │  │   (or direct DRM/KMS output)     │       │
                          │  └─────────────────────────────────┘       │
                          │              WayRay Client                  │
                          └─────────────────────────────────────────────┘
```

### Technology Stack Recommendations

| Component | Recommended Technology | Rationale |
|---|---|---|
| **Server compositor** | wlroots-based custom compositor | Direct access to DMA-BUF, damage tracking, input injection |
| **Capture** | Direct compositor integration (no protocol needed) | Lowest latency, full damage info |
| **Encoding** | VAAPI (primary), NVENC (optional) via FFmpeg/GStreamer | Cross-vendor, zero-copy from DMA-BUF |
| **Video codec** | H.264 (default), AV1 (preferred when supported) | H.264 for compatibility, AV1 for quality/bandwidth |
| **Transport** | QUIC (quinn crate) with TCP fallback | Low latency, multiplexing, 0-RTT |
| **Audio** | Opus over QUIC stream | Low latency, built-in FEC |
| **USB** | usbredir over QUIC stream | Designed for remote desktop |
| **Session management** | Custom (inspired by Sun Ray SRSS) | Session mobility, multi-user |
| **Client display** | DRM/KMS direct or minimal Wayland compositor | Minimal overhead |
| **Language** | Rust | Safety, performance, excellent ecosystem (smithay, quinn, etc.) |

### QUIC Stream Layout

| Stream ID | Type | Priority | Reliability | Content |
|---|---|---|---|---|
| 0 | Bidirectional | Highest | Reliable | Control/session management |
| 1 | Server -> Client | High | Unreliable | Video frames |
| 2 | Client -> Server | Highest | Reliable | Input events |
| 3 | Server -> Client | Medium | Reliable | Audio playback |
| 4 | Client -> Server | Medium | Reliable | Audio capture |
| 5 | Bidirectional | Low | Reliable | USB/IP data |
| 6 | Bidirectional | Medium | Reliable | Clipboard |

### Encoding Strategy

1. **Damage detection**: Compositor reports damaged regions per frame
2. **Content classification**: Heuristically detect video regions vs. desktop content (like SPICE does)
3. **Encoding decision**:
   - Small damage, text/UI: Lossless (zstd-compressed) tile updates
   - Large damage, desktop: H.264/AV1 with high quality, low bitrate
   - Video regions: H.264/AV1 with lower quality, higher frame rate
   - Full screen video: Full-frame H.264/AV1 encoding
4. **Adaptive quality**: Adjust based on measured bandwidth and latency

### Sun Ray Features to Implement

1. **Session mobility**: Associate sessions with authentication tokens, not hardware. Insert token at any client -> session follows.
2. **Stateless clients**: Client boots from network, has no persistent state.
3. **Centralized management**: Server manages all sessions, client configurations, authentication.
4. **Hot desking**: Disconnect from one client, connect at another, session is exactly where you left it.
5. **Multi-monitor**: Support multiple displays per session.

# ADR-002: Network Transport Protocol

## Status
Accepted

## Context
WayRay must transmit display frames, input events, audio, and USB data between server and client with low latency. The transport must handle multiple independent data streams with different priority and reliability requirements.

## Options Considered

### 1. TCP (multiple connections)
- Reliable, ordered delivery
- Head-of-line blocking: a lost display frame packet blocks all subsequent frames
- No built-in multiplexing; need multiple connections or application-layer framing
- Well-understood, simple implementation

### 2. UDP (custom protocol)
- No head-of-line blocking
- Must implement reliability, ordering, and congestion control ourselves
- Maximum flexibility but significant implementation effort
- What SunRay's ALP did (UDP for display/input, TCP for control)

### 3. QUIC (via quinn)
- Built on UDP with TLS 1.3 encryption mandatory
- Independent streams: loss on stream 1 doesn't block stream 2
- 0-RTT connection resumption for fast session reconnection
- Built-in congestion control and flow control
- Multiplexing eliminates need for multiple connections
- Connection migration: survives IP address changes (WiFi -> Ethernet)
- quinn crate is mature and well-maintained in Rust

### 4. WebRTC
- Designed for peer-to-peer media streaming
- Overkill for our client-server architecture
- Complex negotiation (ICE, STUN, TURN)
- Good codec support but unnecessary complexity

## Decision
**Use QUIC via the quinn crate** as the primary transport protocol.

## Stream Architecture

| Stream ID | Channel | Reliability | Priority | Direction |
|-----------|---------|------------|----------|-----------|
| 0 | Control | Reliable | High | Bidirectional |
| 1 | Display | Unreliable* | High | Server -> Client |
| 2 | Input | Reliable | Highest | Client -> Server |
| 3 | Audio | Unreliable* | Medium | Bidirectional |
| 4+ | USB | Reliable | Low | Bidirectional |

*Unreliable streams use QUIC datagrams or short-lived unidirectional streams where old frames are discarded rather than retransmitted.

## Rationale
- Stream multiplexing maps naturally to our independent channels (display, input, audio, USB)
- No head-of-line blocking between channels: a lost display frame doesn't delay input
- 0-RTT reconnection enables sub-500ms session resumption for hot-desking
- Connection migration supports client network changes without session interruption
- TLS 1.3 mandatory means encryption is built-in, not an afterthought
- quinn is production-quality Rust QUIC implementation
- Avoids reinventing reliability/congestion control on raw UDP

## Consequences
- Requires TLS certificate management (self-signed for development, proper PKI for production)
- QUIC is newer than TCP; some corporate firewalls may block UDP
  - Mitigation: fallback to TCP with WebSocket tunneling as future work
- quinn is async (tokio), while Smithay uses calloop; need bridging between the two event loops

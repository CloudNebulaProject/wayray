# Network Transport

WayRay uses QUIC as its network transport protocol, providing encrypted, multiplexed, low-latency communication between server and client.

## Why QUIC?

### Stream Multiplexing
WayRay transmits multiple types of data simultaneously: display frames, input events, audio, and USB data. With TCP, a lost packet on the display stream would block input delivery (head-of-line blocking). QUIC's independent streams eliminate this -- each data type flows independently.

### Built-in Encryption
QUIC mandates TLS 1.3. Every WayRay connection is encrypted by default, with no option to disable it. This is security by design, not by configuration.

### 0-RTT Reconnection
When a client reconnects to a server it has connected to before, QUIC can resume the connection in zero round trips. This is critical for session mobility -- when you move your token to a new terminal, the reconnection must be fast.

### Connection Migration
If a client's IP address changes (e.g., switching from WiFi to Ethernet), QUIC can migrate the connection without dropping it. The session continues seamlessly.

## Stream Layout

| Stream | Purpose | Direction | Reliability |
|--------|---------|-----------|------------|
| 0 | Control | Bidirectional | Reliable |
| 1 | Display | Server -> Client | Semi-reliable* |
| 2 | Input | Client -> Server | Reliable |
| 3 | Audio | Bidirectional | Semi-reliable* |
| 4+ | USB | Bidirectional | Reliable |

*Semi-reliable: uses short-lived streams or datagrams so that old data is dropped rather than blocking new data.

## Priority

Input has the highest priority. A keystroke must reach the server even if a large display frame is in transit. QUIC's flow control and WayRay's priority scheme ensure input events are never delayed by display data.

## Firewall Considerations

QUIC runs over UDP. Some corporate firewalls block non-standard UDP traffic. If you encounter connectivity issues:

1. Ensure UDP port 4433 (default) is open between client and server
2. If UDP is blocked entirely, a future version of WayRay will support TCP fallback via WebSocket tunneling

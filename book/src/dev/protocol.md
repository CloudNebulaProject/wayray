# Protocol Reference

For the complete wire protocol specification, see [docs/protocols/wayray-wire-protocol.md](../../../docs/protocols/wayray-wire-protocol.md) in the repository.

## Quick Reference

### Message Flow: Connection

```
Client                          Server
  │                               │
  │──── QUIC Connect ────────────►│
  │                               │
  │──── ClientHello ─────────────►│
  │     (token, capabilities)     │
  │                               │
  │◄──── ServerHello ─────────────│
  │      (session, keymap)        │
  │                               │
  │◄──── FrameUpdate ────────────│  (display stream starts)
  │──── InputEvent ──────────────►│  (input stream starts)
  │◄───► AudioFrame ─────────────►│  (audio stream starts)
```

### Message Flow: Hot-Desking

```
Client A                Server              Client B
  │                       │                    │
  │ (connected, active)   │                    │
  │                       │                    │
  │── Token Remove ──────►│                    │
  │                       │  Session suspends  │
  │   (disconnected)      │                    │
  │                       │                    │
  │                       │◄── ClientHello ────│
  │                       │    (same token)    │
  │                       │                    │
  │                       │── ServerHello ────►│
  │                       │   (Resumed)        │
  │                       │                    │
  │                       │── FrameUpdate ───►│
```

### Encoding Types

| Type | Use Case | Quality | Bandwidth |
|------|----------|---------|-----------|
| `Zstd` | Text, UI, small changes | Lossless | Low |
| `Jpeg` | Photos, images | Lossy | Medium |
| `H264` | Video, animations | Lossy | Medium-High |
| `Av1` | Video (better compression) | Lossy | Low-Medium |
| `Raw` | Fallback, tiny regions | Lossless | High |

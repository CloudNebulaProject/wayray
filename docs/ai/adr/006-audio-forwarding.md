# ADR-006: Audio Forwarding

## Status
Accepted

## Context
WayRay must forward audio bidirectionally between server and client. Server-side applications produce audio output; client-side microphones capture audio input.

## Decision
**PipeWire for server-side audio capture/playback, Opus codec for transport, QUIC stream for delivery.**

## Design

### Server Side
- Create a virtual PipeWire sink per session (applications route audio here)
- Create a virtual PipeWire source per session (for microphone input from client)
- Capture audio frames from the virtual sink
- Encode to Opus (48kHz, stereo, 20ms frames for balance of latency and efficiency)
- Transmit over dedicated QUIC audio stream

### Client Side
- Receive Opus frames from QUIC audio stream
- Decode with opus crate
- Output to local audio device (via cpal or PipeWire client)
- Capture local microphone input
- Encode to Opus, send back over QUIC audio stream

### Synchronization
- Audio frames carry timestamps aligned with display frame sequence numbers
- Client performs adaptive jitter buffering (target: 20-60ms buffer)
- Lip-sync: audio presented when corresponding display frame is shown

## Rationale
- Opus is the gold standard for low-latency audio: 2.5ms minimum frame size, built-in FEC
- PipeWire is the modern Linux audio stack, replacing PulseAudio
- Dedicated QUIC stream prevents audio stuttering from display traffic
- Per-session virtual devices provide proper isolation

## Consequences
- PipeWire dependency on server (standard on modern Linux)
- Opus adds ~5ms encoding latency + network RTT + ~5ms decoding
- Must handle PipeWire session lifecycle (create/destroy with WayRay sessions)

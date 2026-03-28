# WayRay - SunRay-like Thin Client Wayland Compositor

## Project Overview

WayRay is a modern reimplementation of Oracle/Sun's SunRay thin client architecture as a Wayland compositor written in Rust. The goal is to provide stateless thin client computing with session mobility, hot-desking, and zero-state endpoints over a modern protocol stack.

## Target Platforms

- **illumos** (OpenIndiana, OmniOS, SmartOS) - Primary target, honoring SunRay's Solaris heritage
- **Linux** - Full support with optional GPU acceleration
- Architecture is **headless-first**: Smithay with `default-features = false`, no dependency on DRM/KMS, libinput, udev, or libseat in the core path. Platform-specific backends behind feature flags.

## Language & Stack

- **Language**: Rust (edition 2024)
- **Compositor Framework**: Smithay (Wayland compositor library)
- **Event Loop**: calloop (callback-based, not async)
- **Networking**: QUIC via quinn for low-latency multiplexed transport
- **Encoding**: H.264/AV1 via VAAPI/NVENC for display, Opus for audio
- **Error Reporting**: miette with diagnostic patterns for user-facing errors
- **ORM**: SeaORM for any database needs (never raw SQL)
- **Logging**: tracing crate

## Architecture

WayRay consists of four main components:

1. **wayray-server** (compositor) - Smithay-based Wayland compositor that runs applications, captures framebuffers, and transmits them to clients. Exposes a pluggable WM protocol.
2. **wayray-client** (viewer) - Lightweight display client that decodes frames, renders them locally, captures input, and sends it to the server
3. **wayray-protocol** - Shared protocol definitions for the WayRay wire protocol over QUIC
4. **wayray-ctl** - CLI management tool for server/session administration

## Documentation

- **docs/ai/plans/** - Implementation plans
- **docs/ai/adr/** - Architecture Decision Records
- **docs/architecture/** - System architecture documentation
- **docs/protocols/** - Protocol specifications
- **book/** - mdbook user guide (build with `mdbook build book/`)

## Key Design Decisions

- **Headless-first**: No hard dependency on GPU, DRM, or Linux-specific subsystems
- **illumos + Linux**: Portable core, platform-specific backends behind feature flags
- **Pluggable Window Management**: External WM process via custom Wayland protocol (River-inspired two-phase transaction model). Ships with built-in floating WM as default.
- QUIC over TCP for network transport (multiplexing, 0-RTT reconnect)
- PixmanRenderer for headless server (no GPU needed), GlesRenderer when GPU available
- Damage-tracked differential frame encoding
- Smart card / token-based session mobility (hot-desking)
- Userspace USB forwarding over QUIC (not kernel USB/IP, for portability)
- Audio: PipeWire (Linux) or PulseAudio (illumos/Linux) behind trait abstraction
- `shm_open` fallback when `memfd_create` unavailable (illumos portability)

## Conventions

- Use miette's diagnostic pattern for all user-facing errors with helpful messages
- Never use raw SQL - use SeaORM entities
- Docker builds must use current Rust version with target-platform-specific caches
- Clean up unused variables but investigate if design changes caused them to become unused
- Use context7 for library documentation lookups

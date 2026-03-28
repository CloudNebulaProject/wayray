# ADR-007: Project Structure (Cargo Workspace)

## Status
Accepted

## Context
WayRay consists of multiple binaries and shared libraries. We need a project structure that supports clean separation of concerns while sharing common code.

## Decision
**Cargo workspace with the following crate layout:**

```
wayray/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── wayray-server/      # Wayland compositor + frame encoder + network server
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── compositor/  # Smithay compositor implementation
│   │       ├── encoder/     # Frame encoding pipeline
│   │       ├── network/     # QUIC server, session management
│   │       └── audio/       # PipeWire integration
│   │
│   ├── wayray-client/      # Remote viewer + input capture + decoder
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── decoder/     # Frame decoding pipeline
│   │       ├── renderer/    # Local display (wgpu/winit)
│   │       ├── network/     # QUIC client
│   │       ├── input/       # Input capture and forwarding
│   │       └── audio/       # Local audio playback/capture
│   │
│   ├── wayray-protocol/    # Shared protocol definitions
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── messages.rs  # Wire protocol message types
│   │       ├── codec.rs     # Serialization/deserialization
│   │       └── version.rs   # Protocol versioning
│   │
│   └── wayray-ctl/         # CLI management tool
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
│
├── docs/                   # Documentation
├── book/                   # mdbook user guide
└── tests/                  # Integration tests
```

## Rationale
- `crates/` directory keeps workspace root clean
- Protocol crate ensures server and client agree on wire format at compile time
- Server and client have completely different dependency trees (smithay vs wgpu)
- `wayray-ctl` as separate binary avoids bloating server/client with admin dependencies
- Integration tests at workspace root can spin up server + client together

## Consequences
- Must manage feature flags carefully across workspace
- Protocol changes require updating both server and client
- CI must build and test all crates

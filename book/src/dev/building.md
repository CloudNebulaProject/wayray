# Building from Source

## Prerequisites

Ensure you have:
- Rust toolchain (latest stable, install via [rustup](https://rustup.rs))
- System development libraries (see [Installation](../installation.md))

## Build Commands

```bash
# Development build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release

# Build only the server
cargo build -p wayray-server

# Build only the client
cargo build -p wayray-client

# Run tests
cargo test --workspace

# Run with logging
RUST_LOG=wayray=debug cargo run -p wayray-server
```

## Feature Flags

### wayray-server

| Feature | Default | Description |
|---------|---------|-------------|
| `renderer-pixman` | yes | Software rendering (no GPU needed) |
| `renderer-gles` | no | Hardware OpenGL ES rendering |
| `xwayland` | no | X11 application support |
| `vaapi` | no | Hardware video encoding |

### wayray-client

| Feature | Default | Description |
|---------|---------|-------------|
| `smartcard` | no | PC/SC smart card support |
| `nfc` | no | NFC token support |

## Development Tips

### Running Nested (Recommended for Development)

During development, run the WayRay server nested inside your existing desktop using the Winit backend:

```bash
cargo run -p wayray-server -- --backend winit
```

This opens a window on your desktop that acts as the WayRay display. No need for a separate TTY.

### Running the Client Against a Local Server

```bash
# Terminal 1: Start server
cargo run -p wayray-server -- --backend winit --listen 127.0.0.1:4433

# Terminal 2: Start client
cargo run -p wayray-client -- --server 127.0.0.1:4433 --token dev
```

# ADR-001: Compositor Framework Selection

## Status
Accepted

## Context
We need a Wayland compositor library to build WayRay's server-side compositor. The compositor must support headless operation (no physical display required), framebuffer capture for network transmission, and input injection from remote clients.

## Options Considered

### 1. Smithay (Rust)
- Purpose-built Rust library for Wayland compositors
- Provides backend abstractions (DRM, libinput, EGL), Wayland protocol helpers, and rendering infrastructure
- Used by production compositors: cosmic-comp (System76), niri
- Active development, strong community
- Supports PixmanRenderer (software, headless) and GlesRenderer (hardware)
- ExportMem trait allows framebuffer readback

### 2. wlroots (C)
- Mature C library used by Sway, Hyprland, and many others
- Would require FFI bindings, losing Rust safety guarantees
- Opinionated about rendering pipeline; harder to customize for remote display
- More battle-tested but less idiomatic for a Rust project

### 3. From scratch
- Maximum flexibility but enormous implementation effort
- Would need to implement Wayland protocol, DRM/KMS, input handling manually
- Not justified when Smithay provides these building blocks

## Decision
**Use Smithay** as the compositor framework.

## Rationale
- Native Rust with full type safety and ownership semantics
- Modular architecture: we use only what we need
- PixmanRenderer enables headless server operation without GPU
- ExportMem trait provides the framebuffer capture path we need
- OutputDamageTracker gives us efficient dirty region tracking
- calloop event loop integrates cleanly with our networking layer
- Production-proven by cosmic-comp and niri
- Active maintenance and responsive community

## Consequences
- We inherit Smithay's API design and must follow its handler trait pattern
- We depend on Smithay's release cycle for protocol support
- calloop (not tokio) is our event loop, which affects how we integrate async networking

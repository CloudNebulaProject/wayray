# Phase 0.5: VM Testing Setup — Design Spec

## Goal

Create a reproducible Arch Linux aarch64 VM setup for visually testing the WayRay compositor on a MacBook Air M4 via UTM. This lets the developer see wrsrvd rendering Wayland clients in a real graphical session.

## Context

Phase 0 built a working Smithay compositor (`wrsrvd`) that uses the Winit backend. Winit opens a window inside an existing display server (X11 or Wayland). The development host is a headless x86 Linux system with no display — visual testing requires a separate machine with a GPU and display.

The developer's local machine is an Apple Silicon MacBook Air (M4). UTM provides native aarch64 Linux VMs on macOS using Apple's Virtualization.framework.

## Architecture

```
MacBook Air M4 (macOS)
  └── UTM VM (Arch Linux aarch64)
        └── Sway (minimal Wayland compositor / session host)
              └── wrsrvd window (WayRay compositor via Winit backend)
                    └── foot terminal (Wayland client inside wrsrvd)
```

Sway serves only as the Wayland session host so Winit has a display to open a window in. The wrsrvd window is where the actual compositor output appears. Wayland clients (like foot) connect to wrsrvd's socket and render inside its window.

## Deliverables

### 1. `vm/README.md` — UTM Setup Guide

Short instructions covering:

1. **Download:** Arch Linux ARM ISO from archlinux.org
2. **Create VM in UTM:**
   - Type: Virtualize (Apple Virtualization.framework)
   - CPU: 4 cores
   - RAM: 8 GB
   - Disk: 30 GB
   - Display: VirtIO GPU
   - Network: Shared/NAT
3. **Boot and install Arch:** Use `archinstall` for a minimal install (no desktop environment)
4. **Run setup script:** `bash vm/setup.sh`
5. **Reboot:** VM auto-logs in, starts Sway
6. **Test:** How to launch wrsrvd and connect a client

### 2. `vm/setup.sh` — Provisioning Script

Idempotent script run as a regular user (uses `sudo` where needed). Steps:

1. **System update**
   ```
   sudo pacman -Syu --noconfirm
   ```

2. **Install packages**
   - Build tools: `base-devel`
   - Wayland stack: `wayland`, `wayland-protocols`, `libxkbcommon`
   - Graphics: `mesa` (EGL/OpenGL for GlesRenderer)
   - Session: `sway`, `seatd`, `foot`
   - Tools: `git`, `openssh`

3. **Enable services**
   - `sudo systemctl enable --now seatd`
   - Add user to `seat` group

4. **Install Rust**
   - `rustup` via official installer
   - Stable toolchain (must support edition 2024 — Rust 1.85+)

5. **Clone and build WayRay**
   - `git clone <repo-url>` into `~/wayray` (URL passed as argument or prompted)
   - `cargo build --workspace`

6. **Configure auto-login on TTY1**
   - systemd getty override for automatic login
   - `.bash_profile` starts Sway on TTY1

7. **Write Sway config**
   ```
   # ~/.config/sway/config
   set $mod Mod4
   output * bg #333333 solid_color
   default_border none
   bindsym $mod+Return exec foot
   exec foot
   ```

Each step checks whether it's already complete before running (idempotent).

### 3. No additional project code changes

The existing wrsrvd binary works as-is in this setup. No code modifications needed.

## VM Specs

| Setting | Value | Rationale |
|---------|-------|-----------|
| Type | Virtualize | Native aarch64 via Apple Virtualization.framework |
| CPU | 4 cores | Rust compilation parallelism |
| RAM | 8 GB | Rust compiler is memory-hungry |
| Disk | 30 GB | Arch + Rust toolchain + cargo cache + Mesa |
| Display | VirtIO GPU | Required by Sway for GPU context (Mesa virgl driver). If Virtualize mode fails, fall back to UTM's QEMU backend. |
| Network | Shared/NAT | Pacman, git clone, rustup |

## Test Flow

1. Boot VM — auto-login to TTY1 — Sway starts automatically
2. foot terminal opens (autostarted by Sway config)
3. Run: `cd ~/wayray && cargo run --bin wrsrvd`
4. wrsrvd window opens inside Sway
5. Note the socket name from the log output
6. In the foot terminal: `WAYLAND_DISPLAY=<socket> foot`
7. A second foot terminal appears inside the wrsrvd window
8. Verify: typing works, clicking works, rendering is correct

## Success Criteria

- VM boots to a Sway session without manual intervention
- `cargo build --workspace` succeeds inside the VM
- wrsrvd opens a Winit window inside Sway
- A Wayland client (foot) renders inside the wrsrvd window
- Input (keyboard, mouse) reaches the client through wrsrvd

## Out of Scope

- Automated screenshot testing or CI integration
- x86 VM support (future phase)
- Headless/remote testing (Phase 1+)
- GPU passthrough or hardware acceleration
- illumos VM testing

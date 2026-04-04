# Phase 0.5: VM Testing Setup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a reproducible Arch Linux aarch64 VM setup for visually testing the WayRay compositor on a MacBook Air M4 via UTM.

**Architecture:** Two files in `vm/` — a README with UTM setup instructions and a provisioning script that installs all dependencies, builds WayRay, and configures auto-login into a minimal Sway session. The developer follows the README once, runs the script, and gets a bootable graphical environment for testing.

**Tech Stack:** Bash, pacman, rustup, Sway, UTM

**Spec:** `docs/ai/specs/2026-04-04-phase05-vm-testing-design.md`

---

## File Structure

```
vm/
├── README.md      # UTM VM creation guide + test instructions
└── setup.sh       # Idempotent provisioning script
```

---

## Task 1: Create the provisioning script

**Files:**
- Create: `vm/setup.sh`

- [ ] **Step 1: Create the vm/ directory and setup.sh**

`vm/setup.sh`:
```bash
#!/usr/bin/env bash
# WayRay VM Provisioning Script
# Run as a regular user inside a fresh Arch Linux aarch64 install.
# Idempotent — safe to run multiple times.

set -euo pipefail

REPO_URL="${1:-}"
WAYRAY_DIR="$HOME/wayray"

info() { printf '\033[1;34m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m==> WARNING: %s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m==> %s\033[0m\n' "$*"; }

# ── 1. System packages ───────────────────────────────────────────────

info "Updating system packages..."
sudo pacman -Syu --noconfirm

PACKAGES=(
    # Build tools
    base-devel

    # Wayland
    wayland
    wayland-protocols
    libxkbcommon

    # Graphics (Mesa provides EGL/OpenGL for Winit's GlesRenderer)
    mesa

    # Session
    sway
    seatd
    foot

    # Tools
    git
    openssh
)

info "Installing packages..."
sudo pacman -S --needed --noconfirm "${PACKAGES[@]}"

# ── 2. Services ──────────────────────────────────────────────────────

info "Enabling seatd..."
sudo systemctl enable --now seatd

# Add user to seat group if not already a member.
if ! groups | grep -q '\bseat\b'; then
    info "Adding $USER to seat group..."
    sudo usermod -aG seat "$USER"
    warn "Group change requires re-login. Re-run this script after reboot."
fi

# ── 3. Rust toolchain ────────────────────────────────────────────────

if command -v cargo &>/dev/null; then
    ok "Rust toolchain already installed ($(rustc --version))"
else
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi

# Verify edition 2024 support (Rust 1.85+).
RUST_VERSION=$(rustc --version | grep -oP '\d+\.\d+')
RUST_MAJOR=$(echo "$RUST_VERSION" | cut -d. -f1)
RUST_MINOR=$(echo "$RUST_VERSION" | cut -d. -f2)
if (( RUST_MAJOR < 1 || (RUST_MAJOR == 1 && RUST_MINOR < 85) )); then
    warn "Rust $RUST_VERSION may not support edition 2024. Run: rustup update"
fi

# ── 4. Clone and build WayRay ────────────────────────────────────────

if [ -d "$WAYRAY_DIR" ]; then
    ok "WayRay repo already cloned at $WAYRAY_DIR"
else
    if [ -z "$REPO_URL" ]; then
        echo ""
        echo "WayRay repo URL not provided."
        echo "Usage: $0 <git-repo-url>"
        echo "Example: $0 https://github.com/user/wayray.git"
        echo "         $0 git@github.com:user/wayray.git"
        exit 1
    fi
    info "Cloning WayRay..."
    git clone "$REPO_URL" "$WAYRAY_DIR"
fi

info "Building WayRay..."
cd "$WAYRAY_DIR"
cargo build --workspace
ok "Build complete"

# ── 5. Auto-login on TTY1 ───────────────────────────────────────────

GETTY_OVERRIDE="/etc/systemd/system/getty@tty1.service.d/override.conf"
if [ -f "$GETTY_OVERRIDE" ]; then
    ok "Auto-login already configured"
else
    info "Configuring auto-login on TTY1..."
    sudo mkdir -p "$(dirname "$GETTY_OVERRIDE")"
    sudo tee "$GETTY_OVERRIDE" > /dev/null <<EOF
[Service]
ExecStart=
ExecStart=-/usr/bin/agetty --autologin $USER --noclear %I \$TERM
EOF
    sudo systemctl daemon-reload
fi

# Start Sway on TTY1 login (only if not already in a graphical session).
PROFILE="$HOME/.bash_profile"
SWAY_LAUNCH='[ "$(tty)" = "/dev/tty1" ] && exec sway'
if ! grep -qF 'exec sway' "$PROFILE" 2>/dev/null; then
    info "Adding Sway auto-start to $PROFILE..."
    echo "" >> "$PROFILE"
    echo "# Start Sway on TTY1" >> "$PROFILE"
    echo "$SWAY_LAUNCH" >> "$PROFILE"
fi

# ── 6. Sway config ──────────────────────────────────────────────────

SWAY_CONFIG="$HOME/.config/sway/config"
if [ -f "$SWAY_CONFIG" ]; then
    ok "Sway config already exists at $SWAY_CONFIG"
else
    info "Writing minimal Sway config..."
    mkdir -p "$(dirname "$SWAY_CONFIG")"
    cat > "$SWAY_CONFIG" <<'EOF'
# Minimal Sway config for WayRay development/testing.
# This exists solely as a Wayland session host for the Winit backend.

set $mod Mod4
output * bg #333333 solid_color
default_border none

# Launch a terminal
bindsym $mod+Return exec foot

# Exit Sway
bindsym $mod+Shift+e exit

# Autostart a terminal on login
exec foot
EOF
fi

# ── Done ─────────────────────────────────────────────────────────────

echo ""
ok "Setup complete!"
echo ""
echo "Next steps:"
echo "  1. Reboot: sudo reboot"
echo "  2. VM will auto-login and start Sway"
echo "  3. In the foot terminal:"
echo "       cd ~/wayray && cargo run --bin wrsrvd"
echo "  4. Note the socket name from the log, then:"
echo "       WAYLAND_DISPLAY=<socket> foot"
echo "  5. A foot terminal should appear inside the wrsrvd window"
echo ""
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x vm/setup.sh
```

- [ ] **Step 3: Test the script parses correctly**

```bash
bash -n vm/setup.sh
```

Expected: No output (no syntax errors).

- [ ] **Step 4: Commit**

```bash
git add vm/setup.sh
git commit -m "Add VM provisioning script for Arch Linux aarch64 testing"
```

---

## Task 2: Create the VM setup README

**Files:**
- Create: `vm/README.md`

- [ ] **Step 1: Write the README**

`vm/README.md`:
```markdown
# WayRay VM Testing Setup

Visual testing environment for the WayRay compositor using an Arch Linux
aarch64 VM on macOS Apple Silicon via UTM.

## Prerequisites

- MacBook with Apple Silicon (M1/M2/M3/M4)
- [UTM](https://mac.getutm.app/) installed (free download from website)

## Create the VM

### 1. Download the Arch Linux ARM ISO

Go to <https://archlinux.org/download/> and download the latest aarch64
ISO image.

### 2. Create a new VM in UTM

1. Open UTM, click **Create a New Virtual Machine**
2. Select **Virtualize**
3. Select **Linux**
4. Browse to the downloaded ISO
5. Configure hardware:
   - **CPU:** 4 cores
   - **RAM:** 8192 MB (8 GB)
6. Configure storage:
   - **Disk size:** 30 GB
7. Give it a name (e.g., "WayRay Dev")
8. Click **Save**

The VM defaults to VirtIO display and shared networking, which is what
we need.

### 3. Install Arch Linux

1. Boot the VM from the ISO
2. Run `archinstall`
3. Recommended settings:
   - **Mirrors:** Select your region
   - **Disk configuration:** Use entire disk, ext4
   - **Bootloader:** systemd-boot
   - **Profile:** Minimal
   - **User:** Create a user with sudo privileges
   - **Network:** NetworkManager
4. Complete the installation and reboot
5. Remove the ISO from the VM's CD drive in UTM settings

### 4. Run the setup script

Boot into the installed system and log in, then:

```bash
# Install git first (needed to clone the repo)
sudo pacman -S git --noconfirm

# Clone WayRay and run the setup script
git clone <your-wayray-repo-url> ~/wayray
bash ~/wayray/vm/setup.sh <your-wayray-repo-url>

# Reboot to apply auto-login and start Sway
sudo reboot
```

## Testing the Compositor

After reboot, the VM auto-logs in and starts Sway. A foot terminal
opens automatically.

```bash
# Build and run the compositor
cd ~/wayray
cargo run --bin wrsrvd
```

The wrsrvd window opens inside Sway. Note the Wayland socket name from
the log output (e.g., `wayland-1`).

In the same foot terminal, or open another with `Super+Return`:

```bash
# Launch a client inside the compositor
WAYLAND_DISPLAY=wayland-1 foot
```

A foot terminal should appear inside the wrsrvd window. You can:
- Type in it (keyboard input works)
- Click on it (pointer input works)
- See it render (compositor rendering works)

## Troubleshooting

### Sway fails to start

If Sway fails with a GPU error, the VirtIO GPU may not be working
with Apple Virtualization.framework. Try switching UTM to QEMU backend:

1. Shut down the VM
2. In UTM, edit the VM settings
3. Under **System**, uncheck "Use Apple Virtualization"
4. Under **Display**, ensure VirtIO GPU is selected
5. Boot again

### Black screen after reboot

The auto-login or Sway autostart may have failed. Switch to TTY2 with
`Ctrl+Alt+F2`, log in, and check:

```bash
# Check if seatd is running
systemctl status seatd

# Check if user is in seat group
groups

# Try starting Sway manually
sway
```

### Build fails

```bash
# Make sure Rust is up to date
rustup update

# Check Rust version (need 1.85+ for edition 2024)
rustc --version

# Retry
cargo build --workspace
```
```

- [ ] **Step 2: Commit**

```bash
git add vm/README.md
git commit -m "Add VM setup README for UTM + Arch Linux testing"
```

---

## Notes for the Implementer

This plan creates two files — a shell script and a README. No Rust code is modified. The script is tested only for syntax (`bash -n`); full testing requires running it inside an actual Arch VM, which is a manual step the developer performs on their Mac.

**Before committing:** Run `cargo fmt --all && cargo clippy --workspace` to ensure no regressions (nothing should change since we're not touching Rust code, but verify).

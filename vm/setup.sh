#!/usr/bin/env bash
# WayRay VM Provisioning Script
# Run as a regular user inside an Ubuntu 22.04 aarch64 VM (UTM gallery image).
# Idempotent — safe to run multiple times.

set -euo pipefail

REPO_URL="${1:-https://github.com/CloudNebulaProject/wayray.git}"
WAYRAY_DIR="$HOME/wayray"

info() { printf '\033[1;34m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m==> WARNING: %s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m==> %s\033[0m\n' "$*"; }

# ── 1. System packages ───────────────────────────────────────────────

info "Updating system packages..."
sudo apt-get update
sudo apt-get upgrade -y

PACKAGES=(
    # Build tools
    build-essential
    pkg-config
    cmake

    # Wayland
    libwayland-dev
    wayland-protocols
    libxkbcommon-dev
    libxkbcommon-x11-0

    # Graphics (Mesa provides EGL/OpenGL for Winit's GlesRenderer)
    libgles2-mesa-dev
    libegl1-mesa-dev
    libgbm-dev
    libdrm-dev

    # X11 (fallback if Wayland EGL doesn't work under virgl)
    xorg
    xinit
    openbox
    xterm

    # Session (Wayland)
    sway
    foot
    seatd

    # Tools
    git
    curl
    openssh-client
)

info "Installing packages..."
sudo apt-get install -y "${PACKAGES[@]}"

# ── 2. Services ──────────────────────────────────────────────────────

info "Enabling seatd..."
sudo systemctl enable --now seatd || true

# Add user to seat group if not already a member.
if getent group seat &>/dev/null && ! groups | grep -q '\bseat\b'; then
    info "Adding $USER to seat group..."
    sudo usermod -aG seat "$USER"
    warn "Group change requires re-login. Re-run this script after reboot."
fi

# Some Ubuntu setups need the user in video group for GPU access.
if ! groups | grep -q '\bvideo\b'; then
    info "Adding $USER to video group..."
    sudo usermod -aG video "$USER"
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

# Make cargo available in this session if .cargo/env exists.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

# Verify edition 2024 support (Rust 1.85+).
RUST_VERSION=$(rustc --version | sed -n 's/rustc \([0-9]*\.[0-9]*\).*/\1/p')
RUST_MAJOR=$(echo "$RUST_VERSION" | cut -d. -f1)
RUST_MINOR=$(echo "$RUST_VERSION" | cut -d. -f2)
if [ "$RUST_MAJOR" -lt 1 ] || { [ "$RUST_MAJOR" -eq 1 ] && [ "$RUST_MINOR" -lt 85 ]; }; then
    warn "Rust $RUST_VERSION may not support edition 2024. Run: rustup update"
fi

# ── 4. Clone and build WayRay ────────────────────────────────────────

if [ -d "$WAYRAY_DIR" ]; then
    ok "WayRay repo already cloned at $WAYRAY_DIR"
else
    info "Cloning WayRay from $REPO_URL..."
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
ExecStart=-/sbin/agetty --autologin $USER --noclear %I \$TERM
EOF
    sudo systemctl daemon-reload
fi

# Start Sway on TTY1 login (only if not already in a graphical session).
PROFILE="$HOME/.profile"
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

# Close focused window
bindsym $mod+Shift+q kill

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

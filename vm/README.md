# WayRay VM Testing Setup

Visual testing environment for the WayRay compositor using an Ubuntu
22.04 aarch64 VM on macOS Apple Silicon via UTM.

## Prerequisites

- MacBook with Apple Silicon (M1/M2/M3/M4)
- [UTM](https://mac.getutm.app/) installed (free download from website)

## Create the VM

### 1. Get the Ubuntu 22.04 image from the UTM Gallery

1. Open UTM
2. Click **+** to create a new VM
3. Select **Browse UTM Gallery...**
4. Find and download **Ubuntu 22.04**
5. The VM is pre-configured with reasonable defaults

### 2. Adjust VM settings (optional)

The gallery image works out of the box, but for faster Rust compilation:

1. Select the VM in UTM, click the **settings icon**
2. Under **System**:
   - **CPU:** 4 cores (or more)
   - **RAM:** 8192 MB (8 GB)

### 3. Boot and run the setup script

1. Start the VM and log in (gallery image credentials are shown in UTM)
2. Open a terminal and run:

```bash
# Clone WayRay and run the setup script
git clone https://github.com/CloudNebulaProject/wayray.git ~/wayray
bash ~/wayray/vm/setup.sh

# Reboot to apply auto-login and start Sway
sudo reboot
```

The setup script installs all dependencies (Wayland libs, Mesa, Sway,
Rust), builds WayRay, and configures auto-login into a minimal Sway
session.

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

Open another terminal with `Super+Return`, then:

```bash
# Launch a client inside the compositor
WAYLAND_DISPLAY=wayland-1 foot
```

A foot terminal should appear inside the wrsrvd window. You can:
- Type in it (keyboard input works)
- Click on it (pointer input works)
- See it render (compositor rendering works)

## Keyboard Shortcuts (Sway)

| Shortcut | Action |
|----------|--------|
| `Super+Return` | Open a new terminal |
| `Super+Shift+Q` | Close focused window |
| `Super+Shift+E` | Exit Sway |

## Troubleshooting

### Sway fails to start

If Sway fails with a GPU or seat error:

```bash
# Check seatd is running
systemctl status seatd

# Check group membership (need seat and video)
groups

# If missing, add and reboot
sudo usermod -aG seat,video $USER
sudo reboot
```

If the VirtIO GPU isn't working, try switching UTM to QEMU backend:

1. Shut down the VM
2. Edit VM settings
3. Under **System**, uncheck "Use Apple Virtualization"
4. Boot again

### Black screen after reboot

Switch to TTY2 with `Ctrl+Alt+F2`, log in, and check:

```bash
# Try starting Sway manually
sway

# Check the auto-login override
cat /etc/systemd/system/getty@tty1.service.d/override.conf

# Check .profile for Sway launch
tail ~/.profile
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

### Missing Wayland/EGL libraries

If `cargo build` fails with missing `-lwayland-client` or EGL errors:

```bash
# Reinstall dev packages
sudo apt-get install -y libwayland-dev libegl1-mesa-dev libgles2-mesa-dev libxkbcommon-dev
```

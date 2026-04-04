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
bash ~/wayray/vm/setup.sh

# Reboot to apply auto-login and start Sway
sudo reboot
```

Note: the script skips cloning if `~/wayray` already exists, so passing
the repo URL to `setup.sh` is only needed if you haven't cloned yet.

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

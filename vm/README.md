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

The setup script installs all dependencies (Wayland libs, Mesa, X11,
Rust), builds WayRay, and configures auto-login.

## Testing the Compositor

The Winit backend can run under either X11 or Wayland. Under UTM,
**X11 is more reliable** — virgl's EGL surface handling has issues
with Sway/Wayland in virtualized environments.

### Option A: X11 (recommended for UTM)

Start an X session:

```bash
startx openbox-session
```

Then from a TTY (`Ctrl+Alt+F2`) or an xterm in the Openbox session:

```bash
export DISPLAY=:0
cd ~/wayray
cargo run --bin wrsrvd
```

The wrsrvd window opens inside X11. Note the Wayland socket name from
the log output (e.g., `wayland-2`). In another xterm:

```bash
WAYLAND_DISPLAY=wayland-2 xterm
```

An xterm should appear inside the wrsrvd window.

### Option B: Sway (if GPU supports it)

If you have a working Wayland-capable GPU (e.g., bare metal or a VM
with good virgl support):

```bash
sway
```

Then in the foot terminal:

```bash
cd ~/wayray
cargo run --bin wrsrvd
```

Open another terminal with `Super+Return`, then:

```bash
WAYLAND_DISPLAY=wayland-2 foot
```

### What to verify

- A client window appears inside the wrsrvd compositor window
- Typing works (keyboard input reaches the client)
- Clicking works (pointer input reaches the client)
- The window renders correctly (compositor rendering works)

## Keyboard Shortcuts

**Sway:**

| Shortcut | Action |
|----------|--------|
| `Super+Return` | Open a new terminal |
| `Super+Shift+Q` | Close focused window |
| `Super+Shift+E` | Exit Sway |

**Openbox:** Right-click the desktop for a menu.

## Troubleshooting

### EGL errors under Sway (BAD_SURFACE, BadAlloc)

This is a known issue with virgl + UTM. Use the X11 path instead
(Option A above).

### Build fails

```bash
# Make sure Rust is up to date
rustup update

# Check Rust version (need 1.85+ for edition 2024)
rustc --version

# Retry
cargo build --workspace
```

### Missing libraries

If `cargo build` or `cargo run` fails with missing libraries:

```bash
sudo apt-get install -y libwayland-dev libegl1-mesa-dev libgles2-mesa-dev \
    libxkbcommon-dev libxkbcommon-x11-0
```

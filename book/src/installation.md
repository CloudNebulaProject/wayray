# Installation

## Building from Source

### Prerequisites

**Server:**
- Rust (latest stable)
- Linux with Wayland support
- Development libraries:
  ```bash
  # Debian/Ubuntu
  sudo apt install libwayland-dev libinput-dev libudev-dev libgbm-dev \
    libxkbcommon-dev libpixman-1-dev libseat-dev libpipewire-0.3-dev \
    libssl-dev pkg-config cmake

  # Fedora
  sudo dnf install wayland-devel libinput-devel systemd-devel mesa-libgbm-devel \
    libxkbcommon-devel pixman-devel libseat-devel pipewire-devel \
    openssl-devel pkg-config cmake
  ```

**Client:**
- Rust (latest stable)
- GPU drivers (Vulkan recommended for wgpu)
- Development libraries:
  ```bash
  # Debian/Ubuntu
  sudo apt install libxkbcommon-dev libssl-dev pkg-config

  # Fedora
  sudo dnf install libxkbcommon-devel openssl-devel pkg-config
  ```

### Build

```bash
# Clone the repository
git clone https://github.com/wayray/wayray.git
cd wayray

# Build all components
cargo build --release

# Binaries are in target/release/
ls target/release/wrsrvd target/release/wrclient target/release/wradm
```

### Docker

```dockerfile
# Server image available
docker pull wayray/wrsrvd:latest

# Or build locally
docker build -t wrsrvd -f docker/server.Dockerfile .
```

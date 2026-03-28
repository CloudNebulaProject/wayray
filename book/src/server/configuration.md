# Server Configuration

> This page will be written as the server implementation matures.

## Configuration File

WayRay server reads configuration from `wayray-server.toml`:

```toml
[server]
listen = "0.0.0.0:4433"
renderer = "pixman"  # or "gles"

[tls]
cert = "/etc/wayray/server.crt"
key = "/etc/wayray/server.key"

[session]
timeout = "24h"       # How long suspended sessions persist
max_sessions = 100    # Maximum concurrent sessions

[encoding]
strategy = "adaptive" # "lossless", "lossy", or "adaptive"
max_fps = 60
tile_size = 64

[audio]
enabled = true
codec = "opus"
sample_rate = 48000

[usb]
enabled = true
allowed_classes = ["mass-storage", "printer"]
```

# Client Configuration

> This page will be expanded as the client implementation matures.

## Configuration File

WayRay client reads configuration from `wayray-client.toml`:

```toml
[connection]
server = "192.168.1.100:4433"
ca_cert = "/etc/wayray/ca.crt"  # Server CA for verification

[token]
type = "software"       # "software", "smartcard", or "nfc"
value = "my-token-id"   # For software tokens

[display]
fullscreen = true
scale = 1.0

[audio]
enabled = true
input_device = "default"
output_device = "default"

[usb]
enabled = true
auto_forward = false    # Require manual device selection
```

# Quick Start

This guide gets you running a WayRay server and connecting a client in minutes.

## 1. Generate TLS Certificates

WayRay requires TLS for all connections. For development, generate self-signed certificates:

```bash
wayray-ctl cert generate --output ./certs
```

This creates `certs/server.crt` and `certs/server.key`.

## 2. Start the Server

```bash
wayray-server \
  --cert ./certs/server.crt \
  --key ./certs/server.key \
  --listen 0.0.0.0:4433 \
  --renderer pixman  # Use 'gles' if GPU is available
```

The server starts and listens for client connections on port 4433.

## 3. Connect a Client

On another machine (or the same machine for testing):

```bash
wayray-client \
  --server 192.168.1.100:4433 \
  --token my-dev-token \
  --ca ./certs/server.crt  # Trust the self-signed cert
```

A window opens showing your remote desktop. Open applications on the server and they appear in the client.

## 4. Launch Applications

From an SSH session to the server, or from a terminal within the WayRay session:

```bash
# Set the Wayland display to your WayRay session
export WAYLAND_DISPLAY=wayray-0

# Launch applications
foot &          # Terminal
firefox &       # Browser
nautilus &      # File manager
```

## 5. Test Session Mobility

1. Note the token you used (`my-dev-token`)
2. Close the client window (or press the disconnect key)
3. Your applications keep running on the server
4. Reconnect with the same token:
   ```bash
   wayray-client --server 192.168.1.100:4433 --token my-dev-token
   ```
5. Your desktop reappears with all windows intact

## Next Steps

- [Server Configuration](./server/configuration.md) for production setup
- [Token Setup](./client/tokens.md) for smart card or NFC tokens
- [Session Mobility](./concepts/session-mobility.md) to understand hot-desking

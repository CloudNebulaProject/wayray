# Running the Server

> This page will be expanded as the server implementation matures.

## Standalone

```bash
wayray-server --config /etc/wayray/wayray-server.toml
```

## Systemd Service

```ini
[Unit]
Description=WayRay Thin Client Server
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/wayray-server --config /etc/wayray/wayray-server.toml
Restart=always
User=wayray
Group=wayray

[Install]
WantedBy=multi-user.target
```

## Docker

```bash
docker run -d \
  --name wayray-server \
  -p 4433:4433/udp \
  -v /etc/wayray:/etc/wayray:ro \
  wayray/wayray-server:latest
```

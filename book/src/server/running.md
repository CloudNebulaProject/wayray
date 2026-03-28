# Running the Server

> This page will be expanded as the server implementation matures.

## Standalone

```bash
wrsrvd --config /etc/wayray/wrsrvd.toml
```

## Systemd Service

```ini
[Unit]
Description=WayRay Thin Client Server
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/wrsrvd --config /etc/wayray/wrsrvd.toml
Restart=always
User=wayray
Group=wayray

[Install]
WantedBy=multi-user.target
```

## Docker

```bash
docker run -d \
  --name wrsrvd \
  -p 4433:4433/udp \
  -v /etc/wayray:/etc/wayray:ro \
  wayray/wrsrvd:latest
```

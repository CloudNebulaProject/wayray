# Overview

## System Components

WayRay consists of four components:

### wayray-server

The server is a Wayland compositor built on [Smithay](https://github.com/Smithay/smithay). It:

- Accepts connections from Wayland applications (terminals, browsers, editors, etc.)
- Manages windows, input focus, and display outputs
- Renders all application surfaces into a framebuffer
- Encodes changed regions and transmits them to connected clients
- Manages user sessions and authentication
- Forwards audio via PipeWire

The server runs on Linux and can operate headless (no GPU required) or with GPU acceleration.

### wayray-client

The client is a lightweight viewer application that:

- Connects to a WayRay server over QUIC
- Receives and decodes display frames
- Renders frames to the local display
- Captures keyboard, mouse, and touch input and sends it to the server
- Handles audio playback and microphone capture
- Manages USB device forwarding

The client runs on Linux, with macOS and Windows support planned.

### wayray-protocol

A shared library defining the wire protocol between server and client. This ensures both sides agree on message formats at compile time.

### wayray-ctl

A command-line tool for administering WayRay servers:

- List and manage active sessions
- Configure server settings
- Monitor performance and network statistics
- Manage multi-server groups

## How It Works

```
You sit at a WayRay client terminal.
You insert your token (smart card, tap NFC, or open software token).
The client sends your token to the server.
The server finds or creates your session.
Your desktop appears on screen -- all your applications running, exactly as you left them.
You work normally. Every keystroke goes to the server; every frame comes back.
You pull your token and walk away. Your session suspends but keeps running.
You sit at another terminal across the building. Insert your token.
Your desktop reappears. Same windows, same state. Under a second.
```

## Network Requirements

- **LAN**: 100 Mbps minimum, Gigabit recommended
- **WAN**: 5 Mbps minimum for basic use, 20+ Mbps for video content
- **Latency**: < 30ms for comfortable interactive use, < 10ms ideal
- **Protocol**: QUIC over UDP (port configurable, default 4433)

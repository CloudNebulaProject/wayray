# WayRay

**WayRay** is a modern thin client Wayland compositor inspired by Oracle/Sun's legendary SunRay system. It brings stateless thin client computing into the Wayland era with session mobility, hot-desking, and zero-state endpoints.

## What is a Thin Client Compositor?

In a traditional desktop, applications run on the machine in front of you. In a thin client system, applications run on a remote server, and only the display output is transmitted to your device. Your local device (the "thin client") is just a screen, keyboard, and mouse.

WayRay takes this further by being a **Wayland compositor** on the server side. Applications connect to WayRay via the standard Wayland protocol, completely unaware they're being remoted. WayRay renders their output, encodes it efficiently, and transmits it to the client over the network.

## Inspired by SunRay

Sun Microsystems introduced SunRay in 1999 as a truly stateless thin client. Its defining feature was **session mobility**: users carried a smart card, and wherever they inserted it, their desktop appeared -- all windows, all applications, exact state. Pull the card, walk to another terminal, insert it, and your desktop followed in under a second.

SunRay was discontinued in 2014 when Oracle deprioritized the product line. WayRay aims to bring back these concepts with modern technology:

| SunRay | WayRay |
|--------|--------|
| Proprietary ALP protocol | QUIC with TLS 1.3 |
| X11 server (Xnewt) | Wayland compositor (Smithay) |
| Purpose-built hardware | Any device |
| Smart card only | Smart card, NFC, or software tokens |
| Solaris/Linux server | Linux server |

## Key Features

- **Session Mobility**: Your desktop follows your token, not your device
- **Stateless Clients**: Nothing stored on the client device
- **Wayland Native**: Standard Wayland applications work unmodified
- **Adaptive Encoding**: Lossless text, lossy video, content-aware compression
- **Low Latency**: QUIC transport with priority-based stream multiplexing
- **Audio & USB**: Full audio forwarding and USB device redirection
- **Secure by Default**: TLS 1.3 mandatory, no data at rest on clients

## Who is WayRay For?

- **Enterprise IT**: Centralized desktop management, zero-trust endpoints
- **Education**: Shared lab computers where any student sits anywhere
- **Government/Defense**: Stateless endpoints with no data exfiltration risk
- **Remote Work**: Access your full desktop from any location
- **Kiosk/Public Terminals**: Purpose-built stateless access points

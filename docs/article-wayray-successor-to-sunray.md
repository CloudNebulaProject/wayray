# WayRay: A Modern Successor to Sun's Legendary Thin Client

## The Desktop That Followed You

In 1999, Sun Microsystems shipped a product that was two decades ahead of its time. The Sun Ray was a small, silent, fanless box with no hard drive, no local operating system, and no moving parts. It consumed four watts of power. It had a smart card reader, a screen connector, and an Ethernet port. That was it.

And it was one of the most elegant pieces of computing ever built.

You would sit down at a Sun Ray terminal, insert your smart card, and your desktop appeared. Not a fresh desktop -- *your* desktop. Every window you had open, every application running, every cursor position exactly where you left it. Pull the card, walk across the building to another terminal, insert the card, and your desktop followed you in under a second. The terminal was nothing. The card was everything.

Over 500,000 Sun Ray units were sold. NASA used them. The US military used them. Universities, hospitals, banks, airlines, and government agencies around the world deployed them. Sun Ray terminals ran for years without a single reboot, because there was nothing on them to reboot.

Then Oracle bought Sun Microsystems in 2010. By 2014, the Sun Ray was dead.

Nothing has replaced it since.

## What Made Sun Ray Special

Sun Ray was not just another remote desktop product. It implemented a philosophy: **the endpoint is stateless, the session is mobile, and the network is the computer.**

A Sun Ray Desktop Terminal Unit stored zero user data. If it was stolen, nothing was compromised. If it broke, you replaced it with any other unit -- no configuration, no imaging, no enrollment. Plug it in, it works. The terminal ran a tiny real-time operating system in firmware that did exactly one thing: display pixels from the server and send back keystrokes.

All applications ran on centralized servers. The Sun Ray Software (SRSS) ran an X server instance per user, rendered the desktop, and streamed compressed frames to whichever terminal the user's smart card was in. The proprietary Appliance Link Protocol (ALP) handled display, input, audio, and USB over UDP with purpose-built optimizations for each data type.

Session mobility was the signature feature. The smart card was a session locator, not an authentication mechanism (that was handled separately by the login screen). Insert the card: session attaches. Remove the card: session detaches but keeps running. Insert it somewhere else: session reattaches. In a large deployment with failover groups, your session could follow you across buildings, campuses, and even geographic regions.

The experience was seamless in a way that no modern VDI product has replicated. Citrix, VMware Horizon, and Amazon WorkSpaces all offer "session roaming," but none match the sub-second, pull-card-insert-card simplicity. The difference is architectural: those products bolt mobility onto a design that was fundamentally about remote access. Sun Ray was designed from the ground up around mobility.

## What Went Wrong

Oracle's discontinuation of Sun Ray was not a technology failure. It was a business decision. Sun Ray reportedly generated profit, but not enough profit for Oracle's standards. Oracle is a database and cloud company; niche thin client hardware did not fit their strategy.

The engineering damage was immediate. Key teams were gutted after the acquisition. When American Airlines needed USB printing support for a major kiosk deployment, Oracle had already eliminated the USB engineering team. The feature was never delivered. Airlines standardized on Windows PCs instead. That kind of cascade failure played out across the entire Sun Ray customer base.

But the deeper problem was structural. ALP was proprietary. The hardware was purpose-built. When the single vendor walked away, there was no path forward. Customers could not switch to a different ALP implementation because none existed. They could not repurpose the hardware because it ran nothing else. The most technically elegant thin client system ever built was also the most vendor-locked.

The industry moved on to "good enough" alternatives: managed x86 thin clients running Windows Embedded or Linux, connecting over RDP or Citrix ICA to virtual desktops. These are fine products. They are not Sun Ray. The session mobility is gone. The zero-state endpoint is gone. The simplicity is gone.

## Why These Ideas Still Matter

The problems Sun Ray solved have not gone away. They have gotten worse.

**Endpoint security** is harder than ever. Every laptop is a potential data breach. Zero-trust architectures spend enormous effort trying to secure endpoints that fundamentally cannot be secured because they store data locally. A stateless terminal eliminates the problem entirely.

**Desktop management** at scale remains expensive. Imaging, patching, driver management, hardware variation, user migration -- the operational cost of maintaining thousands of individual desktops is staggering. Centralized compute with stateless endpoints reduces this to server management.

**Session mobility** has become more important, not less. Hybrid work, hot-desking, shared spaces, and bring-your-own-device are now standard. People expect to pick up where they left off, on any device, instantly. Cloud desktops (Windows 365, Amazon WorkSpaces) attempt this but with minutes of startup time, not milliseconds.

**Thin client hardware** has become trivially cheap. A Raspberry Pi costs $35. A refurbished mini-PC costs $50. The purpose-built Sun Ray DTU was necessary in 1999 when display decoding required custom silicon. Today, any commodity computer can serve as a thin client endpoint.

The concepts were right. The implementation was ahead of its time. What was missing was an open-source, vendor-neutral, standards-based reimplementation that could not be killed by a single company's business decision.

## WayRay: Bringing It Back

WayRay is a modern thin client system built on open standards and written in Rust. It is a ground-up reimplementation of the Sun Ray philosophy using today's technology stack.

Where Sun Ray ran a proprietary X server, WayRay runs a **Wayland compositor** built on Smithay. Where ALP streamed frames over proprietary UDP, WayRay uses **QUIC** with mandatory TLS 1.3 encryption. Where Sun Ray required purpose-built hardware, WayRay runs on **any device** that can decode video and display pixels.

The architecture follows Sun Ray's model faithfully: applications run on the server, the compositor renders them into a framebuffer, changed regions are encoded and transmitted to the client, and input flows back. The client stores nothing. The session is server-side state bound to a token.

But WayRay improves on Sun Ray in every dimension that matters.

### Modern Protocol Stack

Sun Ray's ALP was technically excellent but proprietary and undocumented. WayRay uses QUIC, the protocol that powers HTTP/3 and is deployed at global scale by Google, Cloudflare, and every major CDN.

QUIC provides independent stream multiplexing -- a lost display frame packet does not block input delivery. It mandates TLS 1.3 -- every connection is encrypted, always. It supports 0-RTT reconnection -- session resumption completes in zero round trips. And it handles connection migration -- when a client's IP address changes (WiFi to Ethernet, for example), the session continues without interruption.

Sun Ray's ALP encrypted keystrokes and display traffic but left USB device traffic unencrypted. WayRay encrypts everything. There is no unencrypted mode.

### Adaptive Content-Aware Encoding

Sun Ray used a single compression algorithm for all display content. WayRay classifies content by type and encodes each optimally:

- **Text and UI elements**: lossless compression. Terminal output, code editors, and menus are pixel-perfect. No compression artifacts on the characters you read all day.
- **Photographic content**: lossy JPEG/WebP encoding. Photos and images get high compression ratios with imperceptible quality loss.
- **Video regions**: hardware-accelerated H.264 or AV1 encoding. When a video plays, WayRay detects the rapidly changing region and routes it through a video codec. The result is indistinguishable from local playback.
- **Network adaptation**: when bandwidth drops, WayRay adjusts quality automatically. On a gigabit LAN, frames arrive with minimal compression at high framerates. Over a constrained WAN link, aggressive compression kicks in while keeping text perfectly sharp.

This content-adaptive approach means a WayRay session looks perfect for desktop work (the dominant use case) while handling media content efficiently when it appears.

### Session Mobility Without the Smart Card

Sun Ray's session mobility required a physical smart card. WayRay generalizes this to a **pluggable token system**:

- **Software tokens**: a UUID stored on the client device. No hardware required. Suitable for personal devices and development.
- **Smart cards**: direct Sun Ray heritage, for enterprise deployments that want hardware-backed security.
- **NFC**: tap a badge or phone to connect. Modern and fast.
- **Smartphone proximity**: the most interesting option. Your phone runs a companion app that broadcasts a Bluetooth Low Energy beacon. When you walk up to a WayRay terminal, it detects your phone and your session appears. Walk away, and the session suspends. No card to insert, no button to press. Your phone -- the device you always carry -- is the key.

For offices that want the physical precision of a smart card, a wireless charging pad with an NFC reader gives the same experience: phone on the pad is "card inserted," phone lifted is "card removed." You charge your phone and authenticate simultaneously.

The token carries not just your identity but your server address. Sit at a WayRay terminal at a friend's house or a customer's office, and your phone tells the terminal where to find your desktop. Your session comes from your server, over the internet. The local infrastructure is just a screen and a network connection.

### Pluggable Window Management

One of the things Unix enthusiasts loved about the SunRay stack on Solaris was X11's flexibility: you could run any window manager. CDE, FVWM, dwm, i3, xmonad -- the X server did not care. Wayland killed this by merging the compositor and window manager into one process.

WayRay brings it back. The compositor and window manager are separate processes communicating via a custom Wayland protocol. The compositor handles rendering, encoding, and network transport. The window manager handles layout, focus, keybindings, and decorations.

WayRay ships with a default floating window manager for a traditional desktop experience. But you can swap it -- at runtime, without restarting your session -- for a tiling window manager, a keyboard-driven manager, or anything custom. Write your own in Rust, Python, Go, or any language with Wayland client bindings. If the window manager crashes, the compositor continues running and your applications are unaffected.

This is not just nostalgia. It is the Unix philosophy applied to the desktop: small, composable tools that do one thing well.

### Federation and Protocol Gateways

Sun Ray operated within a single organization's network. Session mobility worked across failover groups within one deployment but not across organizational boundaries.

WayRay supports **federation**: servers from different organizations can establish trust relationships and share application windows across boundaries. A consultant visiting a customer site can see the customer's published applications integrated into their own desktop, with clear visual indicators showing the trust boundary.

More powerfully, WayRay includes a **protocol gateway** that handles VPN establishment, RDP session startup, and seamless window integration as a managed service. A maintenance engineer can say "connect to Customer D" and windows from the customer's Windows environment appear in their desktop, each in an isolated network namespace. Multiple customer environments can be active simultaneously, each hermetically sealed from the others.

Virtual desktops tie it together: one workspace for your desktop, one for Customer B's federated WayRay apps, one for Customer C's Windows environment via RDP, one for Customer D behind a VPN. Switch between them with a keyboard shortcut. All in one session that follows you wherever you go.

### illumos: Honoring the Heritage

Sun Ray ran on Solaris. WayRay runs on **illumos** -- the open-source continuation of the Solaris codebase -- as a first-class target alongside Linux.

This is not just a sentimental choice. illumos brings capabilities that are genuinely excellent for a thin client server:

- **Zones**: lightweight virtualization containers that provide proper session isolation with less overhead than Linux containers. Each user session runs in its own zone with its own filesystem view, network stack, and process namespace.
- **ZFS**: the best filesystem for managing user home directories, with per-user datasets, snapshots, quotas, and instant cloning.
- **DTrace**: unparalleled observability for debugging performance issues in production.
- **SMF**: a service management framework that makes running and supervising compositor sessions reliable and inspectable.

WayRay's architecture is headless-first: the compositor renders to a virtual framebuffer, not a physical display. This means the Linux-specific subsystems (DRM/KMS, libinput, udev) are optional add-ons, not core dependencies. The server runs on any illumos or Linux system, with or without a GPU.

For local desktop use on illumos workstations, WayRay can render directly to the framebuffer via `/dev/fb0` -- the classic SunOS `fbio` interface, alive and well in illumos -- or run nested inside an X11 session. No GPU required.

## What WayRay Is Not

WayRay is a compositor, not a desktop environment. You cannot run GNOME or KDE on WayRay, because those *are* compositors -- Mutter and KWin respectively. On Wayland, the desktop environment and the compositor are the same process.

Instead, the WayRay desktop is composed from independent Wayland applications: a window manager, a panel (like waybar), an application launcher (like fuzzel), a notification daemon (like mako), and whatever applications you use. This is how Sway, Hyprland, and River work today. It is more Unix than GNOME ever was.

WayRay does not handle user authentication, home directory mounting, or user environment setup. Those are the host system's responsibility. WayRay owns the compositor session and the token binding. A separate session launcher handles PAM, user provisioning, and environment setup -- the same separation that exists between Sway and greetd.

For cloud deployments, the login screen supports OAuth/OIDC: your phone scans a QR code, you authenticate against your identity provider, and the session launcher maps your cloud identity to a local user. The greeter is just a Wayland application, not a special subsystem.

## The Road Ahead

WayRay is being built in phases. The foundation -- a minimal Wayland compositor with framebuffer capture and QUIC transport -- comes first. Then the client viewer, input forwarding, and the pluggable window management protocol. Then session management and hot-desking. Then audio, USB, and the protocol gateways.

The code is Rust. The protocol is open. The architecture is documented in public Architecture Decision Records. There is no proprietary layer, no vendor lock-in, no single company whose quarterly earnings call can kill the project.

Sun Ray proved that stateless, session-mobile thin client computing was not just viable but superior for a wide range of use cases. Its discontinuation was a loss for the industry. WayRay aims to demonstrate that the idea was right -- it was just waiting for the right moment, the right tools, and the right community to build it again.

This time, in the open.

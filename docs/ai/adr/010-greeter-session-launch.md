# ADR-010: Greeter and Session Launch Architecture

## Status
Accepted

## Context
WayRay is a compositor, not a login manager. But thin clients need a login screen -- when a token arrives with no existing session, the user must authenticate before getting a desktop. We cannot run GNOME/KDE as desktop environments because they ARE Wayland compositors (Mutter/KWin) and cannot run inside WayRay.

### The Wayland DE Problem

On X11, desktop environments were clients of the X server. GNOME, KDE, XFCE all ran on any X server including SunRay's Xnewt. On Wayland, DEs are tightly coupled to their compositor:

| DE | Compositor | Can run on WayRay? |
|----|-----------|-------------------|
| GNOME Shell | Mutter | No -- Mutter IS a compositor |
| KDE Plasma | KWin | No -- KWin IS a compositor |
| XFCE4 (Wayland) | xfwm4/labwc | No -- needs its own compositor |
| Budgie (Wayland) | Magpie | No -- needs its own compositor |

What DOES work: all individual applications from these DEs (Nautilus, Dolphin, Firefox, etc.) are standard Wayland clients.

## Decision

### WayRay Does Not Own User Authentication or Environment Setup

WayRay's responsibility boundary:

**WayRay owns:**
- Wayland compositor (rendering, protocol, encoding, transport)
- Session lifecycle (active, suspended, resumed, destroyed)
- Token-to-session binding (which token maps to which compositor session)
- WM protocol (pluggable window management)

**WayRay does NOT own:**
- User authentication (PAM, LDAP, Kerberos)
- User environment (home dirs, NFS/ZFS mounts, shell, env vars)
- Desktop environment selection
- System-level session management (logind, zones)

### Session Launch Flow

```
┌──────────┐     token      ┌──────────────┐
│  Client   │──────────────►│  WayRay      │
│  (viewer) │               │  Server      │
└──────────┘               └──────┬───────┘
                                   │
                          Does session exist
                          for this token?
                                   │
                    ┌──────────────┼──────────────┐
                    │ YES                         │ NO
                    v                             v
             Resume session              ┌──────────────┐
             (sub-500ms)                 │ Session      │
                                         │ Launcher     │
                                         │ (external)   │
                                         └──────┬───────┘
                                                │
                                    1. Create user environment
                                       (PAM, mount home, etc.)
                                    2. Start WayRay compositor
                                       session for this user
                                    3. Launch greeter as first
                                       Wayland client
                                                │
                                                v
                                         ┌──────────────┐
                                         │ Greeter      │
                                         │ (Wayland     │
                                         │  client)     │
                                         └──────┬───────┘
                                                │
                                    User enters credentials
                                    Greeter authenticates via PAM
                                                │
                                                v
                                    Launch user's configured
                                    session (WM, panel, apps)
                                    Greeter exits
```

### The Session Launcher

A separate daemon/service (not part of WayRay) that:
1. Listens for "new session needed" events from WayRay
2. Creates the user environment (PAM session, mount home, set env vars)
3. Starts a WayRay compositor session for that user
4. Launches the greeter inside it

This is analogous to:
- SunRay: SRSS session manager + dtlogin
- Sway: greetd + gtkgreet/regreet
- GNOME: GDM + gnome-shell (but GDM IS a compositor; our greeter is a client)

We provide a **reference session launcher** (`wayray-session-launcher`) but the interface is a simple protocol/hook so admins can replace it with their own.

### The Greeter

A Wayland client application that:
- Displays a login form (username + password, or smart card PIN)
- Authenticates via PAM
- On success, signals the session launcher to proceed with user session setup
- Exits

We ship a reference greeter (`wayray-greeter`) but any Wayland client implementing the greeter protocol works. Community can build GTK, Qt, or TUI-based greeters.

### The User Session (Desktop Experience)

After authentication, the session launcher starts the user's configured environment:

```toml
# Example: ~/.config/wayray/session.toml

# Window manager (connects via wayray_wm_manager_v1 protocol)
wm = "wayray-wm-floating"
# wm = "wayray-wm-tiling"
# wm = "/usr/local/bin/my-custom-wm"

# Panel (layer-shell Wayland client)
panel = "waybar"

# App launcher
launcher = "fuzzel"

# Notification daemon
notifications = "mako"

# Autostart applications
autostart = [
    "foot",           # Terminal
    "nm-applet",      # Network manager tray
]
```

This composes a full desktop from independent Wayland clients:
- **WM**: Pluggable via our protocol (ADR-009)
- **Panel**: waybar, ironbar, or custom (layer-shell protocol)
- **Launcher**: fuzzel, wofi, tofi (layer-shell protocol)
- **Notifications**: mako, fnott (layer-shell protocol)
- **Apps**: any Wayland or X11 (via XWayland) application

### Integration Points

WayRay exposes hooks for the session launcher:

```
Event: session_requested(token) -> session_launcher handles it
Event: session_authenticated(user, token) -> launcher sets up user env
Event: session_logout(session_id) -> launcher tears down user env
```

The interface is intentionally minimal -- a few events over a Unix socket or D-Bus. This keeps WayRay decoupled from any specific auth/provisioning stack.

## Rationale

- **Separation of concerns**: WayRay is a compositor, not a login system. Authentication and environment provisioning vary wildly between deployments (LDAP vs local, NFS vs ZFS, containers vs bare metal). WayRay shouldn't know or care.
- **SunRay precedent**: SunRay's SRSS managed display sessions. dtlogin handled authentication. Solaris handled user environments. Same separation.
- **greetd precedent**: The Wayland ecosystem already solved this -- greetd + gtkgreet for Sway proves the external greeter model works.
- **Composability**: Administrators can swap any component. Corporate deploy with SSO greeter and locked-down session config. University deploy with student self-service. Home user with minimal auto-login.
- **Greeter as Wayland client**: No special rendering path. The greeter is just the first app that runs in the session. It uses the same compositor, same encoding, same network transport as everything else.

## Consequences

- Must define a stable session launcher interface (events/hooks)
- Must ship a reference greeter and reference session launcher for out-of-box usability
- Users expecting "GNOME over thin client" will be disappointed -- we should be clear in docs that WayRay provides a composable desktop, not a DE
- The session.toml concept needs careful design for both simplicity and flexibility
- Must support `wlr-layer-shell` protocol for panels, launchers, and notification daemons

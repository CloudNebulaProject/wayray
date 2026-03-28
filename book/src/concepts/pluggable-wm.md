# Pluggable Window Management

One of the things Unix enthusiasts loved about the SunRay + Solaris stack was X11's clean separation of display server and window manager. You could run CDE, FVWM, dwm, i3, or anything else -- the display server didn't care. Wayland merged these roles into one monolithic compositor, killing that flexibility.

WayRay brings it back.

## The Design

WayRay separates the **compositor** (rendering, protocol handling, input dispatch, frame encoding) from the **window manager** (layout, focus policy, keybindings, decorations). They are separate processes communicating via a custom Wayland protocol.

```
┌─────────────────────────┐     Wayland Protocol     ┌───────────────────┐
│   WayRay Compositor      │◄────────────────────────►│  Window Manager    │
│                          │                          │                    │
│  Renders frames          │  "Here are the windows"  │  Decides layout    │
│  Handles Wayland clients │  ─────────────────────►  │  Sets focus        │
│  Encodes for network     │                          │  Binds keys        │
│  Manages sessions        │  "Put them here"         │  Manages workspaces│
│                          │  ◄─────────────────────  │                    │
└─────────────────────────┘                          └───────────────────┘
```

## Two-Phase Transactions

Inspired by [River's window management protocol](https://isaacfreund.com/blog/river-window-management/), all WM operations happen in two atomic phases:

### Phase 1: Manage (Policy)
When something changes (new window, resize request, fullscreen request), the compositor tells the WM. The WM responds with policy decisions: "make this window 800x600", "focus this one", "use server-side decorations". The compositor then sends configure events to the affected Wayland clients.

### Phase 2: Render (Visual)
After clients acknowledge their new sizes and commit, the compositor tells the WM the final dimensions. The WM specifies exact visual placement: positions, z-order, borders. The compositor applies everything **atomically in one frame** -- no visual glitches, no half-rendered layouts.

## Supported Workflows

Because the WM is external, any paradigm is possible:

| Style | Description | Examples |
|-------|-------------|---------|
| **Floating** | Traditional desktop with draggable windows | Openbox, FVWM, Mutter |
| **Tiling** | Windows automatically fill the screen | i3, dwm, bspwm |
| **Dynamic** | Switch between tiling and floating on the fly | awesome, xmonad |
| **Keyboard-driven** | Fullscreen windows, prefix-key navigation | ratpoison, StumpWM |
| **Scrolling** | Windows in a strip, viewport scrolls | niri, PaperWM |
| **Custom** | Write your own in any language | You! |

## Default WM

WayRay ships with a built-in floating WM that activates when no external WM is connected. It provides a comfortable default experience with basic keyboard shortcuts. When you connect an external WM, the built-in one steps aside.

## Writing a Custom WM

A window manager for WayRay is just a Wayland client. You can write one in any language with Wayland client bindings:

- **Rust** using wayland-client
- **C** using libwayland-client
- **Python** using pywayland
- **Go** using go-wayland

The WM connects to the compositor, binds the `wayray_wm_manager_v1` global, and starts receiving window events. See the [Protocol Reference](../dev/protocol.md) for details.

## Crash Resilience

If your WM crashes:
- The compositor continues running
- All your applications stay alive
- Windows freeze in their last positions
- Restart the WM and it picks up where it left off

This is critical for a thin client server: a WM bug must never destroy user sessions.

## Hot-Swapping

You can switch WMs without restarting:
1. Start a new WM process
2. It connects and the old WM receives a "replaced" event
3. The old WM disconnects
4. The new WM receives the full window list and takes over

Try i3-style tiling in the morning, switch to floating for a presentation, back to tiling after lunch -- all without closing a single application.

# ADR-009: Pluggable Window Management Protocol

## Status
Accepted

## Context
One of SunRay's strengths in the Unix community was running on Solaris with its rich X11 ecosystem -- users could choose any window manager (CDE, FVWM, dwm, i3, etc.) because X11 cleanly separated the display server from window management via ICCCM/EWMH.

Wayland merged the compositor and window manager into one process, killing this flexibility. River (a Zig/wlroots compositor) pioneered re-separating them via a custom Wayland protocol (`river-window-management-v1`). WayRay should adopt this pattern to let users choose their workflow: floating, tiling, dynamic, or anything custom.

## Design

### Architecture

```
┌─────────────────────────┐
│     WayRay Compositor    │
│                          │
│  Wayland protocol impl   │
│  Rendering + encoding    │
│  Input dispatch          │
│  Session management      │
│                          │
│  ┌────────────────────┐  │
│  │ WM Protocol Server │  │  <── Custom Wayland protocol
│  └────────┬───────────┘  │
└───────────┼──────────────┘
            │ Unix socket (Wayland protocol)
            │
┌───────────┴──────────────┐
│   Window Manager Process  │
│                           │
│  Layout algorithm          │
│  Focus policy              │
│  Keybinding handling       │
│  Decoration decisions      │
│  Workspace management      │
│                           │
│  (any language with        │
│   Wayland client bindings) │
└───────────────────────────┘
```

The WM is a regular Wayland client that connects to the compositor and binds the `wayray_window_manager_v1` global. Only one WM can be bound at a time.

### Two-Phase Transaction Model (from River)

Inspired by River's `river-window-management-v1`, all WM operations happen in two atomic phases:

**Phase 1: Manage (Policy Decisions)**
1. Compositor sends `manage_start` when state changes occur (new window, close, resize request, fullscreen request)
2. WM evaluates and responds with policy: `propose_dimensions`, `set_focus`, `use_ssd`/`use_csd`, `grant_fullscreen`/`deny_fullscreen`
3. WM calls `manage_done`
4. Compositor sends `xdg_toplevel.configure` to affected windows

**Phase 2: Render (Visual Placement)**
1. After clients acknowledge configures and commit, compositor sends `render_start`
2. WM specifies visual state: `set_position`, `set_z_order`, `set_borders`, `show`/`hide`
3. WM calls `render_done`
4. Compositor applies all changes atomically in one frame

### Protocol Interfaces

#### `wayray_wm_manager_v1` (Global)
```xml
<interface name="wayray_wm_manager_v1" version="1">
  <!-- Lifecycle -->
  <event name="window_new">       <!-- New toplevel appeared -->
  <event name="window_closed">    <!-- Toplevel destroyed -->

  <!-- Manage phase -->
  <event name="manage_start">
  <request name="manage_done">

  <!-- Render phase -->
  <event name="render_start">
  <request name="render_done">

  <!-- Outputs -->
  <event name="output_new">
  <event name="output_removed">
  <event name="output_geometry">  <!-- size, scale, usable area -->
</interface>
```

#### `wayray_wm_window_v1` (Per-Window)
```xml
<interface name="wayray_wm_window_v1" version="1">
  <!-- Properties (events from compositor) -->
  <event name="title">
  <event name="app_id">
  <event name="parent">           <!-- Dialog parent -->
  <event name="size_hints">       <!-- min/max size, aspect ratio -->
  <event name="fullscreen_request">
  <event name="maximize_request">
  <event name="close_request">    <!-- User/app requested close -->
  <event name="dimensions">       <!-- Actual committed size after configure -->

  <!-- Policy requests (WM -> compositor, manage phase) -->
  <request name="propose_dimensions">  <!-- width, height -->
  <request name="set_focus">           <!-- keyboard focus to this window -->
  <request name="use_ssd">             <!-- server-side decorations -->
  <request name="use_csd">             <!-- client-side decorations -->
  <request name="grant_fullscreen">
  <request name="deny_fullscreen">
  <request name="close">               <!-- tell client to close -->

  <!-- Visual placement (WM -> compositor, render phase) -->
  <request name="set_position">        <!-- x, y relative to output -->
  <request name="set_z_above">         <!-- place above another window -->
  <request name="set_z_below">         <!-- place below another window -->
  <request name="set_z_top">           <!-- top of stack -->
  <request name="set_z_bottom">        <!-- bottom of stack -->
  <request name="set_borders">         <!-- color, width per edge -->
  <request name="show">
  <request name="hide">
  <request name="set_output">          <!-- assign to output -->
</interface>
```

#### `wayray_wm_seat_v1` (Input/Keybindings)
```xml
<interface name="wayray_wm_seat_v1" version="1">
  <!-- Keybinding registration -->
  <request name="bind_key">       <!-- key + modifiers + mode -->
  <request name="unbind_key">
  <request name="create_mode">    <!-- like i3 binding modes -->
  <request name="activate_mode">

  <!-- Keybinding delivery -->
  <event name="binding_pressed">
  <event name="binding_released">

  <!-- Pointer interactive operations -->
  <request name="start_move">     <!-- interactive window move -->
  <request name="start_resize">   <!-- interactive window resize -->
</interface>
```

#### `wayray_wm_workspace_v1` (Virtual Desktops / Tags)
```xml
<interface name="wayray_wm_workspace_v1" version="1">
  <request name="create_workspace">
  <request name="destroy_workspace">
  <request name="set_active_workspace">  <!-- per output -->
  <request name="assign_window">         <!-- window to workspace -->
  <request name="set_window_tags">       <!-- bitmask: window on multiple tags -->
  <event name="workspace_created">
  <event name="workspace_destroyed">
</interface>
```

### Supported WM Paradigms

| Paradigm | Example | How It Maps |
|----------|---------|-------------|
| Floating | Openbox, FVWM | Arbitrary `set_position` + `set_z_*` + interactive move/resize |
| Tiling | i3, dwm | Calculate layout on `manage_start`, batch `propose_dimensions` + `set_position` |
| Dynamic | awesome, xmonad | Switch layout algorithm, re-layout all windows atomically |
| Stacking/keyboard | ratpoison, StumpWM | Fullscreen windows, `hide`/`show` + binding modes for prefix keys |
| Scrolling | niri, PaperWM | Positions outside visible area, `set_position` with smooth offsets |
| Tags | dwm | `set_window_tags` bitmask, windows visible on multiple tags simultaneously |

### Default WM

WayRay ships with a built-in **floating WM** as the default. If no external WM connects, the built-in WM handles window placement with sane defaults (centered new windows, basic keyboard shortcuts). When an external WM connects, the built-in WM yields.

### Hot-Swap and Crash Resilience

- If the WM process crashes, the compositor continues running. Windows freeze in their last positions. A new WM can connect and take over.
- Hot-swap: a new WM can connect while the old one is running. The compositor sends a `replaced` event to the old WM, which should disconnect gracefully.
- On WM connect, the compositor sends the full window list so the WM can reconstruct state.

## Options Considered

### 1. Monolithic (WM logic in compositor)
- Simplest implementation
- Cannot swap WMs without restarting compositor (and all sessions!)
- Violates Unix philosophy
- Rejected: kills the "choose your workflow" feature

### 2. In-process plugin (like Wayfire/Compiz)
- WM loaded as a shared library
- Fast (no IPC overhead)
- Crash in WM crashes the compositor
- Language-restricted (must be Rust or C FFI)
- Rejected: not crash-resilient, not language-agnostic

### 3. External process via custom Wayland protocol (River model)
- Clean separation, crash-resilient, hot-swappable
- Language-agnostic (any Wayland client library)
- Small IPC overhead (negligible: manage/render happen once per frame at most)
- More complex to implement
- **Selected**: best fit for WayRay's values

## Rationale
- Honors the X11 tradition of WM choice that Unix enthusiasts loved
- A SunRay replacement that only offers one workflow would alienate the community
- River proved this works in practice with their two-phase model
- Crash resilience is critical for a thin client server (WM crash must not kill user sessions)
- Enables a community ecosystem of WMs in any language
- The two-phase transaction model prevents visual glitches (no partial layouts visible)

## Consequences
- Must implement and maintain a custom Wayland protocol
- Default floating WM adds code but is necessary for out-of-box usability
- WM developers need documentation and example implementations
- Slightly more latency than monolithic (one extra IPC roundtrip per layout change, ~microseconds)
- The protocol must be versioned carefully; breaking changes affect all WM implementations

# Phase 0: Foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Set up the Cargo workspace, build a minimal Smithay compositor that hosts Wayland clients, and capture framebuffers with damage tracking.

**Architecture:** Four-crate Cargo workspace (`wrsrvd`, `wrclient`, `wayray-protocol`, `wradm`) under `crates/`. Smithay with `default-features = false` and only portable features. Winit backend for dev/testing. PixmanRenderer is the design default but Phase 0 uses GlesRenderer via Winit for visual feedback. Headless/Pixman path deferred to Phase 1.

**Tech Stack:** Rust 2024, Smithay (wayland_frontend, xwayland, desktop, renderer_gl, backend_winit), calloop, tracing, miette, wayland-server

**Spec:** `docs/ai/plans/001-implementation-roadmap.md` (Phase 0), `docs/ai/adr/001-compositor-framework.md`, `docs/ai/adr/005-rendering-strategy.md`, `docs/ai/adr/007-project-structure.md`, `docs/ai/adr/008-illumos-support.md`

**Milestones:**
- **M0a:** Workspace builds, all crates compile (empty)
- **M0b:** Compositor runs, opens a Winit window, Wayland clients can connect
- **M0c:** Compositor renders client surfaces in the window
- **M0d:** Framebuffer capture works, damage regions tracked

---

## File Structure

```
wayray/
├── Cargo.toml                          # Workspace root
├── crates/
│   ├── wayray-protocol/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs                  # Shared types (empty for Phase 0, placeholder)
│   ├── wrsrvd/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs                 # Entry point: arg parsing, tracing init, event loop
│   │       ├── state.rs                # WayRay compositor state struct + all Smithay handler impls
│   │       ├── handlers/
│   │       │   ├── mod.rs              # Re-export handler modules
│   │       │   ├── compositor.rs       # CompositorHandler, commit logic
│   │       │   ├── xdg_shell.rs        # XdgShellHandler, toplevel/popup lifecycle
│   │       │   └── input.rs            # SeatHandler, keyboard/pointer focus
│   │       └── render.rs              # Render loop: render elements, damage tracking, framebuffer capture
│   ├── wrclient/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs                 # Placeholder binary
│   └── wradm/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs                 # Placeholder binary
└── tests/                              # Integration tests (future)
```

---

## Task 1: Workspace Scaffold

**Files:**
- Rewrite: `Cargo.toml` (workspace root)
- Create: `crates/wayray-protocol/Cargo.toml`
- Create: `crates/wayray-protocol/src/lib.rs`
- Create: `crates/wrsrvd/Cargo.toml`
- Create: `crates/wrsrvd/src/main.rs`
- Create: `crates/wrclient/Cargo.toml`
- Create: `crates/wrclient/src/main.rs`
- Create: `crates/wradm/Cargo.toml`
- Create: `crates/wradm/src/main.rs`
- Delete: `src/lib.rs` (default crate code)

- [ ] **Step 1: Create workspace root Cargo.toml**

Replace the existing `Cargo.toml` with a workspace definition. No `[package]` at root — workspace only.

```toml
[workspace]
resolver = "3"
members = [
    "crates/wayray-protocol",
    "crates/wrsrvd",
    "crates/wrclient",
    "crates/wradm",
]

[workspace.package]
edition = "2024"
version = "0.1.0"
license = "MPL-2.0"

[workspace.dependencies]
wayray-protocol = { path = "crates/wayray-protocol" }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
miette = { version = "7", features = ["fancy"] }
thiserror = "2"
```

- [ ] **Step 2: Create wayray-protocol crate**

`crates/wayray-protocol/Cargo.toml`:
```toml
[package]
name = "wayray-protocol"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
```

`crates/wayray-protocol/src/lib.rs`:
```rust
//! WayRay wire protocol definitions.
//!
//! Shared between wrsrvd (server) and wrclient (client).
```

- [ ] **Step 3: Create wrsrvd crate**

`crates/wrsrvd/Cargo.toml`:
```toml
[package]
name = "wrsrvd"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
wayray-protocol.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
miette.workspace = true
thiserror.workspace = true

smithay = { version = "0.3", default-features = false, features = [
    "wayland_frontend",
    "desktop",
    "renderer_gl",
    "backend_winit",
] }
```

`crates/wrsrvd/src/main.rs`:
```rust
fn main() {
    println!("wrsrvd compositor");
}
```

- [ ] **Step 4: Create wrclient and wradm placeholder crates**

`crates/wrclient/Cargo.toml`:
```toml
[package]
name = "wrclient"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
wayray-protocol.workspace = true
tracing.workspace = true
miette.workspace = true
```

`crates/wrclient/src/main.rs`:
```rust
fn main() {
    println!("wrclient viewer");
}
```

`crates/wradm/Cargo.toml`:
```toml
[package]
name = "wradm"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
wayray-protocol.workspace = true
tracing.workspace = true
miette.workspace = true
```

`crates/wradm/src/main.rs`:
```rust
fn main() {
    println!("wradm administration tool");
}
```

- [ ] **Step 5: Remove old src/lib.rs**

```bash
rm src/lib.rs
rmdir src
```

- [ ] **Step 6: Verify workspace builds**

```bash
cargo build --workspace
```

Expected: All four crates compile. Smithay downloads and builds with the specified features.

- [ ] **Step 7: Run each binary to verify**

```bash
cargo run --bin wrsrvd
cargo run --bin wrclient
cargo run --bin wradm
```

Expected: Each prints its name and exits.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "Set up Cargo workspace with four crates

Workspace: wrsrvd, wrclient, wayray-protocol, wradm under crates/.
Smithay configured with default-features=false, portable features only.
Implements ADR-007 project structure."
```

---

## Task 2: Tracing and Error Infrastructure

**Files:**
- Modify: `crates/wrsrvd/src/main.rs`
- Create: `crates/wrsrvd/src/errors.rs`

- [ ] **Step 1: Create error types**

`crates/wrsrvd/src/errors.rs`:
```rust
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum WayRayError {
    #[error("failed to initialize Winit backend")]
    #[diagnostic(help("ensure a display server (X11 or Wayland) is running"))]
    BackendInit(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("failed to initialize Wayland display")]
    #[diagnostic(help("check that the XDG_RUNTIME_DIR environment variable is set"))]
    DisplayInit(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("event loop error")]
    EventLoop(#[source] Box<dyn std::error::Error + Send + Sync>),
}
```

- [ ] **Step 2: Set up tracing and miette in main.rs**

`crates/wrsrvd/src/main.rs`:
```rust
mod errors;

use miette::{IntoDiagnostic, Result};
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("wrsrvd starting");
    Ok(())
}
```

- [ ] **Step 3: Verify it runs with RUST_LOG**

```bash
RUST_LOG=debug cargo run --bin wrsrvd
```

Expected: Tracing output with "wrsrvd starting" at info level. Debug level shows more output.

- [ ] **Step 4: Commit**

```bash
git add crates/wrsrvd/src/errors.rs crates/wrsrvd/src/main.rs
git commit -m "Add tracing and miette error infrastructure to wrsrvd"
```

---

## Task 3: Compositor State Struct

**Files:**
- Create: `crates/wrsrvd/src/state.rs`
- Modify: `crates/wrsrvd/src/main.rs`

This task defines the central `WayRay` state struct that holds all Smithay subsystem states. No handler impls yet — just the struct and construction.

- [ ] **Step 1: Create the state struct**

`crates/wrsrvd/src/state.rs`:
```rust
use smithay::{
    desktop::Space,
    input::{SeatState, Seat},
    reexports::wayland_server::{Display, DisplayHandle},
    utils::Clock,
    wayland::{
        compositor::CompositorState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        output::OutputManagerState,
    },
};

pub struct WayRay {
    pub display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub output_manager_state: OutputManagerState,
    pub space: Space<smithay::desktop::Window>,
    pub seat: Seat<Self>,
    pub clock: Clock<smithay::utils::Monotonic>,
}

impl WayRay {
    pub fn new(display: &mut Display<Self>) -> Self {
        let dh = display.handle();
        let clock = Clock::new();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let mut seat_state = SeatState::new();
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let space = Space::default();

        let seat = seat_state.new_wl_seat(&dh, "wayray");

        Self {
            display_handle: dh,
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            output_manager_state,
            space,
            seat,
            clock,
        }
    }
}
```

- [ ] **Step 2: Wire state into main.rs**

Update `crates/wrsrvd/src/main.rs`:
```rust
mod errors;
mod state;

use miette::{IntoDiagnostic, Result};
use smithay::reexports::wayland_server::Display;
use tracing::info;

use crate::state::WayRay;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("wrsrvd starting");

    let mut display = Display::<WayRay>::new()
        .map_err(|e| errors::WayRayError::DisplayInit(Box::new(e)))?;

    let _state = WayRay::new(&mut display);

    info!("compositor state initialized");
    Ok(())
}
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo build --bin wrsrvd
```

Expected: Compiles. May require adjusting imports based on exact Smithay API version. Fix any compile errors — the Smithay API docs are at https://docs.rs/smithay/latest/smithay/.

- [ ] **Step 4: Commit**

```bash
git add crates/wrsrvd/src/state.rs crates/wrsrvd/src/main.rs
git commit -m "Define WayRay compositor state struct with Smithay subsystems"
```

---

## Task 4: Wayland Protocol Handlers

**Files:**
- Create: `crates/wrsrvd/src/handlers/mod.rs`
- Create: `crates/wrsrvd/src/handlers/compositor.rs`
- Create: `crates/wrsrvd/src/handlers/xdg_shell.rs`
- Create: `crates/wrsrvd/src/handlers/input.rs`
- Modify: `crates/wrsrvd/src/state.rs` (add ClientData impl)
- Modify: `crates/wrsrvd/src/main.rs` (add mod handlers)

These handlers are required for Wayland clients to connect and interact with the compositor. Each handler implements a Smithay trait and uses the `delegate_*!` macro.

- [ ] **Step 1: Create ClientData and compositor handler**

`crates/wrsrvd/src/handlers/compositor.rs`:
```rust
use smithay::{
    delegate_compositor,
    wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState},
    reexports::wayland_server::{
        Client,
        protocol::wl_surface::WlSurface,
    },
};
use tracing::trace;

use crate::state::WayRay;

/// Per-client state stored by wayland-server.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl wayland_server::backend::ClientData for ClientState {
    fn initialized(&self, _client_id: wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: wayland_server::backend::ClientId,
        _reason: wayland_server::backend::DisconnectReason,
    ) {}
}

impl CompositorHandler for WayRay {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        trace!(?surface, "surface commit");
        // Ensure surface state is applied to any mapped xdg toplevels
        smithay::desktop::utils::on_commit_buffer_handler::<Self>(surface);
    }
}

delegate_compositor!(WayRay);
```

- [ ] **Step 2: Create xdg_shell handler**

`crates/wrsrvd/src/handlers/xdg_shell.rs`:
```rust
use smithay::{
    delegate_xdg_shell,
    desktop::{Space, Window},
    reexports::wayland_server::protocol::{wl_seat, wl_surface::WlSurface},
    utils::Serial,
    wayland::shell::xdg::{
        PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    },
};
use tracing::info;

use crate::state::WayRay;

impl XdgShellHandler for WayRay {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        self.space.map_element(window, (0, 0), false);
        info!("new toplevel mapped");
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // Popups handled minimally for now
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // Popup grabs not implemented yet
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // Popup repositioning not implemented yet
    }
}

delegate_xdg_shell!(WayRay);
```

- [ ] **Step 3: Create input (seat) handler**

`crates/wrsrvd/src/handlers/input.rs`:
```rust
use smithay::{
    delegate_seat, delegate_data_device,
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::selection::data_device::{
        DataDeviceHandler, DataDeviceState, ClientDndGrabHandler, ServerDndGrabHandler,
        set_data_device_focus,
    },
};
use tracing::trace;

use crate::state::WayRay;

impl SeatHandler for WayRay {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {
        trace!("focus changed");
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {
        // Cursor image handling deferred to render task
    }
}

delegate_seat!(WayRay);

// Data device (clipboard/drag-and-drop) — required by many Wayland clients.
impl DataDeviceHandler for WayRay {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for WayRay {}
impl ServerDndGrabHandler for WayRay {}

delegate_data_device!(WayRay);
```

- [ ] **Step 4: Create handlers mod.rs**

`crates/wrsrvd/src/handlers/mod.rs`:
```rust
mod compositor;
mod xdg_shell;
mod input;

pub use compositor::ClientState;
```

- [ ] **Step 5: Add DataDeviceState to WayRay struct**

Update `crates/wrsrvd/src/state.rs` to add:
- `data_device_state: DataDeviceState` field
- Import `smithay::wayland::selection::data_device::DataDeviceState`
- Initialize with `DataDeviceState::new::<Self>(&dh)` in `WayRay::new`

- [ ] **Step 6: Add `mod handlers` to main.rs**

Add `mod handlers;` to `crates/wrsrvd/src/main.rs`.

- [ ] **Step 7: Verify it compiles**

```bash
cargo build --bin wrsrvd
```

Expected: Compiles cleanly. The delegate macros generate the Wayland dispatch glue. Fix any import/API mismatches by consulting https://docs.rs/smithay/latest/smithay/.

Note: The exact Smithay API may require adjustments — trait method signatures, import paths, and delegate macro patterns can vary between Smithay versions. The Smallvil example in the Smithay repo (https://github.com/Smithay/smithay/tree/master/smallvil) is the authoritative reference for a minimal compositor. Consult it if you encounter API mismatches.

- [ ] **Step 8: Commit**

```bash
git add crates/wrsrvd/src/handlers/ crates/wrsrvd/src/state.rs crates/wrsrvd/src/main.rs
git commit -m "Implement Wayland protocol handlers for compositor, xdg_shell, seat"
```

---

## Task 5: Winit Backend and Event Loop

**Files:**
- Modify: `crates/wrsrvd/src/main.rs`
- Modify: `crates/wrsrvd/src/state.rs`

This task wires up the Winit backend (opens a window, creates a GlesRenderer) and the calloop event loop. After this task, wrsrvd opens a window and accepts Wayland client connections.

- [ ] **Step 1: Add Winit backend initialization to main.rs**

Replace the body of `main()` in `crates/wrsrvd/src/main.rs` with the full startup sequence:

```rust
mod errors;
mod handlers;
mod state;

use miette::{IntoDiagnostic, Result};
use smithay::{
    backend::{
        renderer::gles::GlesRenderer,
        winit::{self, WinitEventLoop, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{EventLoop, LoopSignal},
        wayland_server::Display,
    },
    utils::Transform,
};
use tracing::info;

use crate::{handlers::ClientState, state::WayRay};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("wrsrvd starting");

    // Wayland display
    let mut display = Display::<WayRay>::new()
        .map_err(|e| errors::WayRayError::DisplayInit(Box::new(e)))?;

    // Winit backend (opens a window, gives us a GlesRenderer)
    let (mut backend, mut winit_event_loop): (WinitGraphicsBackend<GlesRenderer>, WinitEventLoop) =
        winit::init()
            .map_err(|e| errors::WayRayError::BackendInit(Box::new(e)))?;

    // Create a virtual output matching the window size
    let output = Output::new(
        "wayray-0".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "WayRay".to_string(),
            model: "Virtual".to_string(),
        },
    );

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);

    // Compositor state
    let mut state = WayRay::new(&mut display, output.clone());

    // Map the output into the compositor's space
    state.space.map_output(&output, (0, 0));

    // calloop event loop
    let mut event_loop: EventLoop<WayRay> = EventLoop::try_new()
        .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;
    let loop_signal = event_loop.get_signal();

    // Insert the Wayland display as an event source
    event_loop
        .handle()
        .insert_source(
            smithay::reexports::calloop::generic::Generic::new(
                display.backend().poll_fd().try_clone_to_owned().unwrap(),
                calloop::Interest::READ,
                calloop::Mode::Level,
            ),
            move |_, _, _state| {
                // This is handled below via display.dispatch_clients
                Ok(calloop::PostAction::Continue)
            },
        )
        .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;

    // Print the Wayland socket name so clients can connect
    let socket_name = display
        .add_socket_auto()
        .map_err(|e| errors::WayRayError::DisplayInit(Box::new(e)))?;
    info!(?socket_name, "Wayland socket listening");
    std::env::set_var("WAYLAND_DISPLAY", socket_name.clone());

    info!("entering event loop");
    loop {
        // Process Winit events (window resize, close, input)
        let status = winit_event_loop.dispatch_new_events(|_event| {
            // Input events handled here in a later task
        });

        if let winit::WinitEvent::CloseRequested = status {
            info!("window close requested, shutting down");
            break;
        }

        // Dispatch Wayland clients
        display.dispatch_clients(&mut state)
            .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;
        display.flush_clients()
            .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;

        // Tick the event loop
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(16)), &mut state)
            .map_err(|e| errors::WayRayError::EventLoop(e.into()))?;
    }

    Ok(())
}
```

- [ ] **Step 2: Update WayRay::new to accept Output**

Modify `crates/wrsrvd/src/state.rs` — change the `new` signature to accept an `Output` parameter and store it:

```rust
pub output: Output,
```

And in `new()`:
```rust
pub fn new(display: &mut Display<Self>, output: Output) -> Self {
    // ... existing init ...
    Self {
        // ... existing fields ...
        output,
    }
}
```

- [ ] **Step 3: Verify it runs and opens a window**

```bash
cargo run --bin wrsrvd
```

Expected: A window opens. Log output shows the Wayland socket name. Closing the window exits cleanly.

- [ ] **Step 4: Test client connection**

In a separate terminal:
```bash
WAYLAND_DISPLAY=<socket_name_from_log> weston-info
```

Expected: `weston-info` connects and lists the compositor's globals (wl_compositor, xdg_wm_base, wl_shm, wl_seat, wl_output). It may error on missing protocols — that's fine for now.

Note: The exact event loop wiring depends on the Smithay version. The Smallvil example is the canonical reference. The code above is a guide — expect to adjust based on compile errors and the actual Smithay API.

- [ ] **Step 5: Commit**

```bash
git add crates/wrsrvd/src/main.rs crates/wrsrvd/src/state.rs
git commit -m "Wire up Winit backend and calloop event loop

Opens a window, creates Wayland socket, accepts client connections."
```

---

## Task 6: Rendering Client Surfaces

**Files:**
- Create: `crates/wrsrvd/src/render.rs`
- Modify: `crates/wrsrvd/src/main.rs` (call render in the loop)
- Modify: `crates/wrsrvd/src/handlers/compositor.rs` (schedule re-render on commit)

This task makes client surfaces actually visible in the Winit window. Uses Smithay's Space to collect render elements and OutputDamageTracker for efficient rendering.

- [ ] **Step 1: Create render module**

`crates/wrsrvd/src/render.rs`:
```rust
use smithay::{
    backend::renderer::{
        damage::OutputDamageTracker,
        element::surface::WaylandSurfaceRenderElement,
        gles::GlesRenderer,
    },
    desktop::space::SpaceRenderElements,
};

use crate::state::WayRay;

/// Render elements type alias for the compositor.
pub type WayRayRenderElements = SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>;

/// Render the compositor space to the backend.
///
/// Returns `true` if the output was updated (damage was present).
pub fn render_output(
    state: &mut WayRay,
    backend: &mut smithay::backend::winit::WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: &mut OutputDamageTracker,
) -> bool {
    let output = &state.output;
    let space = &mut state.space;

    // Collect render elements from the space
    let render_elements: Vec<WayRayRenderElements> =
        space.render_elements_for_output(backend.renderer(), output, 1.0).unwrap();

    // Render with damage tracking
    let render_result = backend.bind().and_then(|_| {
        damage_tracker.render_output(
            backend.renderer(),
            &backend.framebuffer(),
            0, // buffer age — Winit doesn't provide this reliably
            &render_elements,
            [0.1, 0.1, 0.1, 1.0], // dark grey background
        )
    });

    match render_result {
        Ok(render_output_result) => {
            let has_damage = !render_output_result.damage.is_empty();
            if has_damage {
                backend.submit(Some(&render_output_result.damage)).ok();
            }

            // Send frame callbacks to all visible surfaces
            space.elements().for_each(|window| {
                window.send_frame(
                    output,
                    state.clock.now(),
                    Some(std::time::Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            });

            has_damage
        }
        Err(err) => {
            tracing::warn!(?err, "render error");
            false
        }
    }
}
```

- [ ] **Step 2: Wire render into the main loop**

In `crates/wrsrvd/src/main.rs`, add to the main loop (after Winit event dispatch, before calloop dispatch):

```rust
// Render
render::render_output(&mut state, &mut backend, &mut damage_tracker);
```

Initialize `damage_tracker` before the loop:
```rust
use smithay::backend::renderer::damage::OutputDamageTracker;

let mut damage_tracker = OutputDamageTracker::from_output(&output);
```

Add `mod render;` at the top.

- [ ] **Step 3: Trigger re-render on surface commit**

In `crates/wrsrvd/src/handlers/compositor.rs`, update the `commit` method to also handle xdg surface initial configure:

```rust
fn commit(&mut self, surface: &WlSurface) {
    trace!(?surface, "surface commit");
    smithay::desktop::utils::on_commit_buffer_handler::<Self>(surface);

    // If this is an xdg toplevel that hasn't been configured yet, send initial configure
    if let Some(window) = self.space.elements().find(|w| {
        w.wl_surface().as_ref() == Some(surface)
    }).cloned() {
        if let Some(toplevel) = window.toplevel() {
            if !toplevel.is_initial_configure_sent() {
                toplevel.send_configure();
            }
        }
    }
}
```

- [ ] **Step 4: Verify rendering works**

```bash
cargo run --bin wrsrvd
```

In another terminal:
```bash
WAYLAND_DISPLAY=<socket> foot  # or weston-terminal, or any Wayland client
```

Expected: The Wayland client window appears inside the Winit compositor window with a dark grey background.

- [ ] **Step 5: Commit**

```bash
git add crates/wrsrvd/src/render.rs crates/wrsrvd/src/main.rs crates/wrsrvd/src/handlers/compositor.rs
git commit -m "Render client surfaces in the Winit window

Space collects render elements, OutputDamageTracker handles
efficient re-rendering. Clients appear in the compositor window."
```

---

## Task 7: Keyboard and Pointer Input

**Files:**
- Modify: `crates/wrsrvd/src/main.rs` (handle Winit input events)
- Modify: `crates/wrsrvd/src/state.rs` (add keyboard init)

Without input handling, clients can't receive keyboard or mouse events. This task routes Winit input events through the Smithay seat.

- [ ] **Step 1: Initialize keyboard on the seat**

In `crates/wrsrvd/src/state.rs`, inside `WayRay::new()`, after creating the seat:

```rust
seat.add_keyboard(Default::default(), 200, 25)
    .expect("failed to add keyboard to seat");
seat.add_pointer();
```

- [ ] **Step 2: Handle Winit input events**

In `crates/wrsrvd/src/main.rs`, replace the Winit event dispatch callback:

```rust
use smithay::backend::winit::WinitEvent;
use smithay::backend::input::Event;

let status = winit_event_loop.dispatch_new_events(|event| {
    match event {
        WinitEvent::Input(input_event) => {
            // Forward input to the compositor state
            state.process_input_event(input_event);
        }
        WinitEvent::Resized { size, .. } => {
            let mode = Mode {
                size,
                refresh: 60_000,
            };
            output.change_current_state(Some(mode), None, None, None);
        }
        _ => {}
    }
});
```

- [ ] **Step 3: Implement process_input_event on WayRay**

Add to `crates/wrsrvd/src/state.rs`:

```rust
use smithay::{
    backend::input::{
        self, InputEvent, KeyboardKeyEvent, PointerMotionAbsoluteEvent,
        PointerButtonEvent, PointerAxisEvent, InputBackend,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
};

impl WayRay {
    pub fn process_input_event<B: InputBackend>(&mut self, event: input::InputEvent<B>) {
        match event {
            InputEvent::Keyboard { event } => {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                let key_code = event.key_code();
                let state = event.state();
                let keyboard = self.seat.get_keyboard().unwrap();

                keyboard.input::<(), _>(
                    self,
                    key_code,
                    state,
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let output_geo = self.space.output_geometry(&self.output).unwrap();
                let pos = event.position_transformed(output_geo.size);
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();
                let under = self.space.element_under(pos).map(|(w, loc)| {
                    (w.wl_surface().unwrap().into(), loc)
                });

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos.to_f64(),
                        serial,
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::PointerButton { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();

                // Set keyboard focus to the window under the pointer on click
                if event.state() == input::ButtonState::Pressed {
                    if let Some((window, _)) = self.space.element_under(pointer.current_location()) {
                        let window = window.clone();
                        self.space.raise_element(&window, true);
                        let wl_surface = window.wl_surface().unwrap();
                        self.seat.get_keyboard().unwrap().set_focus(
                            self,
                            Some(wl_surface),
                            serial,
                        );
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state().into(),
                        serial,
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::PointerAxis { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let source = event.source();
                let mut frame = AxisFrame::new(event.time_msec()).source(source);

                use smithay::backend::input::Axis;
                if let Some(amount) = event.amount(Axis::Horizontal) {
                    frame = frame.value(smithay::input::pointer::Axis::Horizontal, amount);
                }
                if let Some(amount) = event.amount(Axis::Vertical) {
                    frame = frame.value(smithay::input::pointer::Axis::Vertical, amount);
                }

                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Verify input works**

```bash
cargo run --bin wrsrvd
```

Open a client:
```bash
WAYLAND_DISPLAY=<socket> foot
```

Expected: You can click on the terminal window to focus it. Typing produces text. Mouse events work.

- [ ] **Step 5: Commit**

```bash
git add crates/wrsrvd/src/state.rs crates/wrsrvd/src/main.rs
git commit -m "Route Winit input events through Smithay seat

Keyboard, pointer motion, click-to-focus, and scroll wheel
forwarded to Wayland clients."
```

---

## Task 8: Framebuffer Capture with ExportMem

**Files:**
- Modify: `crates/wrsrvd/src/render.rs`
- Modify: `crates/wrsrvd/src/state.rs` (add capture state)

This task adds framebuffer capture after each render pass, using Smithay's `ExportMem` trait. The captured data is the input for network transmission in Phase 1. For now we log the capture and optionally dump to a file for verification.

- [ ] **Step 1: Add capture infrastructure to state**

In `crates/wrsrvd/src/state.rs`, add:

```rust
/// Captured framebuffer data from the last render pass.
pub struct CapturedFrame {
    pub data: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub damage: Vec<smithay::utils::Rectangle<i32, smithay::utils::Physical>>,
}

// Add to WayRay struct:
pub last_capture: Option<CapturedFrame>,
```

- [ ] **Step 2: Add capture to render_output**

In `crates/wrsrvd/src/render.rs`, after successful render and before `backend.submit()`, add framebuffer capture:

```rust
use smithay::backend::renderer::ExportMem;
use smithay::backend::allocator::Fourcc;
use smithay::utils::Rectangle;

// Inside the Ok(render_output_result) branch, before submit:
if has_damage {
    // Capture the framebuffer
    let output_size = output.current_mode().unwrap().size;
    let region = Rectangle::from_loc_and_size(
        (0, 0),
        (output_size.w, output_size.h),
    );

    match backend.renderer().copy_framebuffer(
        &backend.framebuffer(),
        region,
        Fourcc::Argb8888,
    ) {
        Ok(mapping) => {
            match backend.renderer().map_texture(&mapping) {
                Ok(pixels) => {
                    tracing::debug!(
                        width = output_size.w,
                        height = output_size.h,
                        bytes = pixels.len(),
                        damage_rects = render_output_result.damage.len(),
                        "framebuffer captured"
                    );

                    state.last_capture = Some(CapturedFrame {
                        data: pixels.to_vec(),
                        width: output_size.w,
                        height: output_size.h,
                        damage: render_output_result.damage.clone(),
                    });
                }
                Err(err) => tracing::warn!(?err, "failed to map framebuffer"),
            }
        }
        Err(err) => tracing::warn!(?err, "failed to copy framebuffer"),
    }
}
```

Note: The exact `copy_framebuffer` API may differ — it may require `&self` vs `&mut self`, and the framebuffer handle may come from a different method. Adjust per Smithay's actual API. The key pattern is: render → copy_framebuffer → map_texture → get `&[u8]` pixel data.

- [ ] **Step 3: Verify capture works**

```bash
RUST_LOG=debug cargo run --bin wrsrvd
```

Open a client. Expected: Debug log shows "framebuffer captured" messages with correct dimensions and byte counts (width * height * 4 for ARGB8888).

- [ ] **Step 4: Commit**

```bash
git add crates/wrsrvd/src/render.rs crates/wrsrvd/src/state.rs
git commit -m "Add framebuffer capture via ExportMem after render

Captures ARGB8888 pixels from the rendered framebuffer with damage
regions. This is the input for network frame transmission in Phase 1."
```

---

## Task 9: Buffer Handler and Missing Delegates

**Files:**
- Modify: `crates/wrsrvd/src/handlers/mod.rs` or relevant handler files

Smithay requires several additional delegate implementations for a functional compositor. These are small but necessary — clients will fail to connect without them.

- [ ] **Step 1: Add buffer handler**

In `crates/wrsrvd/src/handlers/compositor.rs`:

```rust
use smithay::{delegate_output, wayland::buffer::BufferHandler};

impl BufferHandler for WayRay {
    fn buffer_destroyed(&mut self, _buffer: &wayland_server::protocol::wl_buffer::WlBuffer) {}
}

delegate_output!(WayRay);
```

- [ ] **Step 2: Add any remaining required delegates**

Check what's needed by attempting to compile and connect a client. Common missing delegates:

```rust
// In appropriate handler files:
smithay::delegate_xdg_decoration!(WayRay);  // if xdg_decoration feature enabled
```

The exact set depends on the Smithay version and features enabled. The compiler will tell you what's missing via trait bound errors.

- [ ] **Step 3: Final integration test**

Run the compositor and test with multiple clients:

```bash
cargo run --bin wrsrvd &
WAYLAND_DISPLAY=<socket> foot &
WAYLAND_DISPLAY=<socket> weston-simple-egl  # or another Wayland client
```

Expected:
- Multiple clients render in the compositor window
- Click to focus works
- Keyboard input reaches the focused client
- Debug logs show framebuffer capture on each render

- [ ] **Step 4: Commit**

```bash
git add crates/wrsrvd/src/
git commit -m "Add buffer handler and remaining Wayland delegates

Compositor now fully functional: hosts multiple Wayland clients,
renders them, handles input, captures framebuffers. Phase 0 complete."
```

---

## Notes for the Implementer

### Smithay API Stability
Smithay's API evolves between versions. The code in this plan is based on the 0.3.x API patterns. If you encounter mismatches:
1. Check the Smallvil example: https://github.com/Smithay/smithay/tree/master/smallvil
2. Check the Anvil example: https://github.com/Smithay/smithay/tree/master/anvil
3. Use `cargo doc --open` to browse the local Smithay API docs
4. Use context7 for library documentation lookups

### Common Pitfalls
- **Missing delegate macros**: Every Smithay handler trait needs a corresponding `delegate_*!` macro call. The compiler error will say something about `Dispatch` not being implemented.
- **Client connect failures**: If clients fail to connect, check that `add_socket_auto()` was called and `WAYLAND_DISPLAY` is set correctly.
- **Render not updating**: Make sure `send_frame` is called on surfaces after rendering, otherwise clients won't commit new buffers.
- **Import paths**: Smithay re-exports wayland-server types. Prefer `smithay::reexports::wayland_server` over depending on wayland-server directly.

### What's Next (Phase 1)
Phase 1 builds on this foundation:
- `wayray-protocol` crate gets wire protocol message definitions
- QUIC transport layer added to `wrsrvd` (server) and `wrclient` (client)
- `CapturedFrame` data gets encoded (zstd diff) and sent over QUIC
- `wrclient` receives, decodes, and displays frames

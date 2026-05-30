//! Ensure the crate is rebuilt when the protocol XML changes.
//!
//! The `wayland_scanner` macros generate the bindings from the XML at compile
//! time, but proc-macro file reads are not tracked by cargo's change detection.
//! Without this, editing `wayray-wm-v1.xml` would not trigger a rebuild and the
//! generated `Request`/`Event` enums would silently go stale.

fn main() {
    println!("cargo:rerun-if-changed=protocols/wayray-wm-v1.xml");
}

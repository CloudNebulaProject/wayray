# Vendored wayland-backend 0.3.15 with illumos support

This is `wayland-backend` 0.3.15 from crates.io (MIT licensed, see
LICENSE.txt), vendored via `[patch.crates-io]` in the workspace `Cargo.toml`
because upstream has no illumos support: the pure-Rust server poller only has
Linux/Android/Redox (epoll via rustix) and BSD/macOS (kqueue) code paths, so
the crate does not compile for `x86_64-unknown-illumos`.

## Local changes (all gated on `target_os = "illumos"`)

- `src/rs/server_impl/event_port.rs` (new): poller over illumos native event
  ports (`port_create(3C)` via `rustix::event::port`, whose `event` feature
  is already enabled). epoll exists on illumos only as a Linux-compat shim
  and is deliberately not used. Associations are one-shot, so the dispatch
  loop re-arms ready fds after handling them.
- `src/rs/server_impl/mod.rs`: declares the shim module.
- `src/rs/server_impl/common_poll.rs`: illumos poll-fd creation,
  `dispatch_all_clients` impl (with one-shot re-arm), and client
  deregistration on read error.
- `src/rs/server_impl/handle.rs`: illumos client registration in
  `insert_client`.
- `src/rs/socket.rs`: illumos lacks `MSG_CMSG_CLOEXEC`; use the macOS-style
  fallback (plain `MSG_DONTWAIT` + explicit `FD_CLOEXEC` on received fds).

Also removed from the published crate: `.cargo-ok`, `.cargo_vcs_info.json`,
`Cargo.toml.orig`, `Cargo.lock`.

No behavior change on any other platform. Drop this vendored copy once the
patch (or equivalent) lands in upstream wayland-rs and a release ships.

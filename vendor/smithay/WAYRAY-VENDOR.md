# Vendored smithay 0.7.0 with illumos build fixes

This is `smithay` 0.7.0 from crates.io (MIT licensed, see LICENSE.txt),
vendored via `[patch.crates-io]` in the workspace `Cargo.toml` because two
small pieces of platform-generic code fail to compile for
`x86_64-unknown-illumos`.

## Local changes (all gated on `target_os = "illumos"`)

- `src/backend/allocator/dmabuf.rs`: `rustix::ioctl::opcode` is gated off
  illumos; the `DMA_BUF_SYNC` opcode constant is spelled out manually there
  (dma-buf is Linux-only and the ioctl is never issued on illumos — the
  constant just needs to compile).
- `src/wayland/shm/pool.rs`: illumos libc exposes `siginfo_t::si_addr` as a
  method, not a field; added an illumos variant of `siginfo_si_addr`.

## Lint fixes (all platforms)

Path dependencies are compiled without `--cap-lints allow`, so this crate's
pre-existing warnings would fail CI's `RUSTFLAGS=-D warnings`:

- `src/wayland/shm/pool.rs`: route the SIGBUS handler cast through a function
  pointer (avoids `function_casts_as_integer`).
- `src/input/keyboard/mod.rs`, `src/wayland/seat/keyboard.rs`:
  `#[allow(unused_imports)]` on `tracing` imports that are unused under some
  feature sets.

Also removed from the published crate: `.cargo-ok`, `.cargo_vcs_info.json`,
`Cargo.toml.orig`, `Cargo.lock`.

No behavior change on any other platform. Drop this vendored copy once
equivalent fixes land upstream and a release ships.

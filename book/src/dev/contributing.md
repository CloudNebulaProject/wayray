# Contributing

## Development Setup

1. Clone the repository
2. Install prerequisites (see [Building from Source](./building.md))
3. Run `cargo build` to verify everything compiles
4. Run `cargo test --workspace` to verify tests pass

## Code Style

- Follow standard Rust conventions (`cargo fmt`, `cargo clippy`)
- Use `miette` for user-facing error messages with helpful diagnostics
- Use `tracing` for logging (not `println!` or `log`)
- Use `SeaORM` for any database operations (never raw SQL)
- Write tests for new functionality

## Architecture Guidelines

- Keep the client as simple as possible -- it's a "dumb terminal"
- Server-side complexity is preferred over client-side complexity
- Protocol changes must update both server and client in the same PR
- Performance matters: benchmark before and after for encoding/networking changes

## Pull Request Process

1. Create a feature branch from `main`
2. Make your changes with descriptive commits
3. Ensure `cargo test --workspace` passes
4. Ensure `cargo clippy --workspace` is clean
5. Open a PR with a description of what and why

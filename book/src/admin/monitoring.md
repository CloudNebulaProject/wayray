# Monitoring

> This page will be expanded as monitoring features are implemented.

## Metrics

WayRay exposes metrics for monitoring:

- Active sessions count
- Frame encoding latency (p50, p95, p99)
- Network bandwidth per session
- Audio latency
- CPU and memory usage per session

## Logging

WayRay uses structured logging via the `tracing` crate. Configure log levels via `RUST_LOG`:

```bash
RUST_LOG=wayray=info wayray-server   # Standard
RUST_LOG=wayray=debug wayray-server  # Verbose
RUST_LOG=wayray=trace wayray-server  # Everything
```

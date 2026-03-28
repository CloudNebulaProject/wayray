# Troubleshooting

> This page will be expanded as common issues are identified.

## Common Issues

### Client can't connect to server
- Check that UDP port 4433 is open
- Verify TLS certificates are correct
- Check server logs: `RUST_LOG=wayray=debug wrsrvd`

### High latency / poor responsiveness
- Check network latency: `ping <server>`
- Verify server encoding settings (try `strategy = "lossless"` for LAN)
- Monitor server CPU usage -- encoding may be bottlenecked

### Audio stuttering
- Check network jitter
- Increase audio buffer size in client config
- Verify PipeWire is running on the server

### Session doesn't resume
- Check session timeout hasn't expired
- Verify using the same token
- Check server logs for session lifecycle events

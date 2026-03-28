# Session Management

> This page will be expanded as session management is implemented.

## Session States

- **Creating**: New session being initialized
- **Active**: Session connected to a client, rendering and transmitting
- **Suspended**: Client disconnected, applications still running, no rendering
- **Destroyed**: Session terminated, all applications closed

## Managing Sessions

```bash
# List active sessions
wradm session list

# View session details
wradm session info <session-id>

# Terminate a session
wradm session kill <session-id>

# Set session timeout
wradm session set-timeout <session-id> 48h
```

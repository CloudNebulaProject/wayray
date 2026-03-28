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
wayray-ctl session list

# View session details
wayray-ctl session info <session-id>

# Terminate a session
wayray-ctl session kill <session-id>

# Set session timeout
wayray-ctl session set-timeout <session-id> 48h
```

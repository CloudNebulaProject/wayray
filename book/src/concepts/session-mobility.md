# Session Mobility

Session mobility is WayRay's signature feature, inherited from SunRay's legendary hot-desking capability.

## The Concept

Your desktop session is not tied to a physical device. It's tied to a **token** -- a smart card, NFC tag, or software identifier. Wherever you present that token, your session appears.

## How It Works

### Starting a Session

1. You sit at any WayRay client terminal
2. You present your token (insert smart card, tap NFC, or the client sends a stored software token)
3. The client sends the token to the server
4. The server checks: does a session for this token already exist?
   - **No**: Create a new session, authenticate the user (PAM login), start the desktop
   - **Yes**: Resume the existing session (skip to step 5)
5. The session binds to your client. Your desktop appears.

### Moving Between Terminals

1. You're working at Terminal A with your token
2. You remove your token (or press the disconnect shortcut)
3. Your session **suspends**: the client disconnects, but all applications keep running on the server
4. You walk to Terminal B and present your token
5. The server finds your suspended session and **resumes** it on Terminal B
6. Your desktop appears on Terminal B -- all windows, all applications, exact state

The entire reconnection happens in under 500 milliseconds on a LAN.

### Session Persistence

When you disconnect, your session enters the **Suspended** state:
- All applications continue running
- Window positions and states are preserved
- No frames are rendered or transmitted (saves CPU)
- The session stays alive for a configurable timeout (default: 24 hours)
- After timeout, the session is destroyed and applications are terminated

## Token Types

WayRay supports pluggable token providers:

### Software Token (Default)
A UUID stored on the client device. Simplest to set up, no hardware needed. Suitable for personal devices and development.

### Smart Card (PC/SC)
A physical smart card with a unique ID. Insert to connect, remove to disconnect. Enterprise-grade, tamper-resistant. Requires a smart card reader on the client.

### NFC
Tap a badge or phone to connect. Modern, fast, convenient. Requires an NFC reader on the client.

## Multi-Server Hot-Desking

In a multi-server deployment, your session might be running on Server A while you sit at a client connected to Server B:

1. Client connects to Server B with your token
2. Server B checks its local session registry -- no match
3. Server B queries the distributed session registry
4. Registry says: this token's session is on Server A
5. Server B redirects the client to Server A
6. Client connects to Server A, session resumes

This works across buildings, campuses, or even geographic regions (with higher latency).

## Security Considerations

- **Token theft**: A stolen smart card can access the session. Mitigate with PIN requirements or multi-factor authentication.
- **Session timeout**: Sessions should not persist indefinitely. Configure timeouts appropriate to your security policy.
- **Lock on disconnect**: Optionally lock the session screen when the token is removed, requiring password re-entry on resume.
- **Simultaneous tokens**: If the same token appears on two clients simultaneously, the older connection is terminated (last-writer-wins).

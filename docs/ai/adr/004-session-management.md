# ADR-004: Session Management and Hot-Desking

## Status
Accepted

## Context
SunRay's defining feature was session mobility: users could pull their smart card, walk to any terminal, insert the card, and their entire desktop appeared in under a second. WayRay must replicate this capability with modern technology.

## Design

### Session Lifecycle

```
                    token insert
    [No Session] ───────────────> [Creating]
                                      │
                                      v
                    ┌──────────> [Active] <──────────┐
                    │                 │               │
                    │    token remove │  token insert  │
                    │                 v    (new client)│
                    │           [Suspended] ──────────┘
                    │                 │
                    │      timeout    │
                    │                 v
                    │           [Destroyed]
                    │
                    │  client reconnect
                    └──── (same token)
```

### Session Identity
- Each session is bound to a **token** (smart card ID, badge ID, or software token)
- The token is NOT the user's identity -- it's a session locator
- Authentication happens separately via PAM after session attachment
- Multiple tokens can map to the same user (e.g., badge + phone NFC)

### Session Storage
- Active session state: in-memory (fast lookup)
- Session metadata (token mappings, user associations): SQLite via SeaORM
- Session registry for multi-server: distributed key-value store (etcd or custom)

### Hot-Desking Flow

1. Client connects to any WayRay server with a token
2. Server checks local session registry for token
3. If not found locally, queries other servers in the group
4. If session found on another server:
   - Source server suspends the session (detaches from old client)
   - Source server provides session endpoint to requesting server
   - Client is redirected to the source server
   - Session resumes on source server with new client
5. If no session exists: create new session, authenticate user via PAM
6. Entire flow target: < 500ms

### Session Suspension
When a token is removed or client disconnects:
- Compositor continues running (all applications stay alive)
- Frame rendering pauses (no ExportMem, save CPU)
- Network streams close gracefully
- Session enters Suspended state with configurable timeout (default: 24 hours)
- After timeout: session destroyed, all applications terminated

## Options Considered for Token Mechanism

### 1. Smart Card (PC/SC)
- Direct SunRay heritage
- Requires hardware on client (smart card reader)
- Well-suited for enterprise/government environments
- Libraries: pcsc-rust crate

### 2. NFC / RFID
- Modern equivalent of smart card
- Phone-based NFC as software token
- Lower-friction than smart card insertion

### 3. Software Token
- UUID-based token stored on client device
- No hardware required
- Can be combined with TOTP/FIDO2 for security
- Most accessible for development and lightweight deployments

### 4. All of the above (pluggable)
- Token is an opaque identifier; the mechanism doesn't matter to session management
- Different authentication backends provide tokens via a trait

## Decision
**Pluggable token system** with software tokens as the default, smart card and NFC as optional backends.

```rust
trait TokenProvider {
    fn token_inserted(&self) -> Option<SessionToken>;
    fn token_removed(&self) -> bool;
    fn watch(&self) -> TokenEventStream;
}
```

## Rationale
- Session mobility is the core differentiator; it must work across arbitrary token types
- Software tokens lower the barrier to entry (no hardware needed)
- Smart card support honors the SunRay heritage for enterprise deployments
- Pluggable design avoids committing to a single mechanism

## Consequences
- Must handle race conditions: token inserted on two clients simultaneously
- Session redirect adds latency; must be optimized carefully
- Multi-server session registry is a distributed systems problem (consistency vs availability)
- Software tokens are less secure than hardware tokens (can be copied); mitigate with device binding

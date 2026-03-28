# ADR-014: Federation and Foreign Surface Integration

## Status
Accepted

## Context

### The B2B Problem

Real-world scenarios that current remote desktop solutions handle poorly:

**Scenario 1: Visiting consultant**
A consultant from Company A visits Company B. They sit at Company B's thin client terminal. They need access to their own desktop on Company A's servers -- their email, their files, their tools. Today this means VPN + RDP + lag + frustration.

**Scenario 2: Customer-provided desktop**
Company B grants a contractor a desktop on Company B's infrastructure. The contractor needs to use it alongside their own desktop. Today this means switching between two RDP windows.

**Scenario 3: Shared application access**
Company B publishes specific applications (e.g., an internal tool) that Company A's employees can access. Not a full desktop -- just individual apps that appear in the user's desktop as if they were local.

**Scenario 4: Cross-server apps within one org**
A large organization runs WayRay servers in multiple departments. A user's session is on Server A but they need an app that's only licensed/installed on Server B. That app should appear in their desktop seamlessly.

### The Direct Remote Access Problem

**Scenario 4b: At a friend's place**
You're at a friend's house. They have a WayRay terminal. You want YOUR desktop from YOUR server (at home, in the cloud, at the office). Your friend's infrastructure is not involved at all -- their terminal is just a screen, keyboard, and network connection. No federation needed, no server-to-server trust. The client connects directly to your server over the internet.

**Scenario 4c: BYOD at a customer site**
You're working on-site at a customer. You sit at any available terminal (theirs, a shared kiosk, a conference room). You authenticate against your own company's server. Your desktop comes from your server over the internet. The customer's network is just a pipe.

These are different from federation: no cooperation between servers is required. The WayRay client is truly stateless and server-agnostic -- it can connect to ANY WayRay server the user directs it to.

#### Direct Remote Flow

```
┌──────────────┐                              ┌──────────────────┐
│ Friend's     │       Internet / WAN         │ Your WayRay      │
│ thin client  │─────────────────────────────►│ server (home /   │
│              │   QUIC + TLS 1.3             │ cloud / office)  │
│ Your token   │                              │                  │
│ authenticates│◄─────────────────────────────│ Your session     │
│ against YOUR │   Display + Audio frames     │ resumes          │
│ server       │                              │                  │
└──────────────┘                              └──────────────────┘

Friend's WayRay server: not involved at all.
The client just needs network access to your server.
```

This requires:
1. **Multi-server client UI**: The greeter/client must let the user choose which server to connect to (not hardcoded to one server)
2. **Token routing**: The user's token (BLE, NFC, software) carries or is associated with a server address
3. **Internet-reachable servers**: The user's WayRay server must be accessible (direct, QUIC NAT traversal, or relay)
4. **No local server involvement**: The friend's/customer's WayRay server is completely bypassed

#### Server Selection in the Client

```
┌─────────────────────────────────────────┐
│           WayRay Client                  │
│                                          │
│  Connect to:                             │
│                                          │
│  ┌─ Recent ──────────────────────────┐  │
│  │  ★ home.wayray.example.com        │  │
│  │    work.corpx.com                  │  │
│  └───────────────────────────────────┘  │
│                                          │
│  ┌─ Local Server ────────────────────┐  │
│  │    this-office.local (auto-discovered)│
│  └───────────────────────────────────┘  │
│                                          │
│  ┌─ Custom ──────────────────────────┐  │
│  │    [server address...............]  │  │
│  └───────────────────────────────────┘  │
│                                          │
│         [Connect]                        │
└──────────────────────────────────────────┘
```

Or with token-based routing: the BLE/NFC companion app stores the user's server address alongside the session token. When the phone is detected, the client knows both WHO the user is AND where their server lives. Fully automatic.

### The Sandboxing Problem

Even within a single session, not all apps should be equal:

**Scenario 5: Untrusted application**
User wants to run an app downloaded from the internet. It should be sandboxed -- limited filesystem access, no clipboard sniffing, no screen capture of other windows. But it should still appear as a window in the desktop.

**Scenario 6: Security zones**
A defense/government user has apps at different classification levels. SECRET apps run in a separate zone from UNCLASSIFIED apps. Both appear on the same display but with clear visual demarcation and strict isolation.

**Scenario 7: Multi-tenant on single server**
Multiple users' sessions run on the same server. Each session is isolated in a zone/container. Apps within a session cannot see other sessions' windows.

### The Common Thread

These scenarios fall into three categories:

1. **Direct remote**: Client connects to user's own server, no local server involved (4b, 4c). Simplest -- just needs multi-server client UI and internet reachability.
2. **Federation**: Surfaces from one server appear in a session on another server (1, 2, 3, 4). Requires server-to-server trust.
3. **Sandboxing**: Surfaces from an isolated local environment appear alongside local windows (5, 6, 7). Requires local trust boundaries.

Categories 2 and 3 share the same underlying mechanism -- foreign surface integration:

```
┌─────────────────────────────────────────┐
│             Trust Boundary               │
│                                          │
│  "Foreign" surface source:               │
│    - Remote WayRay server (federation)   │
│    - Local zone/container (sandbox)      │
│    - Different session on same server    │
│                                          │
│  Produces window surfaces that need      │
│  to appear in the "local" desktop        │
└─────────────┬───────────────────────────┘
              │
              │  Surface forwarding
              │  (frames + input + metadata)
              │
┌─────────────┴───────────────────────────┐
│         Local WayRay Compositor          │
│                                          │
│  Presents foreign surfaces alongside     │
│  local windows. WM manages all of them.  │
│  Visual indicators show trust boundary.  │
└─────────────────────────────────────────┘
```

## Decision

### Concept: Foreign Surfaces

A **foreign surface** is a window whose content comes from outside the local compositor's Wayland client pool. It could originate from:

1. A remote WayRay server (federation)
2. A local sandboxed environment (zone, container, namespace)
3. A different session on the same server (app sharing)

Foreign surfaces are presented to the local WM as regular windows but carry additional metadata about their origin and trust level.

### Display Modes

Foreign surfaces can be integrated at three levels:

#### Mode 1: Desktop-in-Desktop (Full Remote Session)

The remote desktop appears as a single window in the local compositor. All remote windows are composited remotely and arrive as one frame stream.

```
┌─ Local Desktop ──────────────────────────────┐
│                                               │
│  ┌─ Local App ──┐  ┌─ Remote Desktop ──────┐ │
│  │              │  │ ┌─────┐ ┌──────────┐  │ │
│  │  foot        │  │ │ vim │ │ firefox  │  │ │
│  │              │  │ └─────┘ └──────────┘  │ │
│  └──────────────┘  │  (Company A desktop)  │ │
│                     └──────────────────────┘ │
└───────────────────────────────────────────────┘
```

**Implementation:** Standard WayRay client connected to the remote server, running as a Wayland client in the local compositor. This works today with the basic architecture -- it's just a local app that happens to be a WayRay viewer.

**Use case:** Scenario 1 (visiting consultant accessing full home desktop).

#### Mode 2: Merged Windows (Seamless Apps)

Individual windows from the remote source appear as separate windows in the local desktop, managed by the local WM alongside local apps. No visible "remote desktop" container.

```
┌─ Local Desktop ──────────────────────────────┐
│                                               │
│  ┌─ foot ───────┐  ┌─ vim (remote) ────────┐ │
│  │ (local)      │  │ (from Company A)       │ │
│  │              │  │                         │ │
│  └──────────────┘  └────────────────────────┘ │
│                                               │
│  ┌─ firefox (remote) ───────────────────────┐ │
│  │ (from Company A)                          │ │
│  └───────────────────────────────────────────┘ │
└───────────────────────────────────────────────┘
```

**Implementation:** The remote server streams per-window frame updates instead of a composited desktop. Each remote window is registered as a foreign surface in the local compositor. The local WM positions and manages them like any other window.

**Use case:** Scenarios 2, 3, 4, 5, 6 -- anywhere individual apps need to integrate into the local desktop.

#### Mode 3: App Embedding (Portal)

A remote app's surface is embedded inside a local window (picture-in-picture, sidebar, etc.). Managed by the local app, not the WM.

**Implementation:** A local Wayland client requests a subsurface that is fed by a remote stream. More complex, less common. Future work.

### Federation Protocol

Federation extends the WayRay wire protocol (ADR-002) with:

#### Server-to-Server Trust

```
┌──────────────┐         Mutual TLS          ┌──────────────┐
│  WayRay      │◄────────────────────────────►│  WayRay      │
│  Server A    │    Federation established     │  Server B    │
│  (Company A) │                               │  (Company B) │
└──────────────┘                               └──────────────┘
```

Servers establish federation via mutual TLS with pre-exchanged certificates or an OIDC-based trust chain (both servers trust the same IdP, or IdPs trust each other).

#### Federation Discovery

```toml
# /etc/wayray/federation.toml

[federation]
enabled = true

# This server's identity
server_id = "wayray.companyb.com"
server_cert = "/etc/wayray/federation/server.crt"
server_key = "/etc/wayray/federation/server.key"

# Trusted federated servers
[[federation.peers]]
server_id = "wayray.companya.com"
ca_cert = "/etc/wayray/federation/companya-ca.crt"
# What this peer is allowed to do
permissions = ["seamless_apps", "full_desktop"]

[[federation.peers]]
server_id = "wayray.vendorx.com"
ca_cert = "/etc/wayray/federation/vendorx-ca.crt"
permissions = ["seamless_apps"]  # No full desktop access

# OIDC-based dynamic trust (any server whose cert chains to this IdP)
[federation.oidc_trust]
issuer = "https://federation-idp.example.com"
allowed_claims = { "org" = ["companya", "vendorx"] }
```

#### Seamless App Protocol

For Mode 2 (merged windows), the remote server sends per-window streams:

```
ForeignWindowAnnounce {
    window_id: u64,
    title: String,
    app_id: String,
    origin: ServerIdentity,     // "wayray.companya.com"
    trust_level: TrustLevel,    // Trusted, Sandboxed, Untrusted
    initial_size: (u32, u32),
    // Window content arrives on a dedicated QUIC stream
    stream_id: u64,
}

ForeignWindowUpdate {
    window_id: u64,
    // Same as FrameUpdate but for a single window
    regions: Vec<EncodedRegion>,
}

ForeignWindowInput {
    window_id: u64,
    // Input forwarded from local compositor to remote server
    event: InputEvent,
}

ForeignWindowClosed {
    window_id: u64,
}
```

The local compositor creates a synthetic Wayland surface for each foreign window and registers it with the WM. The WM manages it like any other window. Input events on that surface are forwarded back to the originating server.

### Sandboxing Architecture

Sandboxing reuses the foreign surface mechanism with a local transport:

```
┌─ Local WayRay Compositor ─────────────────────┐
│                                                │
│  Local Wayland clients ──► direct surfaces     │
│                                                │
│  Sandbox connector ──► foreign surfaces        │
│    │                                           │
│    │ Unix socket / shared memory               │
│    │                                           │
│  ┌─┴──────────────────────────┐                │
│  │  Sandbox (illumos zone /   │                │
│  │  Linux container / bubblewrap) │             │
│  │                             │                │
│  │  Nested WayRay compositor   │                │
│  │  (headless, per-app output) │                │
│  │                             │                │
│  │  Sandboxed app ──► Wayland  │                │
│  └─────────────────────────────┘                │
└─────────────────────────────────────────────────┘
```

**illumos zones** are particularly well-suited: each sandbox is a lightweight zone with its own filesystem view, network stack, and process namespace. The zone runs a headless WayRay compositor that forwards per-window surfaces to the parent.

**The sandbox connector** is the same protocol as federation but over a local Unix socket instead of QUIC. Same `ForeignWindowAnnounce`, same `ForeignWindowUpdate`, same input forwarding. The trust boundary is just local instead of remote.

### Trust Level Visual Indicators

Foreign surfaces carry a `TrustLevel` that the compositor renders as visual cues:

| Trust Level | Indicator | Source |
|-------------|-----------|--------|
| `Local` | None (normal window) | Local Wayland client |
| `Trusted` | Subtle origin badge (e.g., company logo in title bar) | Federated server with full trust |
| `Sandboxed` | Colored border (e.g., yellow) | Local sandbox (zone/container) |
| `Untrusted` | Prominent border (e.g., red) + origin label | Remote server with limited trust |
| `Classified` | Mandatory banner with classification level | Security-labeled zone |

The WM protocol (ADR-009) is extended to include trust metadata:

```xml
<interface name="wayray_wm_window_v1" version="1">
  <!-- ... existing events ... -->
  <event name="origin">
    <arg name="server_id" type="string" />
    <arg name="trust_level" type="uint" />  <!-- Local, Trusted, Sandboxed, Untrusted -->
  </event>
</interface>
```

The WM decides how to decorate based on trust level. A security-focused WM might enforce thick red borders for untrusted windows. A casual WM might show a subtle badge.

### Input Isolation

Foreign surfaces have restricted input capabilities by default:

| Capability | Local | Trusted | Sandboxed | Untrusted |
|-----------|-------|---------|-----------|-----------|
| Keyboard input (when focused) | Yes | Yes | Yes | Yes |
| Clipboard read | Yes | Configurable | No | No |
| Clipboard write | Yes | Configurable | No | No |
| Drag-and-drop | Yes | Configurable | No | No |
| Screen capture of other windows | Yes | No | No | No |
| Global keybinding registration | Yes | No | No | No |

The compositor enforces these restrictions at the protocol level -- a sandboxed window simply never receives clipboard events, regardless of what the app inside requests.

### B2B Invite Flow

Scenario 3 in practice:

```
Company B admin:
  1. Publishes app "internal-tool" for federation
  2. Creates invite: wayray-ctl federation invite \
       --app internal-tool \
       --peer wayray.companya.com \
       --user jdoe@companya.com \
       --expires 2026-06-01

Company A user (jdoe):
  1. Sits at their normal WayRay terminal
  2. Opens federation panel or runs: wayray-ctl federation accept <invite-url>
  3. OIDC authentication confirms jdoe's identity to Company B
  4. Company B's "internal-tool" appears as a window in jdoe's desktop
  5. Window has "Trusted" border and "Company B" origin badge
  6. Input goes to Company B's server, frames come back
  7. jdoe works with the tool alongside their own apps
```

### Relationship to Existing ADRs

| ADR | Relationship |
|-----|-------------|
| ADR-002 (QUIC Transport) | Federation uses QUIC between servers with mutual TLS |
| ADR-003 (Frame Encoding) | Per-window encoding for seamless apps |
| ADR-004 (Session Management) | Federated sessions carry origin metadata |
| ADR-009 (Pluggable WM) | WM manages foreign surfaces via `origin` event |
| ADR-010 (Greeter) | Greeter can show federated desktop options |
| ADR-012 (Cloud Auth) | OIDC trust chain enables dynamic federation |

## Rationale

- **Unified mechanism**: Federation (remote) and sandboxing (local) use the same foreign surface protocol. Write once, use for both.
- **Seamless apps are the killer feature**: Full remote desktops exist everywhere. Per-window integration across trust boundaries is rare and valuable.
- **illumos zones are perfect for sandboxing**: Lightweight, mature, proper filesystem/network isolation. WayRay on illumos gets sandboxing almost for free.
- **B2B invites solve a real pain point**: Consultants, vendors, and partners working across organizations is universal. Current solutions (VPN + RDP) are painful.
- **Trust indicators prevent confusion**: Users must always know which windows are local vs. remote vs. sandboxed. Visual cues are non-optional for security.
- **Progressive implementation**: Mode 1 (desktop-in-desktop) works today. Mode 2 (seamless) builds on it. Federation trust adds policy on top.

## Consequences

- Per-window streaming adds complexity to frame encoding (must track and stream individual surfaces, not just the composited output)
- Federation introduces distributed trust management (certificate exchange, OIDC trust chains)
- Seamless foreign windows have higher latency than local windows (network RTT). Visual responsiveness may differ noticeably.
- Input forwarding across trust boundaries must be carefully audited (keystroke interception, clipboard exfiltration)
- Sandbox zones consume additional memory and CPU per sandbox instance
- The nested compositor in sandboxes adds overhead but is necessary for proper Wayland isolation
- B2B invite system needs careful security design (invite revocation, time-limited access, audit logging)
- Protocol versioning across federated servers: both sides must speak compatible protocol versions

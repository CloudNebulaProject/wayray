# ADR-015: Virtual Desktops, RDP Integration, and Protocol Gateways

## Status
Accepted

## Context

A consultant or maintenance engineer works across multiple organizations daily. They need:
- Their own desktop (email, tools, docs)
- Access to Customer B's internal systems (WayRay federation)
- Access to Customer C's Windows environment (RDP)
- Access to Customer D behind a VPN (VPN + RDP)

Today this means juggling VPN clients, RDP windows, browser tabs, and context-switching between disconnected environments. With WayRay's virtual desktops, federation (ADR-014), and protocol gateways, all of this can live in one unified desktop experience.

## Virtual Desktops for Multi-Environment Work

### WM Workspace Integration

Virtual desktops are already part of the pluggable WM protocol (ADR-009, `wayray_wm_workspace_v1`). Each workspace is a container of windows. The key insight: **a workspace can mix local windows, federated foreign surfaces, and RDP client windows freely**.

```
┌─ Workspace 1: "My Desktop" ─────────────────────────────┐
│                                                           │
│  ┌─ foot ──────┐  ┌─ firefox ───────┐  ┌─ vscode ─────┐ │
│  │ (local)     │  │ (local)         │  │ (local)       │ │
│  └─────────────┘  └─────────────────┘  └───────────────┘ │
│                                                           │
├─ Workspace 2: "Customer B (WayRay)" ────────────────────┤
│                                                           │
│  ┌─ internal-tool ──────┐  ┌─ their-jira ──────────────┐ │
│  │ [Trusted: Customer B] │  │ [Trusted: Customer B]     │ │
│  │ (foreign surface via  │  │ (foreign surface via      │ │
│  │  WayRay federation)   │  │  WayRay federation)       │ │
│  └───────────────────────┘  └──────────────────────────┘ │
│                                                           │
├─ Workspace 3: "Customer C (Windows RDP)" ───────────────┤
│                                                           │
│  ┌─ Outlook ────────────┐  ┌─ Excel ──────────────────┐ │
│  │ [RDP: Customer C]     │  │ [RDP: Customer C]        │ │
│  │ (FreeRDP RAIL         │  │ (FreeRDP RAIL            │ │
│  │  seamless mode)       │  │  seamless mode)          │ │
│  └───────────────────────┘  └──────────────────────────┘ │
│                                                           │
├─ Workspace 4: "Customer D (VPN + Windows)" ─────────────┤
│                                                           │
│  ┌─ SAP GUI ────────────┐  ┌─ Remote Desktop ─────────┐ │
│  │ [RDP+VPN: Customer D] │  │ [RDP+VPN: Customer D]    │ │
│  │ (gateway: VPN tunnel  │  │ (gateway: VPN tunnel     │ │
│  │  + RDP + RAIL)        │  │  + RDP + full desktop)   │ │
│  └───────────────────────┘  └──────────────────────────┘ │
│                                                           │
└───────────────────────────────────────────────────────────┘
```

### Workspace Metadata

Workspaces carry metadata about their primary source, which the WM uses for trust indicators and grouping:

```rust
WorkspaceConfig {
    name: String,
    source: WorkspaceSource,
    trust_level: TrustLevel,
    // Visual: border color, background, badge
    decoration: WorkspaceDecoration,
}

enum WorkspaceSource {
    Local,
    Federated { server_id: String },
    RdpDirect { host: String },
    Gateway { gateway_id: String, target: String },
}
```

The WM can enforce policies per workspace: "Customer C workspace cannot read clipboard from My Desktop workspace" etc.

## RDP Integration: Three Tiers

### Tier 1: RDP Client as Local App (Works Today)

FreeRDP runs as a regular Wayland application on the WayRay server.

```
WayRay Server
  └─ freerdp /v:customer-c.rdp.example.com /u:jdoe
       └─ Renders Windows desktop into a Wayland surface
            └─ WayRay composites it like any window
```

**Pros:** Zero WayRay-specific work. FreeRDP is mature.
**Cons:** Entire Windows desktop in one window. Can't manage individual Windows apps via WM.

### Tier 2: FreeRDP RAIL/RemoteApp Mode (Seamless Windows Apps)

FreeRDP's RAIL (Remote Application Integrated Locally) mode requests individual application windows from the RDP server instead of a full desktop. Each Windows app becomes a separate Wayland surface.

```
WayRay Server
  └─ freerdp /v:customer-c.rdp.example.com /u:jdoe /app:outlook
       ├─ Outlook.exe → Wayland surface 1
       ├─ Excel.exe   → Wayland surface 2
       └─ Dialog box  → Wayland surface 3 (popup)
            └─ WayRay WM manages each as a separate window
```

**Pros:** Windows apps sit alongside Linux apps in the WM. Full tiling/floating control.
**Cons:** Requires Windows RemoteApp/RAIL configuration on the RDP server. Not all apps work well in RAIL mode.

**Implementation:** A thin wrapper around FreeRDP that:
1. Starts FreeRDP in RAIL mode with the Wayland backend
2. Registers each RAIL window as having origin metadata (`RDP: customer-c`)
3. Handles RAIL window lifecycle (new/close/resize) events
4. Optionally auto-starts configured apps

```toml
# ~/.config/wayray/connections/customer-c.toml

[connection]
name = "Customer C"
type = "rdp"
host = "rdp.customer-c.example.com"
username = "jdoe"
# Credentials via keyring, not config file
credential_store = "keyring"

[rdp]
mode = "rail"  # "desktop" for full desktop, "rail" for seamless apps
color_depth = 32
audio = true

[rdp.apps]
# Auto-start these apps when workspace is activated
autostart = ["outlook", "excel"]

[workspace]
name = "Customer C"
trust_level = "trusted"
# Restrict clipboard flow
clipboard = "bidirectional"  # or "to_remote_only", "from_remote_only", "disabled"
```

### Tier 3: Protocol Gateway (Future -- VPN + RDP + Seamless)

A WayRay **protocol gateway** that handles the entire connection lifecycle -- VPN establishment, RDP session start, and seamless window forwarding -- as a managed service. The user doesn't run FreeRDP manually; the gateway does it.

This is the most powerful tier and the one that transforms maintenance work.

## Protocol Gateway Architecture

### What It Is

A protocol gateway is a server-side service that:
1. Establishes network connectivity to a remote environment (VPN)
2. Starts a remote desktop session (RDP, VNC, or future protocols)
3. Translates remote window surfaces into WayRay foreign surfaces
4. Forwards them to the user's compositor session seamlessly

The user sees: "Connect to Customer D" → windows appear. The gateway handles VPN, authentication, RDP, and surface translation behind the scenes.

### Architecture

```
┌─ User's WayRay Session ────────────────────────────────────┐
│                                                             │
│  Local apps ──► Wayland surfaces                            │
│                                                             │
│  Gateway connector ──► foreign surfaces                     │
│    │                                                        │
│    │ ForeignWindow protocol (Unix socket or QUIC)           │
│    │                                                        │
│  ┌─┴────────────────────────────────────────────────────┐   │
│  │  Protocol Gateway Service (wrgw)            │   │
│  │                                                       │   │
│  │  ┌─────────────┐  ┌───────────────┐  ┌───────────┐  │   │
│  │  │ VPN Client  │  │ RDP Client    │  │ Surface   │  │   │
│  │  │             │  │ (FreeRDP      │  │ Translator│  │   │
│  │  │ WireGuard / │  │  library)     │  │           │  │   │
│  │  │ OpenVPN /   │  │               │  │ RDP RAIL  │  │   │
│  │  │ IPsec       │──►  RAIL mode   │──► windows   │  │   │
│  │  │             │  │               │  │ → Foreign │  │   │
│  │  │             │  │               │  │   Surface │  │   │
│  │  └──────┬──────┘  └───────────────┘  └─────┬─────┘  │   │
│  │         │                                   │        │   │
│  └─────────┼───────────────────────────────────┼────────┘   │
│            │                                   │            │
└────────────┼───────────────────────────────────┼────────────┘
             │ VPN tunnel                        │ ForeignWindow
             │                                   │ events
             v                                   v
   ┌──────────────────┐                ┌───────────────────┐
   │ Customer D       │                │ User's compositor  │
   │ Network          │                │ displays windows   │
   │                  │                │ with trust badges  │
   │ RDP Server       │                └───────────────────┘
   │ (Windows)        │
   └──────────────────┘
```

### Gateway Lifecycle

```
User action: "Connect to Customer D"
  │
  ├─ 1. Gateway reads connection profile
  │     (VPN config, RDP host, credentials from keyring)
  │
  ├─ 2. Gateway establishes VPN tunnel
  │     (WireGuard, OpenVPN, or IPsec -- configured per customer)
  │     VPN runs in an isolated network namespace/zone
  │
  ├─ 3. Gateway starts RDP session through VPN tunnel
  │     FreeRDP in RAIL mode connects to customer's RDP server
  │     Authenticates with stored/delegated credentials
  │
  ├─ 4. Gateway translates RAIL windows to ForeignWindow events
  │     Each Windows app window → ForeignWindowAnnounce
  │     Frame updates → ForeignWindowUpdate
  │     Input from user → forwarded to RDP session
  │
  ├─ 5. User's compositor displays windows with trust indicators
  │     [RDP+VPN: Customer D] badges on each window
  │     WM places them in the configured workspace
  │
  └─ User action: "Disconnect Customer D"
       Gateway tears down: RDP session → VPN tunnel → cleanup
```

### Gateway Connection Profiles

```toml
# ~/.config/wayray/gateways/customer-d.toml

[gateway]
name = "Customer D"
description = "Customer D maintenance access"

[vpn]
type = "wireguard"  # or "openvpn", "ipsec"
config = "/etc/wayray/vpn/customer-d.conf"
# VPN credentials
credential_store = "keyring"
# Network isolation: VPN traffic stays in its own namespace
# Customer D traffic cannot reach other gateways or local network
isolate = true

[rdp]
host = "10.200.1.50"  # Address within VPN
port = 3389
username = "maintenance-jdoe"
credential_store = "keyring"
mode = "rail"
# Or "desktop" for full desktop in one window

[rdp.apps]
# Published RemoteApp programs on the RDP server
available = ["sap-gui", "monitoring-console", "remote-desktop"]
autostart = ["monitoring-console"]

[security]
trust_level = "trusted"
# Clipboard policy
clipboard = "to_remote_only"  # Can paste INTO customer env, not OUT
# File transfer
file_transfer = "disabled"
# Screen capture of gateway windows
screen_capture = "disabled"

[workspace]
name = "Customer D"
# Auto-create workspace when gateway connects
auto_workspace = true
```

### Gateway Management via wradm

```bash
# List configured gateways
wradm gateway list

# Connect to a gateway
wradm gateway connect customer-d

# Status of active gateways
wradm gateway status
# customer-d: connected (VPN: up, RDP: 3 windows active)
# customer-b: connected (WayRay federation: 2 apps)

# Disconnect
wradm gateway disconnect customer-d

# Import a gateway profile (shared by team lead / IT admin)
wradm gateway import customer-d.toml
```

### Network Isolation

Each gateway runs its VPN in an **isolated network namespace** (Linux) or **zone** (illumos):

```
┌─ Main network namespace ───────────────────┐
│  User's session, local apps                 │
│  WayRay QUIC to client                      │
│  Federation connections                     │
│                                             │
│  ┌─ VPN namespace: customer-d ───────────┐ │
│  │  WireGuard tunnel to 10.200.0.0/16    │ │
│  │  FreeRDP connects to 10.200.1.50      │ │
│  │  NO access to main network            │ │
│  │  NO access to other VPN namespaces    │ │
│  └───────────────────────────────────────┘ │
│                                             │
│  ┌─ VPN namespace: customer-e ───────────┐ │
│  │  OpenVPN tunnel to 172.16.0.0/12      │ │
│  │  FreeRDP connects to 172.16.5.20      │ │
│  │  Completely isolated from customer-d   │ │
│  └───────────────────────────────────────┘ │
└─────────────────────────────────────────────┘
```

This prevents a compromised customer environment from pivoting to other customers or the user's own network. Each VPN tunnel is hermetically sealed.

### Security Policies Per Gateway

| Policy | Purpose | Example |
|--------|---------|---------|
| `clipboard` | Control clipboard flow direction | `to_remote_only`: paste in, never copy out |
| `file_transfer` | Allow/deny file drag-and-drop | `disabled` for sensitive customers |
| `screen_capture` | Can other windows screenshot this? | `disabled` for classified environments |
| `audio` | Audio forwarding through RDP | `enabled` or `disabled` |
| `usb` | USB device forwarding through RDP | `disabled` (security risk over VPN) |
| `idle_timeout` | Auto-disconnect after inactivity | `30m` for maintenance windows |
| `session_recording` | Record gateway session for audit | `enabled` for compliance |

### Future Protocol Support

The gateway architecture is protocol-agnostic. The surface translator is a trait:

```rust
trait RemoteProtocolAdapter {
    /// Establish connection to remote environment
    fn connect(&mut self, config: &ConnectionConfig) -> Result<()>;

    /// Get the next window event (new window, update, close)
    fn next_event(&mut self) -> Option<ForeignWindowEvent>;

    /// Forward input to the remote session
    fn send_input(&mut self, window_id: u64, event: InputEvent);

    /// Disconnect and clean up
    fn disconnect(&mut self);
}
```

Planned adapters:

| Adapter | Protocol | Source | Use Case |
|---------|----------|--------|----------|
| `RdpAdapter` | RDP (FreeRDP) | Windows servers | Most enterprise/maintenance |
| `VncAdapter` | VNC/RFB | Linux/legacy systems | Older infrastructure |
| `WayRayAdapter` | WayRay federation | WayRay servers | B2B, cross-org (ADR-014) |
| `SpiceAdapter` | SPICE | libvirt/QEMU VMs | Virtual machine access |
| `SshXAdapter` | SSH + X11 forwarding | Any Unix host | Legacy X11 apps on remote hosts |

Each adapter translates the remote protocol's window/surface model into `ForeignWindowEvent`s that the compositor understands.

## The Maintenance Engineer's Day

Putting it all together:

```
08:00 - Arrive at office, phone on charging pad
        → Session resumes on office terminal
        → Workspace 1: "My Desktop" with email, chat, docs

09:00 - Customer B maintenance window
        → wradm gateway connect customer-b
        → VPN tunnel establishes automatically
        → Workspace 2 appears: "Customer B"
        → SAP GUI and monitoring console open seamlessly
        → Fix the issue, close the ticket in customer's Jira

10:30 - Disconnect Customer B
        → wradm gateway disconnect customer-b
        → VPN torn down, workspace closes
        → Back to Workspace 1

11:00 - Customer C meeting (they use WayRay too)
        → Federation auto-connects (pre-configured trust)
        → Workspace 3: "Customer C" shows their shared dashboard
        → Collaborate on shared app alongside your own tools

13:00 - Lunch, pick up phone
        → Session suspends

13:30 - Back, phone on pad
        → Session resumes, all workspaces intact
        → Gateway connections still active (VPN maintained by server)

14:00 - Visit Customer D on-site
        → Sit at their conference room terminal
        → Phone detected, connects to YOUR server over internet
        → Your full desktop with all workspaces appears
        → Customer D gateway already connected (VPN from your server)
        → Work on their systems from their conference room
        → Their terminal is just a screen, your server does everything

17:00 - Head home, phone leaves proximity
        → Session suspends
        → All gateway VPNs maintained (reconnect instantly tomorrow)
```

## Relationship to Existing ADRs

| ADR | Relationship |
|-----|-------------|
| ADR-009 (Pluggable WM) | Workspaces managed by WM, per-workspace trust policies |
| ADR-013 (Phone Proximity) | Phone triggers session, carries server address for remote |
| ADR-014 (Federation) | WayRay-to-WayRay federation is one gateway adapter type |
| ADR-012 (Cloud Auth) | Gateway credentials can use OIDC delegation |

## Rationale

- **Maintenance work is inherently multi-environment**: consultants, MSPs, and IT teams work across many customer environments daily. Making this seamless is a genuine productivity win.
- **VPN + RDP is table stakes**: most customer environments require VPN access to reach their RDP servers. Automating VPN setup removes friction.
- **Network isolation is non-negotiable**: customer VPN tunnels must be hermetically sealed from each other. A compromised customer network must not be able to reach other customers or the user's own environment.
- **Gateway as managed service**: the user says "connect to Customer D", not "start WireGuard, then open FreeRDP, then configure RAIL mode". The gateway handles the mechanics.
- **Protocol-agnostic adapter trait**: the world isn't all RDP. VNC, SPICE, SSH X11 forwarding, and WayRay federation are all valid sources. One gateway, many protocols.
- **Session persistence across disconnects**: gateway connections (and their VPN tunnels) survive session suspend/resume. Pick up your phone, walk to another terminal, gateways are still connected.

## Consequences

- Gateway service adds significant complexity (VPN management, RDP session lifecycle, error handling)
- Must bundle or depend on VPN clients (WireGuard tools, OpenVPN)
- Must bundle or depend on FreeRDP library (libfreerdp)
- RAIL mode depends on customer's RDP server being configured for RemoteApp
- VPN credentials management needs careful security design (keyring integration, no plaintext configs)
- Network namespace/zone management requires elevated privileges
- Session recording for audit compliance adds storage and privacy considerations
- Each gateway adapter is a separate maintenance burden
- Gateway profiles shared across teams need a distribution mechanism (IT admin tooling)

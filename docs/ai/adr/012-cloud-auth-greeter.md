# ADR-012: Cloud Authentication via OAuth/OIDC Greeter

## Status
Accepted

## Context
In cloud and enterprise deployments, users authenticate against a centralized Identity Provider (IdP) rather than local `/etc/passwd` accounts. The WayRay greeter (ADR-010) must support OAuth 2.0 / OpenID Connect flows to bridge cloud identity to a local user context (PAM session, home dir, etc.).

The greeter is the natural integration point -- it runs before any user context exists, talks to the IdP, and triggers local user provisioning + PAM session creation.

## The Authentication Flow

### Cloud/Enterprise Login

```
┌──────────┐     token        ┌──────────────┐
│  Client   │────────────────►│  WayRay       │
│  (viewer) │                 │  Server       │
└──────────┘                 └──────┬────────┘
                                    │
                           No session for token
                                    │
                                    v
                        ┌───────────────────────┐
                        │   Session Launcher     │
                        │   (pre-auth context)   │
                        └───────────┬───────────┘
                                    │
                          Start greeter in ephemeral
                          session (no user yet)
                                    │
                                    v
                        ┌───────────────────────┐
                        │   Greeter (Wayland)    │
                        │                        │
                        │  ┌──────────────────┐  │
                        │  │  Login Method     │  │
                        │  │  Selection        │  │
                        │  │                   │  │
                        │  │  [Local Login]    │  │
                        │  │  [SSO / OAuth]    │  │
                        │  │  [Smart Card]     │  │
                        │  └──────────────────┘  │
                        └───────────┬───────────┘
                                    │
                          User selects SSO / OAuth
                                    │
                         ┌──────────┴──────────┐
                         │                     │
                    Full browser           Device code
                    (embedded webview)     (phone/external)
                         │                     │
                         v                     v
                    ┌─────────┐          ┌─────────────┐
                    │  IdP    │          │  IdP         │
                    │  Login  │          │  Device Code │
                    │  Page   │          │  + QR Code   │
                    └────┬────┘          └──────┬──────┘
                         │                      │
                         └──────────┬───────────┘
                                    │
                            IdP returns:
                            - ID Token (JWT)
                            - Access Token
                            - UserInfo claims
                                    │
                                    v
                        ┌───────────────────────┐
                        │   Greeter resolves     │
                        │   cloud identity to    │
                        │   local user           │
                        │                        │
                        │   Claims mapping:      │
                        │   sub/email → uid      │
                        │   groups → gids        │
                        └───────────┬───────────┘
                                    │
                                    v
                        ┌───────────────────────┐
                        │   Session Launcher     │
                        │                        │
                        │   1. Provision user     │
                        │      (if first login)  │
                        │   2. PAM session open   │
                        │   3. Mount home dir     │
                        │   4. Set env vars       │
                        │   5. Start user session │
                        └───────────────────────┘
```

### Pre-Auth Ephemeral Session

The greeter itself needs a Wayland session to display the login UI, but no user is authenticated yet. The session launcher creates a minimal **ephemeral session** running as a service user (e.g., `wayray-greeter`):
- Limited compositor session (no user apps, no shell access)
- Only the greeter client is allowed to connect
- Destroyed after authentication succeeds or times out

This is analogous to GDM running as the `gdm` user before any human logs in.

## OAuth 2.0 / OIDC Flows

### Flow 1: Authorization Code (Full Browser)

Best when the greeter can embed or launch a browser window:

1. Greeter generates PKCE challenge
2. Opens IdP authorization URL in embedded webview or spawned browser
3. User authenticates at IdP (password, MFA, passkey, etc.)
4. IdP redirects to local callback (`http://localhost:<port>/callback`)
5. Greeter exchanges authorization code for tokens
6. Greeter validates ID token, extracts claims

**Pros:** Full IdP experience (MFA, conditional access, passkeys)
**Cons:** Needs a browser/webview on the server. Thin client greeter rendering a remote browser is recursive (browser in compositor in client).

### Flow 2: Device Authorization Grant (RFC 8628) -- Recommended

Best for thin clients where the greeter UI is minimal:

1. Greeter requests device code from IdP: `POST /device/authorize`
2. IdP returns: `device_code`, `user_code`, `verification_uri`
3. Greeter displays:
   ```
   ┌─────────────────────────────────┐
   │                                 │
   │   Sign in with your phone:     │
   │                                 │
   │   1. Go to: idp.example.com/   │
   │      device                    │
   │                                 │
   │   2. Enter code: ABCD-1234     │
   │                                 │
   │   ┌─────────────────────────┐  │
   │   │      [QR CODE]          │  │
   │   │                         │  │
   │   └─────────────────────────┘  │
   │                                 │
   │   Waiting for authorization... │
   │                                 │
   └─────────────────────────────────┘
   ```
4. User scans QR code or types URL on their phone/laptop
5. User authenticates on their own device (full browser, MFA, etc.)
6. Greeter polls IdP: `POST /token` with `device_code`
7. Once authorized, IdP returns tokens
8. Greeter validates ID token, extracts claims

**Pros:** No browser needed on server. User authenticates on their own device with full capabilities. Perfect for thin clients. Works with any MFA method.
**Cons:** Requires user to have a second device (phone). Polling adds a few seconds.

### Flow 3: Resource Owner Password (Legacy)

Direct username/password POST to IdP token endpoint. Simple but:
- No MFA support
- IdP must enable this flow (many disable it)
- Only for legacy/migration scenarios

## Identity-to-User Mapping

The greeter/session-launcher must map IdP claims to a local user:

### Claim Mapping Configuration

```toml
# /etc/wayray/auth.toml

[oidc]
issuer = "https://idp.example.com"
client_id = "wayray-server"
# client_secret via env var or file reference
client_secret_file = "/etc/wayray/oidc-secret"

# Which flow to use
flow = "device_code"  # or "authorization_code"

# Scopes to request
scopes = ["openid", "profile", "email", "groups"]

[mapping]
# How to derive the local username from IdP claims
username_claim = "preferred_username"
# Fallback: derive from email (strip @domain)
username_fallback = "email_local_part"

# Group mapping: IdP group claim -> local groups
groups_claim = "groups"

[mapping.group_map]
"cloud-admins" = "wheel"
"developers" = "staff"
"users" = "users"

[provisioning]
# Auto-create local user on first login?
auto_provision = true
# Home directory template
home_template = "/export/home/{username}"
# Default shell
default_shell = "/usr/bin/bash"
# Default groups for new users
default_groups = ["users"]
# Create home dir via
home_create = "zfs"  # "zfs" (create ZFS dataset) or "mkdir" or "autofs"

[provisioning.zfs]
# ZFS dataset for home dirs
parent_dataset = "rpool/export/home"
quota = "10G"
```

### Provisioning Flow (First Login)

1. IdP claims arrive: `sub=abc123, preferred_username=jdoe, email=jdoe@corp.com, groups=[developers]`
2. Session launcher checks if local user `jdoe` exists
3. If not: create user via `useradd` (or illumos equivalent), create home dir (ZFS dataset), set groups
4. Open PAM session for `jdoe`
5. Start desktop session

### Subsequent Logins

1. IdP claims arrive
2. Local user exists → update group memberships if changed
3. Open PAM session
4. Resume or create desktop session

## PAM Integration

The session launcher uses PAM, not the greeter directly. Two PAM paths:

### Path A: `pam_oidc` module
A PAM module that validates OIDC tokens. The greeter passes the ID token to the session launcher, which passes it to PAM. PAM validates the token signature, checks claims, and opens the session.

### Path B: PAM bypass with pre-validated token
The greeter validates the OIDC token itself (signature, issuer, audience, expiry). It then tells the session launcher "user `jdoe` is authenticated" and the session launcher opens a PAM session using `pam_permit` or a custom module that trusts the greeter's assertion. The token validation happens in the greeter, not in PAM.

**Recommendation:** Path B for simplicity. The greeter is a trusted component (runs on the server). Let it handle OIDC complexity. PAM handles session setup (limits, env, mount).

## Greeter as Integration Tool

The greeter's role expands in cloud deployments:

| Responsibility | Local Mode | Cloud Mode |
|---------------|-----------|------------|
| Display login UI | Username + password form | Device code + QR, or browser |
| Authenticate | PAM directly | OAuth/OIDC with IdP |
| Identity mapping | N/A (local user) | Claims → local user |
| User provisioning | N/A (pre-existing) | Auto-create on first login |
| Session creation | Tell launcher "user X" | Tell launcher "user X" (same) |

The session launcher interface stays the same either way:
```
Greeter → Session Launcher: "authenticated user: jdoe, uid: 1001"
```

What changes is how the greeter *arrives* at that identity.

## Greeter Plugin Architecture

Rather than hardcoding auth methods, the greeter supports **auth plugins**:

```
wayray-greeter
  ├── auth-local        (username + password → PAM)
  ├── auth-oidc         (OAuth/OIDC device code or auth code)
  ├── auth-smartcard    (PC/SC → certificate → PAM or IdP)
  └── auth-kerberos    (kinit-style → PAM)
```

The greeter UI presents available methods based on configuration. Each plugin implements:

```rust
trait AuthPlugin {
    /// Display name for the login screen
    fn display_name(&self) -> &str;

    /// Start the authentication flow, returning UI elements to render
    fn start(&mut self) -> AuthUI;

    /// Poll for completion (for async flows like device code)
    fn poll(&mut self) -> AuthState;

    /// Returns the authenticated identity on success
    fn identity(&self) -> Option<AuthenticatedUser>;
}

enum AuthState {
    Pending,
    NeedsInput(Vec<AuthField>),  // username, password, PIN, etc.
    Success(AuthenticatedUser),
    Failed(String),
}

struct AuthenticatedUser {
    username: String,
    uid: Option<u32>,          // if already resolved
    groups: Vec<String>,
    token: Option<String>,     // OIDC token for session launcher
    claims: HashMap<String, String>,
}
```

## Rationale

- **Device code flow is ideal for thin clients**: No browser on server, user authenticates on their own device with full MFA support. QR code makes it frictionless.
- **Greeter as integration point**: It already exists (ADR-010), it runs pre-auth, it has a UI. Adding OIDC is a natural extension, not a new component.
- **Plugin architecture**: Different deployments need different auth. Enterprise wants OIDC + smart card. University wants LDAP. Home user wants password. Don't force one choice.
- **Auto-provisioning**: Cloud users shouldn't need a sysadmin to create their Unix account. Map claims to user, create on first login.
- **Session launcher interface unchanged**: `Greeter → "user X authenticated" → Launcher`. The launcher doesn't care HOW authentication happened.

## Consequences

- Greeter becomes more complex (OIDC client library, QR code rendering, polling)
- Need an OIDC client library in Rust (openidconnect crate)
- Device code flow adds ~5-15 seconds to first login (user must authenticate on phone)
- Auto-provisioning needs careful security review (who can get an account?)
- Group mapping configuration can be complex for large orgs
- Token refresh: long-running sessions may need token refresh for access to IdP-gated resources
- The ephemeral pre-auth session is a new concept that needs careful lifecycle management

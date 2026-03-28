# ADR-013: Smartphone as Proximity Token

## Status
Accepted

## Context
SunRay's session mobility was driven by smart card insertion/removal. WayRay supports pluggable tokens (ADR-004). A smartphone is the one device users always carry. If the phone can act as a proximity token, we get automatic session follow without any explicit action -- walk up to a terminal, your desktop appears; walk away, it suspends.

This maps directly to SunRay's smart card semantics:
- Smart card insert → phone enters proximity range
- Smart card remove → phone leaves proximity range

But better: no physical insertion, works from your pocket.

## Technology Options

### BLE (Bluetooth Low Energy) -- Recommended

The phone runs a small companion app that broadcasts a BLE advertisement containing a session token. The WayRay client has a BLE receiver that detects nearby phones.

**How BLE beacons work:**
- Phone advertises a BLE beacon with a service UUID specific to WayRay
- The advertisement payload contains an encrypted session token (≤31 bytes in legacy advertising, ≤255 bytes in extended)
- The client scans for WayRay beacons and reads the token
- RSSI (signal strength) determines proximity -- configurable threshold
- When RSSI drops below threshold (user walked away), trigger disconnect

**Advantages:**
- Always-on: phone advertises in background, no user action needed
- Works from pocket (no need to pull phone out)
- Range tunable via RSSI threshold (1-10 meters typical)
- Low power: BLE advertising uses ~1-5% battery per day
- Universal: every modern smartphone has BLE
- Works through walls at close range (meeting rooms)

**BLE Token Flow:**
```
Phone (companion app):
  1. User authenticates in app once (OIDC, biometric, etc.)
  2. App receives signed session token from IdP/server
  3. App begins BLE advertising:
     Service UUID: WayRay-specific
     Payload: encrypted(token_id + timestamp + HMAC)
  4. Rotates payload periodically (replay prevention)

WayRay Client (BLE scanner):
  1. Continuously scans for WayRay service UUID
  2. Detects beacon → reads token → validates HMAC + timestamp
  3. If new token in range: trigger session attach
  4. If token leaves range (RSSI below threshold for N seconds):
     trigger session detach
  5. If multiple tokens: nearest (highest RSSI) wins
```

### NFC -- Complementary (and Charging Pad Mode)

Phone tap on NFC reader for explicit authentication:
- Quick deliberate action (tap to connect)
- Works as fallback when BLE is disabled
- Can trigger initial token provisioning
- Very short range (~4cm) -- no proximity tracking, clean binary signal

**Wireless charging pad as card reader:** In high-security or dense office deployments, the client's NFC reader can be embedded in (or placed next to) a Qi wireless charging pad. The phone on the pad is simultaneously charging and authenticating -- exactly mimicking a smart card in a reader slot. NFC provides the crisp insert/remove semantics (present = attached, lifted = detached) without RSSI ambiguity. The companion app responds to NFC with the session token via HCE (Host Card Emulation on Android) or Core NFC (iOS).

**Combined NFC + BLE mode:** For deployments that want both:
- NFC: immediate attach when phone placed on pad (desk-distance, deliberate)
- BLE: sustained heartbeat confirming phone is still present (survives brief NFC interrupts)
- NFC absence + BLE absence: detach (phone physically removed from area)

This gives precise control -- some offices want centimeter-range "phone on pad" behavior, others want meter-range "phone in pocket" behavior. Same `TokenProvider` trait, deployment configuration picks the mode:

```toml
# /etc/wayray/token.toml

[proximity]
# "ble" = walk-up proximity (meter range)
# "nfc" = charging pad / tap (centimeter range)
# "ble+nfc" = NFC for attach, BLE for heartbeat
# "ble|nfc" = either can attach independently
mode = "nfc"

[ble]
rssi_threshold = -70    # dBm
attach_delay = 2        # seconds
detach_delay = 10       # seconds

[nfc]
# No thresholds needed -- NFC is binary (present/absent)
detach_delay = 3        # seconds grace period for brief lifts
```

### UWB (Ultra-Wideband) -- Future

Precise distance measurement (10cm accuracy):
- iPhone U1/U2 chip, some Samsung/Google phones
- Could enable "desk assignment" -- know exactly which terminal you're closest to
- Not yet universal enough to depend on
- Consider as enhancement when hardware penetration increases

### WiFi Proximity (mDNS) -- Fallback

Phone app announces presence on local network:
- Works without BLE hardware on client
- Coarse proximity (same VLAN/subnet)
- Higher latency (mDNS discovery takes seconds)
- Can't distinguish between terminals in the same room
- Useful as a fallback when BLE isn't available

## Decision

**BLE as primary proximity mechanism, NFC as explicit-action complement, WiFi/mDNS as software-only fallback.**

### Proximity State Machine

```
                          BLE beacon detected
                          (RSSI > threshold)
    [No Phone] ─────────────────────────────────> [Detected]
                                                      │
                                                      │ stable for
                                                      │ T_attach seconds
                                                      v
                    ┌──────────────────────────── [Attached]
                    │                                  │
                    │  BLE beacon returns              │ RSSI < threshold
                    │  (RSSI > threshold)              │ for T_detach seconds
                    │                                  v
                    └──────────────────────────── [Detaching]
                                                      │
                                                      │ timeout expires
                                                      v
                                                 [Detached]
                                                      │
                                                      │ (session suspends)
                                                      v
                                                 [No Phone]
```

**Timers prevent flapping:**
- `T_attach`: delay before attaching (default: 2 seconds). Prevents drive-by session grabs when walking past a terminal.
- `T_detach`: delay before detaching (default: 10 seconds). Prevents session drop when phone briefly loses signal (body shielding, phone rotates in pocket).
- Both configurable per-deployment.

### Security Considerations

**Replay attacks:** Token payload includes a timestamp and HMAC. Client rejects tokens older than N seconds. Phone rotates payload every 30 seconds.

**Relay attacks:** An attacker could relay the BLE signal from a distant phone to a nearby client. Mitigations:
- Token payload includes a challenge-response nonce (requires phone app to respond)
- RSSI-based distance bounding (relayed signals have abnormal RSSI patterns)
- Optional: require NFC tap for initial session attachment, BLE only for persistence
- For high-security deployments: disable BLE proximity, use smart card only

**Token theft:** If someone clones the BLE advertisement, they get the session token. Mitigations:
- Token rotation (new token every 30s, phone signs each one)
- Mutual authentication: client challenges phone via BLE GATT connection
- Binding token to phone's hardware attestation key (Android SafetyNet / iOS DeviceCheck)

**Multi-phone scenarios:** When multiple phones are near a terminal:
- Highest RSSI wins (closest phone)
- If tie: first-arrived wins
- Explicit NFC tap overrides BLE proximity (deliberate action beats passive detection)

### Hardware Requirements

**Client side:**
- BLE 4.0+ receiver (USB dongle or built-in)
- Optional: NFC reader (USB)
- Commodity hardware: USB BLE dongles cost ~$5-10

**Phone side:**
- Companion app (Android + iOS)
- BLE 4.0+ (every phone since ~2013)
- Background execution permission for BLE advertising

### Implementation as Auth Plugin

```rust
struct BleProximityPlugin {
    scanner: BleScanner,
    known_tokens: HashMap<TokenId, ProximityState>,
    rssi_threshold: i8,     // e.g., -70 dBm
    attach_delay: Duration, // e.g., 2 seconds
    detach_delay: Duration, // e.g., 10 seconds
}

impl TokenProvider for BleProximityPlugin {
    fn watch(&self) -> TokenEventStream {
        // Emits:
        // TokenInserted(token_id) -- phone entered proximity
        // TokenRemoved(token_id)  -- phone left proximity
    }
}
```

This implements the same `TokenProvider` trait as smart cards (ADR-004). The session management layer doesn't know or care whether the token came from a smart card slot or a BLE beacon.

### Companion App Scope

The phone app is intentionally minimal:
1. One-time setup: authenticate with IdP (OIDC), receive signing key
2. Configure home server address (e.g., `home.wayray.example.com:4433`)
3. Background service: broadcast BLE beacon with rotating signed token
4. Optional: respond to GATT challenges for mutual auth
5. Optional: show notification when session attaches/detaches
6. No remote desktop functionality -- the phone is a **key**, not a viewer

The BLE beacon payload includes the user's server address alongside the session token. When a WayRay client detects the phone, it knows both WHO the user is AND where their server lives. This enables direct remote access (ADR-014): sit at any terminal anywhere in the world, your phone tells it where to find your desktop.

```
BLE Payload (encrypted):
  token_id:  "abc123"
  server:    "home.wayray.example.com:4433"
  timestamp: 1743206400
  hmac:      <signature>
```

Platform:
- Android: foreground service with BLE advertising
- iOS: Core Bluetooth peripheral mode (works in background with limitations)
- Could be a PWA using Web Bluetooth (limited background support)

## Rationale

- **Zero-friction session mobility**: walk up, session appears. Walk away, session suspends. No card to insert, no button to press, no QR to scan.
- **Users already carry phones**: unlike smart cards which are an additional device to manage and can be forgotten
- **Maps to SunRay semantics**: insert/remove maps to enter/leave proximity. Same session management, different physical mechanism.
- **Pluggable**: implements `TokenProvider` trait. Composable with other token types. Smart card overrides BLE on explicit insertion.
- **Tunable security/convenience tradeoff**: high security deployments add NFC tap requirement or disable BLE entirely. Casual deployments use pure proximity.

## Consequences

- Requires BLE hardware on client devices (USB dongle if not built-in)
- Must develop and maintain companion apps for Android and iOS
- BLE scanning has power implications on battery-powered clients
- RSSI is noisy and affected by environment (walls, bodies, interference). Threshold tuning is deployment-specific.
- iOS background BLE advertising has limitations (Apple throttles frequency)
- Must handle edge cases: phone in adjacent room, phone dies mid-session, multiple phones
- Privacy consideration: BLE beacons are detectable by nearby devices. Token payload must be encrypted so only WayRay clients can read the session token.

//! Certificate-pinning primitives shared by the QUIC client and server.
//!
//! WayRay servers use self-signed certificates (there is no central CA in the
//! SunRay-style deployment model). Standard WebPKI verification therefore does
//! not apply, and *accepting any certificate* leaves the wire open to a
//! man-in-the-middle who can harvest the session token (the sole credential).
//!
//! Instead we pin certificates by their SHA-256 fingerprint, using two
//! complementary mechanisms:
//!
//! 1. **Configured pins** — a deployment may record each server's fingerprint
//!    in `cluster.toml` (`fingerprint = "sha256:…"`). When a pin is known for a
//!    target, it is enforced strictly: a mismatch aborts the handshake.
//! 2. **Trust on first use (TOFU)** — when no pin is configured, the first
//!    fingerprint seen for a peer is recorded in a `known_hosts`-style file and
//!    enforced on every later connection (the SSH model). This closes the
//!    silent-MITM hole: an attacker cannot transparently impersonate a server
//!    that the client has already contacted.
//!
//! This module is deliberately free of any TLS-library types so it compiles
//! identically on illumos and Linux; the thin `rustls` verifier that calls into
//! it lives in each binary's network layer.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Compute the pin string for a DER-encoded certificate: `"sha256:<hex>"`.
pub fn fingerprint(cert_der: &[u8]) -> String {
    let digest = Sha256::digest(cert_der);
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Normalize a user-supplied fingerprint for comparison: lower-cased, with any
/// `sha256:` prefix and internal `:`/whitespace separators removed so that
/// `AA:BB`, `sha256:aabb`, and `aabb` all compare equal.
pub fn normalize_fingerprint(fp: &str) -> String {
    let fp = fp.trim();
    let fp = fp.strip_prefix("sha256:").unwrap_or(fp);
    fp.chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Whether a presented certificate fingerprint matches an expected pin,
/// tolerating formatting differences (prefix, colons, case).
pub fn fingerprints_match(presented: &str, expected: &str) -> bool {
    normalize_fingerprint(presented) == normalize_fingerprint(expected)
}

/// Outcome of consulting the TOFU pin store for a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinOutcome {
    /// The peer was already pinned and the fingerprint matched.
    Match,
    /// The peer was not previously known; the fingerprint has now been recorded.
    FirstUse,
    /// The peer was pinned to a *different* fingerprint — a possible MITM.
    Mismatch {
        /// Fingerprint recorded on a previous connection.
        expected: String,
        /// Fingerprint presented now.
        presented: String,
    },
}

/// SSH-style `known_hosts` store mapping a peer identity (host:port or server
/// name) to the certificate fingerprint first seen for it.
///
/// The backing file is created with `0600` permissions: it records which server
/// identities the user trusts and must not be world-writable.
#[derive(Debug)]
pub struct PinStore {
    path: PathBuf,
    pins: HashMap<String, String>,
}

impl PinStore {
    /// Open (or start) the pin store at the default location
    /// (`$XDG_CONFIG_HOME/wayray/known_hosts`, falling back to
    /// `$HOME/.config/...`). A missing file is treated as an empty store.
    pub fn open_default() -> io::Result<Self> {
        Self::open(default_known_hosts_path())
    }

    /// Open (or start) the pin store backed by `path`.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let pins = match std::fs::read_to_string(&path) {
            Ok(contents) => parse_known_hosts(&contents),
            Err(e) if e.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self { path, pins })
    }

    /// The recorded fingerprint for `identity`, if any.
    pub fn get(&self, identity: &str) -> Option<&str> {
        self.pins.get(identity).map(String::as_str)
    }

    /// Verify a presented fingerprint against the store, recording it on first
    /// use. Persists the file (mode `0600`) when a new pin is added.
    pub fn verify_or_remember(
        &mut self,
        identity: &str,
        presented: &str,
    ) -> io::Result<PinOutcome> {
        match self.pins.get(identity) {
            Some(expected) => {
                if fingerprints_match(presented, expected) {
                    Ok(PinOutcome::Match)
                } else {
                    Ok(PinOutcome::Mismatch {
                        expected: expected.clone(),
                        presented: presented.to_string(),
                    })
                }
            }
            None => {
                self.pins
                    .insert(identity.to_string(), presented.to_string());
                self.persist()?;
                Ok(PinOutcome::FirstUse)
            }
        }
    }

    /// Write the store back to disk with owner-only (`0600`) permissions.
    fn persist(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut body = String::new();
        let mut entries: Vec<(&String, &String)> = self.pins.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (identity, fp) in entries {
            body.push_str(identity);
            body.push(' ');
            body.push_str(fp);
            body.push('\n');
        }
        write_private(&self.path, body.as_bytes())
    }
}

/// Parse a `known_hosts` file: one `identity sha256:hex` pair per line.
fn parse_known_hosts(contents: &str) -> HashMap<String, String> {
    let mut pins = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((identity, fp)) = line.split_once(char::is_whitespace) {
            pins.insert(identity.trim().to_string(), fp.trim().to_string());
        }
    }
    pins
}

/// Default `known_hosts` location for client/peer pins.
pub fn default_known_hosts_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("wayray").join("known_hosts")
}

/// Write `data` to `path`, creating it with `0600` (owner read/write only) on
/// Unix so credential-adjacent files are never world-readable.
pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        let mut file = opts.open(path)?;
        use std::io::Write;
        file.write_all(data)?;
        // Tighten permissions even if the file pre-existed with a looser mode.
        let perms = std::os::unix::fs::PermissionsExt::from_mode(0o600);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        std::fs::write(path, data)
    }
}

/// A `rustls` certificate verifier that pins servers by SHA-256 fingerprint
/// instead of trusting a CA chain, and still performs real handshake-signature
/// verification (so a pinned fingerprint cannot be replayed with a forged key).
///
/// Available with the `quic-tls` feature.
#[cfg(feature = "quic-tls")]
pub mod verifier {
    use std::sync::{Arc, Mutex};

    use rustls::DigitallySignedStruct;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    use super::{PinOutcome, PinStore, fingerprint, fingerprints_match};

    /// How a server certificate is authenticated.
    #[derive(Debug)]
    pub enum PinPolicy {
        /// Enforce exactly this fingerprint (a configured pin). A mismatch
        /// aborts the handshake.
        Fixed(String),
        /// Trust-on-first-use: record the first fingerprint seen for `identity`
        /// in the shared store, then require it to match thereafter.
        Tofu {
            /// Stable peer identity (e.g. `host:port`) used as the store key.
            identity: String,
            /// Shared known-hosts store (wrapped for interior mutability across
            /// the `Arc`'d verifier).
            store: Arc<Mutex<PinStore>>,
        },
    }

    /// Pinning verifier. Construct with [`PinnedServerCertVerifier::new`] and
    /// install via `rustls::ClientConfig::dangerous().with_custom_certificate_verifier`.
    #[derive(Debug)]
    pub struct PinnedServerCertVerifier {
        provider: Arc<CryptoProvider>,
        policy: PinPolicy,
    }

    impl PinnedServerCertVerifier {
        /// Build a verifier using the ring crypto provider for signature checks.
        pub fn new(policy: PinPolicy) -> Self {
            Self {
                provider: Arc::new(rustls::crypto::ring::default_provider()),
                policy,
            }
        }

        fn check_pin(&self, end_entity: &CertificateDer<'_>) -> Result<(), rustls::Error> {
            let presented = fingerprint(end_entity.as_ref());
            match &self.policy {
                PinPolicy::Fixed(expected) => {
                    if fingerprints_match(&presented, expected) {
                        Ok(())
                    } else {
                        Err(rustls::Error::General(format!(
                            "server certificate fingerprint {presented} does not match the configured pin {expected} (possible man-in-the-middle)"
                        )))
                    }
                }
                PinPolicy::Tofu { identity, store } => {
                    let mut store = store
                        .lock()
                        .map_err(|_| rustls::Error::General("pin store lock poisoned".into()))?;
                    match store.verify_or_remember(identity, &presented) {
                        Ok(PinOutcome::Match) => Ok(()),
                        Ok(PinOutcome::FirstUse) => {
                            tracing::warn!(
                                %identity,
                                fingerprint = %presented,
                                "trusting server certificate on first use; pin recorded in known_hosts"
                            );
                            Ok(())
                        }
                        Ok(PinOutcome::Mismatch {
                            expected,
                            presented,
                        }) => Err(rustls::Error::General(format!(
                            "server {identity} presented certificate {presented} but is pinned to {expected} (possible man-in-the-middle)"
                        ))),
                        Err(e) => Err(rustls::Error::General(format!("pin store error: {e}"))),
                    }
                }
            }
        }
    }

    impl ServerCertVerifier for PinnedServerCertVerifier {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            self.check_pin(end_entity)?;
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.provider
                .signature_verification_algorithms
                .supported_schemes()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_prefixed() {
        let fp = fingerprint(b"hello");
        assert!(fp.starts_with("sha256:"));
        // SHA-256 of "hello".
        assert_eq!(
            fp,
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn fingerprints_match_ignores_formatting() {
        assert!(fingerprints_match("sha256:AABB", "aabb"));
        assert!(fingerprints_match("AA:BB:CC", "aabbcc"));
        assert!(!fingerprints_match("aabb", "ccdd"));
    }

    #[test]
    fn pin_store_first_use_then_match() {
        let dir = std::env::temp_dir().join(format!("wayray-pins-{}", std::process::id()));
        let path = dir.join("known_hosts");
        let _ = std::fs::remove_file(&path);

        let mut store = PinStore::open(&path).unwrap();
        assert_eq!(
            store
                .verify_or_remember("a.example:4433", "sha256:aa")
                .unwrap(),
            PinOutcome::FirstUse
        );
        // Same identity + fingerprint → match.
        assert_eq!(
            store
                .verify_or_remember("a.example:4433", "sha256:aa")
                .unwrap(),
            PinOutcome::Match
        );

        // Reloading from disk preserves the pin.
        let mut reloaded = PinStore::open(&path).unwrap();
        assert_eq!(reloaded.get("a.example:4433"), Some("sha256:aa"));

        // A different fingerprint for the same identity is a mismatch.
        match reloaded
            .verify_or_remember("a.example:4433", "sha256:bb")
            .unwrap()
        {
            PinOutcome::Mismatch {
                expected,
                presented,
            } => {
                assert_eq!(expected, "sha256:aa");
                assert_eq!(presented, "sha256:bb");
            }
            other => panic!("expected mismatch, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn written_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wayray-priv-{}", std::process::id()));
        let path = dir.join("secret");
        write_private(&path, b"data").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Multi-server cluster configuration.
//!
//! WayRay supports a small cluster of compositor servers. Rather than relying
//! on mDNS/DNS-SD (which depends on platform multicast APIs or Avahi/Bonjour C
//! libraries that are absent or unreliable inside illumos zones, and on
//! multicast being enabled on segmented networks), the cluster is described by
//! a static TOML file. This needs zero extra dependencies (reusing the `toml`
//! crate already present), compiles identically on illumos and Linux, and
//! matches SunRay's historically central/configured topology.
//!
//! Example `cluster.toml`:
//!
//! ```toml
//! local_id = "a"
//!
//! [[servers]]
//! id = "a"
//! addr = "10.0.0.1:4433"
//! weight = 1
//!
//! [[servers]]
//! id = "b"
//! addr = "10.0.0.2:4433"
//! weight = 2
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default per-server session capacity used for load-factor calculations when
/// none is configured. A guess pending deployment sizing.
pub const DEFAULT_CAPACITY: u32 = 50;

/// A single server entry in the cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    /// Stable identifier for the server (matches `ClusterConfig::local_id`
    /// on the owning server).
    pub id: String,
    /// Network address as `host:port` (e.g. `10.0.0.1:4433`).
    pub addr: String,
    /// Relative placement weight for load balancing. Higher means the server
    /// is preferred for new sessions. Defaults to `1`.
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

/// Static, config-driven view of the cluster.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClusterConfig {
    /// This server's own id within the `servers` list. Empty when running as a
    /// standalone single-server deployment.
    pub local_id: String,
    /// All servers participating in the cluster.
    pub servers: Vec<ServerEntry>,
}

impl ClusterConfig {
    /// Load a cluster config from a TOML file.
    pub fn load(path: &Path) -> Result<Self, ClusterConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ClusterConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        toml::from_str(&content).map_err(|e| ClusterConfigError::Parse {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Load from the default location, falling back to an empty config
    /// (single-server mode) when the file is absent.
    ///
    /// Search order:
    /// 1. `$XDG_CONFIG_HOME/wayray/cluster.toml` (or `$HOME/.config/...`)
    /// 2. `/etc/wayray/cluster.toml` (system-wide; primary on illumos)
    pub fn load_default() -> Self {
        for path in default_config_paths() {
            match Self::load(&path) {
                Ok(config) => {
                    tracing::info!(path = %path.display(), servers = config.servers.len(), "loaded cluster config");
                    return config;
                }
                Err(ClusterConfigError::Io { .. }) => {
                    // Not present at this location — try the next.
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse cluster config, ignoring");
                }
            }
        }
        // No cluster config found: single-server mode.
        Self::default()
    }

    /// Look up a server entry by its id.
    pub fn server(&self, id: &str) -> Option<&ServerEntry> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// Resolve a server id to its address.
    pub fn addr_of(&self, id: &str) -> Option<&str> {
        self.server(id).map(|s| s.addr.as_str())
    }

    /// Peers other than the local server.
    pub fn peers(&self) -> impl Iterator<Item = &ServerEntry> {
        self.servers.iter().filter(|s| s.id != self.local_id)
    }

    /// Whether this config describes a real cluster (more than the local node).
    pub fn is_clustered(&self) -> bool {
        self.servers.iter().any(|s| s.id != self.local_id)
    }
}

/// Candidate default locations for the cluster config, in priority order.
fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        });
    if let Some(base) = base {
        paths.push(base.join("wayray").join("cluster.toml"));
    }

    // System-wide location (primary on illumos).
    paths.push(PathBuf::from("/etc/wayray/cluster.toml"));

    paths
}

/// Errors from loading cluster config.
#[derive(Debug)]
pub enum ClusterConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl std::fmt::Display for ClusterConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClusterConfigError::Io { path, source } => {
                write!(f, "failed to read {}: {}", path.display(), source)
            }
            ClusterConfigError::Parse { path, source } => {
                write!(f, "failed to parse {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for ClusterConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_two_servers() {
        let toml = r#"
            local_id = "a"

            [[servers]]
            id = "a"
            addr = "10.0.0.1:4433"
            weight = 1

            [[servers]]
            id = "b"
            addr = "10.0.0.2:4433"
            weight = 2
        "#;
        let config: ClusterConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.local_id, "a");
        assert_eq!(config.servers.len(), 2);
        assert_eq!(config.addr_of("a"), Some("10.0.0.1:4433"));
        assert_eq!(config.addr_of("b"), Some("10.0.0.2:4433"));
        assert_eq!(config.server("b").unwrap().weight, 2);
        assert!(config.is_clustered());

        let peers: Vec<&ServerEntry> = config.peers().collect();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, "b");
    }

    #[test]
    fn weight_defaults_to_one() {
        let toml = r#"
            [[servers]]
            id = "solo"
            addr = "127.0.0.1:4433"
        "#;
        let config: ClusterConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.servers[0].weight, 1);
    }

    #[test]
    fn empty_config_is_single_server() {
        let config = ClusterConfig::default();
        assert!(config.servers.is_empty());
        assert!(!config.is_clustered());
        assert_eq!(config.peers().count(), 0);
    }

    #[test]
    fn load_missing_file_is_io_error() {
        // A missing file yields an Io error, which `load_default` treats as
        // "not present here" and falls through to single-server mode. We assert
        // the error classification directly to avoid mutating process-global
        // environment variables (which would race other tests).
        let missing = std::env::temp_dir().join("wayray-cluster-does-not-exist-xyz.toml");
        match ClusterConfig::load(&missing) {
            Err(ClusterConfigError::Io { .. }) => {}
            other => panic!("expected Io error for missing file, got {other:?}"),
        }
    }

    #[test]
    fn load_parses_file() {
        let dir = std::env::temp_dir().join(format!("wayray-cluster-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cluster.toml");
        std::fs::write(
            &path,
            r#"
                local_id = "x"
                [[servers]]
                id = "x"
                addr = "1.2.3.4:4433"
                weight = 3
            "#,
        )
        .unwrap();

        let config = ClusterConfig::load(&path).unwrap();
        assert_eq!(config.local_id, "x");
        assert_eq!(config.server("x").unwrap().weight, 3);

        let _ = std::fs::remove_file(&path);
    }
}

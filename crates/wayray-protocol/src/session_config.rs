//! Session desktop configuration.
//!
//! Defines the `~/.config/wayray/session.toml` format that specifies
//! which window manager, panel, launcher, and autostart apps to run.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Desktop session configuration loaded from `session.toml`.
///
/// Specifies the composable desktop environment: WM, panel, launcher,
/// notification daemon, and autostart applications.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Window manager binary (connects via wayray_wm_manager_v1 protocol).
    /// Default: "wr-wm-floating" (built-in floating WM).
    pub wm: String,

    /// Panel binary (layer-shell Wayland client, e.g., waybar).
    pub panel: Option<String>,

    /// Application launcher binary (layer-shell, e.g., fuzzel).
    pub launcher: Option<String>,

    /// Notification daemon binary (e.g., mako).
    pub notifications: Option<String>,

    /// Applications to start automatically after session setup.
    #[serde(default)]
    pub autostart: Vec<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            wm: "wr-wm-floating".to_string(),
            panel: None,
            launcher: None,
            notifications: None,
            autostart: Vec::new(),
        }
    }
}

impl SessionConfig {
    /// Load session config from a TOML file.
    pub fn load(path: &Path) -> Result<Self, SessionConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| SessionConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        toml::from_str(&content).map_err(|e| SessionConfigError::Parse {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Load from the default location (`~/.config/wayray/session.toml`),
    /// falling back to defaults if the file doesn't exist.
    pub fn load_default() -> Self {
        let config_path = default_config_path();
        match Self::load(&config_path) {
            Ok(config) => config,
            Err(SessionConfigError::Io { .. }) => {
                // File doesn't exist — use defaults.
                Self::default()
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse session config, using defaults");
                Self::default()
            }
        }
    }

    /// Get all binaries that should be launched for this session.
    /// Returns (binary_name, is_wm) pairs. WM is listed first.
    pub fn launch_list(&self) -> Vec<(&str, bool)> {
        let mut list = vec![(self.wm.as_str(), true)];

        if let Some(panel) = &self.panel {
            list.push((panel.as_str(), false));
        }
        if let Some(launcher) = &self.launcher {
            list.push((launcher.as_str(), false));
        }
        if let Some(notifications) = &self.notifications {
            list.push((notifications.as_str(), false));
        }
        for app in &self.autostart {
            list.push((app.as_str(), false));
        }

        list
    }
}

/// Default config file path: `$XDG_CONFIG_HOME/wayray/session.toml`.
pub fn default_config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("wayray").join("session.toml")
}

/// Errors from loading session config.
#[derive(Debug)]
pub enum SessionConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl std::fmt::Display for SessionConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionConfigError::Io { path, source } => {
                write!(f, "failed to read {}: {}", path.display(), source)
            }
            SessionConfigError::Parse { path, source } => {
                write!(f, "failed to parse {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for SessionConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = SessionConfig::default();
        assert_eq!(config.wm, "wr-wm-floating");
        assert!(config.panel.is_none());
        assert!(config.autostart.is_empty());
    }

    #[test]
    fn parse_full_config() {
        let toml = r#"
            wm = "wr-wm-tiling"
            panel = "waybar"
            launcher = "fuzzel"
            notifications = "mako"
            autostart = ["foot", "nm-applet"]
        "#;
        let config: SessionConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.wm, "wr-wm-tiling");
        assert_eq!(config.panel.as_deref(), Some("waybar"));
        assert_eq!(config.launcher.as_deref(), Some("fuzzel"));
        assert_eq!(config.notifications.as_deref(), Some("mako"));
        assert_eq!(config.autostart, vec!["foot", "nm-applet"]);
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"wm = "my-custom-wm""#;
        let config: SessionConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.wm, "my-custom-wm");
        assert!(config.panel.is_none());
        assert!(config.autostart.is_empty());
    }

    #[test]
    fn parse_empty_uses_defaults() {
        let config: SessionConfig = toml::from_str("").unwrap();
        assert_eq!(config.wm, "wr-wm-floating");
    }

    #[test]
    fn launch_list_order() {
        let config = SessionConfig {
            wm: "my-wm".to_string(),
            panel: Some("waybar".to_string()),
            launcher: None,
            notifications: Some("mako".to_string()),
            autostart: vec!["foot".to_string()],
        };
        let list = config.launch_list();
        assert_eq!(list[0], ("my-wm", true));
        assert_eq!(list[1], ("waybar", false));
        assert_eq!(list[2], ("mako", false));
        assert_eq!(list[3], ("foot", false));
    }
}

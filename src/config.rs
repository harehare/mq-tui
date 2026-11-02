//! User-editable app settings (theme, hint bar visibility, ...), read from
//! the user's config directory next to the favorite queries file. The app
//! only reads this file; CLI flags override it for a single run.

use std::path::PathBuf;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum ThemeName {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    /// Whether to show the persistent, mode-specific key-hint bar above the status line.
    pub show_hint_bar: bool,
    /// Color theme applied to the UI.
    pub theme: ThemeName,
    /// Enlarge H1/H2 in the preview on terminals supporting kitty's text sizing protocol.
    pub scale_headings: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_hint_bar: true,
            theme: ThemeName::default(),
            scale_headings: true,
        }
    }
}

/// `<config dir>/mq-tui/config.toml`, honoring `XDG_CONFIG_HOME`/`APPDATA`.
fn config_path() -> Option<PathBuf> {
    let dir = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if cfg!(windows) {
        PathBuf::from(std::env::var("APPDATA").ok()?)
    } else {
        PathBuf::from(std::env::var("HOME").ok()?).join(".config")
    };
    Some(dir.join("mq-tui").join("config.toml"))
}

/// Load settings from disk, or defaults if missing/unparseable.
pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    load_from(&path)
}

fn load_from(path: &PathBuf) -> Config {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Config::default();
    };
    toml::from_str(&content).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.show_hint_bar);
        assert_eq!(config.theme, ThemeName::Dark);
    }

    #[test]
    fn test_roundtrip_serialization() {
        let config = Config {
            show_hint_bar: false,
            theme: ThemeName::Light,
            scale_headings: false,
        };
        let content = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&content).unwrap();
        assert!(!parsed.show_hint_bar);
        assert!(!parsed.scale_headings);
        assert_eq!(parsed.theme, ThemeName::Light);
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        let path = std::env::temp_dir().join("mq-tui-test-config-does-not-exist.toml");
        let config = load_from(&path);
        assert!(config.show_hint_bar);
    }

    #[test]
    fn test_load_from_partial_toml_fills_defaults() {
        let path = std::env::temp_dir().join(format!(
            "mq-tui-test-config-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, "theme = \"light\"\n").unwrap();
        let config = load_from(&path);
        std::fs::remove_file(&path).ok();

        assert!(config.show_hint_bar);
        assert_eq!(config.theme, ThemeName::Light);
    }
}

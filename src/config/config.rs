use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    pub system_path: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub archive_path: Option<String>,
    pub template: Option<String>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub telemetry: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, ToolConfig>,
}

impl Config {
    pub fn telemetry_enabled(&self) -> bool {
        self.telemetry.unwrap_or(false)
    }

    pub fn merge(mut self, other: Config) -> Self {
        if let Some(t) = other.telemetry {
            self.telemetry = Some(t);
        }
        self.tools.extend(other.tools);
        self
    }
}

pub fn load_config(
    start_path: &Path,
    home_dir: Option<&Path>,
    xdg_config_env: Option<&Path>,
) -> Result<Config, Box<dyn std::error::Error>> {
    let mut config = Config::default();

    // 1. System-Level Configuration
    let sys_config_path = Path::new("/etc/scrim/scrim.yaml");
    if sys_config_path.exists() {
        let c = read_config(sys_config_path)?;
        config = config.merge(c);
    }

    // 2. User-Level Configuration (XDG Base Directory Specification)
    let user_config_dir = resolve_config_dir(xdg_config_env, home_dir);
    let user_config_path = user_config_dir.join("scrim/scrim.yaml");
    if user_config_path.exists() {
        let c = read_config(&user_config_path)?;
        config = config.merge(c);
    }

    // 3. Recursive Search
    let mut current = start_path.to_path_buf();
    let mut search_paths = Vec::new();
    
    loop {
        let config_file = current.join("scrim.yaml");
        if config_file.exists() {
            search_paths.push(config_file);
        }
        
        // Stop search if we hit a .git directory (repo boundary)
        if current.join(".git").is_dir() {
            break;
        }

        if !current.pop() {
            break;
        }
    }
    
    // Reverse so parents are merged before children
    for path in search_paths.into_iter().rev() {
        let c = read_config(&path)?;
        config = config.merge(c);
    }
    
    Ok(config)
}

pub fn read_config(path: &Path) -> Result<Config, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = serde_yaml::from_str(&content)?;
    Ok(config)
}

pub fn resolve_config_dir(xdg_config_env: Option<&Path>, home_dir: Option<&Path>) -> PathBuf {
    if let Some(xdg_path) = xdg_config_env {
        if xdg_path.is_absolute() {
            return xdg_path.to_path_buf();
        }
    }

    if let Some(home) = home_dir {
        home.join(".config")
    } else {
        PathBuf::from(".config")
    }
}

pub fn resolve_cache_dir(
    xdg_cache_env: Option<&Path>,
    home_dir: Option<&Path>,
) -> PathBuf {
    let base_cache = if let Some(xdg_path) = xdg_cache_env {
        if xdg_path.is_absolute() {
            xdg_path.to_path_buf()
        } else if let Some(home) = home_dir {
            home.join(".cache")
        } else {
            PathBuf::from(".cache")
        }
    } else if let Some(home) = home_dir {
        home.join(".cache")
    } else {
        PathBuf::from(".cache")
    };

    base_cache.join("scrim")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_load_config_in_current_dir() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("scrim.yaml");
        fs::write(&config_path, "{}").unwrap();

        let config = load_config(dir.path(), Some(dir.path()), None).unwrap();
        assert!(config.tools.is_empty());
    }

    #[test]
    fn test_load_config_layering() {
        let root = tempdir().unwrap();

        // Parent config
        let parent_config_path = root.path().join("scrim.yaml");
        fs::write(&parent_config_path, "telemetry: false\ntools:\n  foo: { system_path: '/bin/parent_foo' }\n  bar: { system_path: '/bin/parent_bar' }").unwrap();

        let child = root.path().join("child");
        fs::create_dir_all(&child).unwrap();

        // Child config
        let child_config_path = child.join("scrim.yaml");
        fs::write(&child_config_path, "telemetry: true\ntools:\n  foo: { system_path: '/bin/child_foo' }").unwrap();

        let config = load_config(&child, Some(root.path()), None).unwrap();
        
        // Child wins for telemetry
        assert_eq!(config.telemetry, Some(true));
        
        // Child wins for foo
        assert_eq!(config.tools.get("foo").unwrap().system_path.as_deref(), Some("/bin/child_foo"));
        
        // Parent retained for bar
        assert_eq!(config.tools.get("bar").unwrap().system_path.as_deref(), Some("/bin/parent_bar"));
    }

    #[test]
    fn test_load_config_not_found() {
        let dir = tempdir().unwrap();
        
        let config = load_config(dir.path(), Some(dir.path()), None).unwrap();
        assert!(config.tools.is_empty());
    }

    #[test]
    fn test_load_config_stops_at_git() {
        let root = tempdir().unwrap();
        
        // Config outside repo
        let config_path = root.path().join("scrim.yaml");
        fs::write(&config_path, "tools:\n  outside: { system_path: '/bin/out' }").unwrap();

        let repo_root = root.path().join("repo");
        let git_dir = repo_root.join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        let child = repo_root.join("child");
        fs::create_dir_all(&child).unwrap();

        // Should return empty because it stops at .git and doesn't see outside config
        let config = load_config(&child, Some(root.path()), None).unwrap();
        assert!(!config.tools.contains_key("outside"));
    }

    #[test]
    fn test_parse_config() {
        let yaml = r#"
telemetry: true
tools:
  node:
    system_path: /usr/local/bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.telemetry, Some(true));
        assert_eq!(config.tools.get("node").unwrap().system_path.as_ref().unwrap(), "/usr/local/bin/node");
        assert_eq!(config.tools.get("go").unwrap().url.as_ref().unwrap(), "https://go.dev/dl/go1.21.5.linux-amd64.tar.gz");
    }

    #[test]
    fn test_parse_config_rejects_unknown_fields() {
        let yaml = r#"
telemetry: true
tools:
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
    acrhive_bin: go/bin/go
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "Expected parsing to fail due to unknown field 'acrhive_bin'");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("unknown field `acrhive_bin`"), "Error should mention the unknown field");
    }

    #[test]
    fn test_telemetry_disabled_by_default() {
        let yaml = r#"
tools:
  node:
    system_path: /usr/local/bin/node
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.telemetry, None);
        assert_eq!(config.telemetry_enabled(), false);
    }

    #[test]
    fn test_load_config_xdg_config_home() {
        let root = tempdir().unwrap();
        let fake_home = root.path().join("fake_home");
        let fake_xdg = root.path().join("custom_xdg_config");
        let project_dir = root.path().join("project");
        fs::create_dir_all(&fake_home).unwrap();
        fs::create_dir_all(&project_dir).unwrap();

        // 1. When XDG_CONFIG_HOME is set and absolute, it loads <XDG_CONFIG_HOME>/scrim/scrim.yaml
        let xdg_scrim_dir = fake_xdg.join("scrim");
        fs::create_dir_all(&xdg_scrim_dir).unwrap();
        fs::write(
            xdg_scrim_dir.join("scrim.yaml"),
            "tools:\n  from_xdg: { system_path: '/bin/from_xdg' }",
        )
        .unwrap();

        let config = load_config(&project_dir, Some(&fake_home), Some(&fake_xdg)).unwrap();
        assert_eq!(
            config.tools.get("from_xdg").unwrap().system_path.as_deref(),
            Some("/bin/from_xdg")
        );

        // 2. When XDG_CONFIG_HOME is relative, it falls back to $HOME/.config/scrim/scrim.yaml
        let home_scrim_dir = fake_home.join(".config/scrim");
        fs::create_dir_all(&home_scrim_dir).unwrap();
        fs::write(
            home_scrim_dir.join("scrim.yaml"),
            "tools:\n  from_home: { system_path: '/bin/from_home' }",
        )
        .unwrap();

        let config_rel = load_config(
            &project_dir,
            Some(&fake_home),
            Some(Path::new("relative/path")),
        )
        .unwrap();
        assert_eq!(
            config_rel.tools.get("from_home").unwrap().system_path.as_deref(),
            Some("/bin/from_home")
        );
    }

    #[test]
    fn test_resolve_config_dir() {
        let home = Path::new("/home/user");
        let xdg_abs = Path::new("/custom/xdg_config");
        let xdg_rel = Path::new("custom/xdg_config");
        let empty_path = Path::new("");

        // Absolute path is respected
        assert_eq!(
            resolve_config_dir(Some(xdg_abs), Some(home)),
            PathBuf::from("/custom/xdg_config")
        );

        // Relative path is ignored per XDG spec, falls back to $HOME/.config
        assert_eq!(
            resolve_config_dir(Some(xdg_rel), Some(home)),
            PathBuf::from("/home/user/.config")
        );

        // Empty path falls back to $HOME/.config
        assert_eq!(
            resolve_config_dir(Some(empty_path), Some(home)),
            PathBuf::from("/home/user/.config")
        );

        // None falls back to $HOME/.config
        assert_eq!(
            resolve_config_dir(None, Some(home)),
            PathBuf::from("/home/user/.config")
        );

        // HOME absent falls back to .config
        assert_eq!(
            resolve_config_dir(None, None),
            PathBuf::from(".config")
        );
    }

    #[test]
    fn test_resolve_cache_dir() {
        let home = Path::new("/home/user");
        let xdg_abs = Path::new("/custom/xdg_cache");
        let xdg_rel = Path::new("custom/xdg_cache");
        let empty_path = Path::new("");

        // Absolute path is respected
        assert_eq!(
            resolve_cache_dir(Some(xdg_abs), Some(home)),
            PathBuf::from("/custom/xdg_cache/scrim")
        );

        // Relative path is ignored per XDG spec, falls back to $HOME/.cache
        assert_eq!(
            resolve_cache_dir(Some(xdg_rel), Some(home)),
            PathBuf::from("/home/user/.cache/scrim")
        );

        // Empty path falls back to $HOME/.cache
        assert_eq!(
            resolve_cache_dir(Some(empty_path), Some(home)),
            PathBuf::from("/home/user/.cache/scrim")
        );

        // None falls back to $HOME/.cache
        assert_eq!(
            resolve_cache_dir(None, Some(home)),
            PathBuf::from("/home/user/.cache/scrim")
        );

        // HOME absent falls back to .cache
        assert_eq!(
            resolve_cache_dir(None, None),
            PathBuf::from(".cache/scrim")
        );
    }

    #[test]
    fn test_config_serialization_order() {
        let mut tools = BTreeMap::new();
        tools.insert("zeta".to_string(), ToolConfig {
            system_path: Some("/bin/zeta".to_string()),
            ..Default::default()
        });
        tools.insert("alpha".to_string(), ToolConfig {
            system_path: Some("/bin/alpha".to_string()),
            ..Default::default()
        });
        tools.insert("beta".to_string(), ToolConfig {
            system_path: Some("/bin/beta".to_string()),
            ..Default::default()
        });
        let config = Config {
            telemetry: Some(true),
            tools,
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let alpha_pos = yaml.find("alpha").unwrap();
        let beta_pos = yaml.find("beta").unwrap();
        let zeta_pos = yaml.find("zeta").unwrap();

        assert!(alpha_pos < beta_pos);
        assert!(beta_pos < zeta_pos);
    }
}

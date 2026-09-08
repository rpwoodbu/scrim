use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tools: HashMap<String, ToolConfig>,
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

pub fn load_config(start_path: &Path, home_dir: Option<&Path>) -> Result<Config, Box<dyn std::error::Error>> {
    let mut config = Config::default();

    // 1. System-Level Configuration
    let sys_config_path = Path::new("/etc/scrim/scrim.yaml");
    if sys_config_path.exists() {
        let c = read_config(sys_config_path)?;
        config = config.merge(c);
    }

    // 2. User-Level Configuration
    if let Some(home) = home_dir {
        let user_config_path = Path::new(home).join(".config/scrim/scrim.yaml");
        if user_config_path.exists() {
            let c = read_config(&user_config_path)?;
            config = config.merge(c);
        }
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

        let config = load_config(dir.path(), Some(dir.path())).unwrap();
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

        let config = load_config(&child, Some(root.path())).unwrap();
        
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
        
        let config = load_config(dir.path(), Some(dir.path())).unwrap();
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
        let config = load_config(&child, Some(root.path())).unwrap();
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
}

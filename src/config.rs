use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolConfig {
    pub path: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub archive_bin: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_telemetry")]
    pub telemetry: bool,
    pub tools: HashMap<String, ToolConfig>,
}

fn default_telemetry() -> bool {
    true
}

pub fn find_config(start_path: &Path) -> Option<PathBuf> {
    let mut current = start_path.to_path_buf();
    loop {
        let config_file = current.join("scrim.yaml");
        if config_file.exists() {
            return Some(config_file);
        }
        
        // Stop search if we hit a .git directory (repo boundary)
        if current.join(".git").is_dir() {
            break;
        }

        if !current.pop() {
            break;
        }
    }
    None
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
    fn test_find_config_in_current_dir() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("scrim.yaml");
        fs::write(&config_path, "{}").unwrap();

        assert_eq!(find_config(dir.path()), Some(config_path));
    }

    #[test]
    fn test_find_config_in_parent_dir() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("scrim.yaml");
        fs::write(&config_path, "{}").unwrap();

        let child = root.path().join("child/grandchild");
        fs::create_dir_all(&child).unwrap();

        assert_eq!(find_config(&child), Some(config_path));
    }

    #[test]
    fn test_find_config_not_found() {
        let dir = tempdir().unwrap();
        assert_eq!(find_config(dir.path()), None);
    }

    #[test]
    fn test_find_config_stops_at_git() {
        let root = tempdir().unwrap();
        let config_path = root.path().join("scrim.yaml");
        fs::write(&config_path, "{}").unwrap();

        let repo_root = root.path().join("repo");
        let git_dir = repo_root.join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        let child = repo_root.join("child");
        fs::create_dir_all(&child).unwrap();

        // Should return None because it shouldn't look past the .git directory
        assert_eq!(find_config(&child), None);
    }

    #[test]
    fn test_parse_config() {
        let yaml = r#"
telemetry: true
tools:
  node:
    path: /usr/local/bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.telemetry, true);
        assert_eq!(config.tools.get("node").unwrap().path.as_ref().unwrap(), "/usr/local/bin/node");
        assert_eq!(config.tools.get("go").unwrap().url.as_ref().unwrap(), "https://go.dev/dl/go1.21.5.linux-amd64.tar.gz");
    }
}

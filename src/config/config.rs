use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportPrecedence {
    Before,
    After,
}

impl Default for ImportPrecedence {
    fn default() -> Self {
        ImportPrecedence::Before
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportObject {
    pub path: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub precedence: ImportPrecedence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImportEntry {
    Shorthand(String),
    Structured(ImportObject),
}

impl ImportEntry {
    pub fn path(&self) -> &str {
        match self {
            ImportEntry::Shorthand(path) => path,
            ImportEntry::Structured(obj) => &obj.path,
        }
    }

    pub fn optional(&self) -> bool {
        match self {
            ImportEntry::Shorthand(_) => false,
            ImportEntry::Structured(obj) => obj.optional,
        }
    }

    pub fn precedence(&self) -> &ImportPrecedence {
        match self {
            ImportEntry::Shorthand(_) => &ImportPrecedence::Before,
            ImportEntry::Structured(obj) => &obj.precedence,
        }
    }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    pub system_path: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>,
    pub archive_path: Option<String>,
    pub template: Option<String>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub telemetry: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<ImportEntry>,
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
    let mut import_stack = Vec::new();
    read_config_internal(path, &mut import_stack)
}

fn read_config_internal(
    path: &Path,
    import_stack: &mut Vec<PathBuf>,
) -> Result<Config, Box<dyn std::error::Error>> {
    let canonical = path.canonicalize().map_err(|e| {
        format!("Failed to resolve configuration path '{}': {}", path.display(), e)
    })?;

    if import_stack.contains(&canonical) {
        let cycle_chain = import_stack
            .iter()
            .chain(std::iter::once(&canonical))
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(format!("Circular import detected in configuration: {}", cycle_chain).into());
    }

    let base_dir = canonical
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/"));

    import_stack.push(canonical);

    let content = std::fs::read_to_string(path).map_err(|e| {
        format!("Failed to read configuration file '{}': {}", path.display(), e)
    })?;

    let parsed: Config = serde_yaml::from_str(&content).map_err(|e| {
        format!("Failed to parse configuration file '{}': {}", path.display(), e)
    })?;

    let mut before_config = Config::default();
    let mut after_config = Config::default();

    for import_entry in &parsed.imports {
        let import_path_str = import_entry.path();
        let target_path = if Path::new(import_path_str).is_absolute() {
            PathBuf::from(import_path_str)
        } else {
            base_dir.join(import_path_str)
        };

        if !target_path.exists() {
            if import_entry.optional() {
                continue;
            } else {
                return Err(format!(
                    "Imported configuration file '{}' declared in '{}' does not exist",
                    target_path.display(),
                    path.display()
                )
                .into());
            }
        }

        let imported = read_config_internal(&target_path, import_stack)?;
        match import_entry.precedence() {
            ImportPrecedence::Before => {
                before_config = before_config.merge(imported);
            }
            ImportPrecedence::After => {
                after_config = after_config.merge(imported);
            }
        }
    }

    import_stack.pop();

    // Layering inside file:
    // 1. All precedence: before imports (in order)
    // 2. Current file's own definitions
    // 3. All precedence: after imports (in order)
    let current_file_clean = Config {
        telemetry: parsed.telemetry,
        imports: Vec::new(),
        tools: parsed.tools,
    };

    let result = before_config.merge(current_file_clean).merge(after_config);
    Ok(result)
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
            imports: Vec::new(),
            tools,
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let alpha_pos = yaml.find("alpha").unwrap();
        let beta_pos = yaml.find("beta").unwrap();
        let zeta_pos = yaml.find("zeta").unwrap();

        assert!(alpha_pos < beta_pos);
        assert!(beta_pos < zeta_pos);
    }

    #[test]
    fn test_parse_imports_shorthand_and_structured() {
        let yaml = r#"
imports:
  - base.yaml
  - path: overrides.yaml
    optional: true
    precedence: after
tools:
  node:
    system_path: /bin/node
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.imports.len(), 2);
        assert_eq!(config.imports[0].path(), "base.yaml");
        assert_eq!(config.imports[0].optional(), false);
        assert_eq!(config.imports[0].precedence(), &ImportPrecedence::Before);

        assert_eq!(config.imports[1].path(), "overrides.yaml");
        assert_eq!(config.imports[1].optional(), true);
        assert_eq!(config.imports[1].precedence(), &ImportPrecedence::After);
    }

    #[test]
    fn test_imports_precedence_and_overrides() {
        let root = tempdir().unwrap();

        // base.yaml (precedence: before)
        let base_path = root.path().join("base.yaml");
        fs::write(
            &base_path,
            r#"
tools:
  tool_a:
    system_path: /bin/base_a
  tool_b:
    system_path: /bin/base_b
  tool_c:
    system_path: /bin/base_c
"#,
        )
        .unwrap();

        // override.yaml (precedence: after)
        let override_path = root.path().join("override.yaml");
        fs::write(
            &override_path,
            r#"
tools:
  tool_b:
    system_path: /bin/override_b
"#,
        )
        .unwrap();

        // main scrim.yaml:
        // - imports base.yaml (precedence before, so main's tool_a overrides base's tool_a)
        // - defines tool_a (overrides base) and tool_b (will be overridden by after)
        // - imports override.yaml (precedence after, so overrides main's tool_b)
        let main_path = root.path().join("scrim.yaml");
        fs::write(
            &main_path,
            r#"
imports:
  - base.yaml
  - path: override.yaml
    precedence: after
tools:
  tool_a:
    system_path: /bin/main_a
  tool_b:
    system_path: /bin/main_b
"#,
        )
        .unwrap();

        let config = read_config(&main_path).unwrap();
        // tool_a should be /bin/main_a (main overrides base)
        assert_eq!(
            config.tools.get("tool_a").unwrap().system_path.as_deref(),
            Some("/bin/main_a")
        );
        // tool_b should be /bin/override_b (override.yaml takes precedence after main)
        assert_eq!(
            config.tools.get("tool_b").unwrap().system_path.as_deref(),
            Some("/bin/override_b")
        );
        // tool_c should be /bin/base_c (inherited from base)
        assert_eq!(
            config.tools.get("tool_c").unwrap().system_path.as_deref(),
            Some("/bin/base_c")
        );
    }

    #[test]
    fn test_imports_sequential_order_within_same_precedence() {
        let root = tempdir().unwrap();

        let base1 = root.path().join("base1.yaml");
        fs::write(
            &base1,
            "tools:\n  tool1: { system_path: '/bin/base1' }",
        )
        .unwrap();

        let base2 = root.path().join("base2.yaml");
        fs::write(
            &base2,
            "tools:\n  tool1: { system_path: '/bin/base2' }",
        )
        .unwrap();

        let main_path = root.path().join("scrim.yaml");
        fs::write(
            &main_path,
            "imports:\n  - base1.yaml\n  - base2.yaml\n",
        )
        .unwrap();

        let config = read_config(&main_path).unwrap();
        // base2 was listed second, so it wins over base1
        assert_eq!(
            config.tools.get("tool1").unwrap().system_path.as_deref(),
            Some("/bin/base2")
        );
    }

    #[test]
    fn test_imports_relative_to_declaring_file() {
        let root = tempdir().unwrap();
        let sub = root.path().join("subdir");
        fs::create_dir_all(&sub).unwrap();

        let helper_path = sub.join("helper.yaml");
        fs::write(
            &helper_path,
            "tools:\n  helper: { system_path: '/bin/helper' }",
        )
        .unwrap();

        let sub_config = sub.join("scrim.yaml");
        fs::write(
            &sub_config,
            "imports:\n  - helper.yaml\n",
        )
        .unwrap();

        // Reading sub_config must resolve helper.yaml relative to subdir/
        let config = read_config(&sub_config).unwrap();
        assert_eq!(
            config.tools.get("helper").unwrap().system_path.as_deref(),
            Some("/bin/helper")
        );
    }

    #[test]
    fn test_imports_missing_required_fails() {
        let root = tempdir().unwrap();
        let main_path = root.path().join("scrim.yaml");
        fs::write(
            &main_path,
            "imports:\n  - nonexistent.yaml\n",
        )
        .unwrap();

        let err = read_config(&main_path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nonexistent.yaml"));
        assert!(msg.contains("does not exist"));
    }

    #[test]
    fn test_imports_missing_optional_succeeds() {
        let root = tempdir().unwrap();
        let main_path = root.path().join("scrim.yaml");
        fs::write(
            &main_path,
            r#"
imports:
  - path: nonexistent.yaml
    optional: true
tools:
  node:
    system_path: /bin/node
"#,
        )
        .unwrap();

        let config = read_config(&main_path).unwrap();
        assert_eq!(
            config.tools.get("node").unwrap().system_path.as_deref(),
            Some("/bin/node")
        );
    }

    #[test]
    fn test_imports_circular_dependency_fails() {
        let root = tempdir().unwrap();
        let file_a = root.path().join("a.yaml");
        let file_b = root.path().join("b.yaml");

        fs::write(
            &file_a,
            "imports:\n  - b.yaml\ntools:\n  a: { system_path: '/bin/a' }",
        )
        .unwrap();

        fs::write(
            &file_b,
            "imports:\n  - a.yaml\ntools:\n  b: { system_path: '/bin/b' }",
        )
        .unwrap();

        let err = read_config(&file_a).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Circular import detected"));
        assert!(msg.contains("a.yaml"));
        assert!(msg.contains("b.yaml"));
    }

    #[test]
    fn test_imports_diamond_dependency_succeeds() {
        let root = tempdir().unwrap();
        let file_d = root.path().join("d.yaml");
        let file_b = root.path().join("b.yaml");
        let file_c = root.path().join("c.yaml");
        let file_a = root.path().join("a.yaml");

        // D is imported by both B and C (diamond graph: A -> B, C -> D)
        fs::write(
            &file_d,
            "tools:\n  shared_tool: { system_path: '/bin/shared_tool' }",
        )
        .unwrap();

        fs::write(
            &file_b,
            "imports:\n  - d.yaml\ntools:\n  tool_b: { system_path: '/bin/tool_b' }",
        )
        .unwrap();

        fs::write(
            &file_c,
            "imports:\n  - d.yaml\ntools:\n  tool_c: { system_path: '/bin/tool_c' }",
        )
        .unwrap();

        fs::write(
            &file_a,
            "imports:\n  - b.yaml\n  - c.yaml\n",
        )
        .unwrap();

        let config = read_config(&file_a).expect("Diamond imports should not be flagged as circular");
        assert_eq!(
            config.tools.get("shared_tool").unwrap().system_path.as_deref(),
            Some("/bin/shared_tool")
        );
        assert_eq!(
            config.tools.get("tool_b").unwrap().system_path.as_deref(),
            Some("/bin/tool_b")
        );
        assert_eq!(
            config.tools.get("tool_c").unwrap().system_path.as_deref(),
            Some("/bin/tool_c")
        );
    }
}

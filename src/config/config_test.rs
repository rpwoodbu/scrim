use config::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_platform_parsing_valid() {
    assert_eq!(
        Platform::parse("linux-x86_64").unwrap(),
        Platform::new(Os::Linux, Arch::X86_64)
    );
    assert_eq!(
        Platform::parse("linux-aarch64").unwrap(),
        Platform::new(Os::Linux, Arch::Aarch64)
    );
    assert_eq!(
        Platform::parse("macos-x86_64").unwrap(),
        Platform::new(Os::Macos, Arch::X86_64)
    );
    assert_eq!(
        Platform::parse("macos-aarch64").unwrap(),
        Platform::new(Os::Macos, Arch::Aarch64)
    );

    // Wildcards
    assert_eq!(
        Platform::parse("linux-any").unwrap(),
        Platform::new(Os::Linux, Arch::Any)
    );
    assert_eq!(
        Platform::parse("macos-any").unwrap(),
        Platform::new(Os::Macos, Arch::Any)
    );
    assert_eq!(
        Platform::parse("any-x86_64").unwrap(),
        Platform::new(Os::Any, Arch::X86_64)
    );
    assert_eq!(
        Platform::parse("any-aarch64").unwrap(),
        Platform::new(Os::Any, Arch::Aarch64)
    );
    assert_eq!(
        Platform::parse("any").unwrap(),
        Platform::new(Os::Any, Arch::Any)
    );
    assert_eq!(
        Platform::parse("any-any").unwrap(),
        Platform::new(Os::Any, Arch::Any)
    );
}

#[test]
fn test_platform_parsing_synonyms_and_errors() {
    // Arch synonyms
    let err = Platform::parse("arm64").unwrap_err();
    assert!(err.contains("missing OS component"));
    assert!(err.contains("did you mean 'aarch64'?"));

    let err = Platform::parse("linux-arm64").unwrap_err();
    assert!(err.contains("'arm64' is an invalid architecture (did you mean 'aarch64'?)"));
    assert!(err.contains("Did you mean 'linux-aarch64'?"));

    let err = Platform::parse("linux-amd64").unwrap_err();
    assert!(err.contains("'amd64' is an invalid architecture (did you mean 'x86_64'?)"));
    assert!(err.contains("Did you mean 'linux-x86_64'?"));

    let err = Platform::parse("linux-x86-64").unwrap_err();
    assert!(err.contains("'x86-64' is an invalid architecture (did you mean 'x86_64'?)"));
    assert!(err.contains("Did you mean 'linux-x86_64'?"));

    let err = Platform::parse("linux-x64").unwrap_err();
    assert!(err.contains("'x64' is an invalid architecture (did you mean 'x86_64'?)"));
    assert!(err.contains("Did you mean 'linux-x86_64'?"));

    let err = Platform::parse("linux-aarch_64").unwrap_err();
    assert!(err.contains("'aarch_64' is an invalid architecture (did you mean 'aarch64'?)"));
    assert!(err.contains("Did you mean 'linux-aarch64'?"));

    let err = Platform::parse("aarch_64").unwrap_err();
    assert!(err.contains("missing OS component"));
    assert!(err.contains("did you mean 'aarch64'?"));

    // OS synonyms
    let err = Platform::parse("darwin").unwrap_err();
    assert!(err.contains("missing architecture component"));
    assert!(err.contains("did you mean 'macos'?"));

    let err = Platform::parse("darwin-x86_64").unwrap_err();
    assert!(err.contains("'darwin' is an invalid OS (did you mean 'macos'?)"));
    assert!(err.contains("Did you mean 'macos-x86_64'?"));

    let err = Platform::parse("osx-aarch64").unwrap_err();
    assert!(err.contains("'osx' is an invalid OS (did you mean 'macos'?)"));
    assert!(err.contains("Did you mean 'macos-aarch64'?"));

    // Both synonyms
    let err = Platform::parse("darwin-arm64").unwrap_err();
    assert!(err.contains("'darwin' is an invalid OS (did you mean 'macos'?)"));
    assert!(err.contains("'arm64' is an invalid architecture (did you mean 'aarch64'?)"));
    assert!(err.contains("Did you mean 'macos-aarch64'?"));

    // Underscore separator
    let err = Platform::parse("linux_x86_64").unwrap_err();
    assert!(err.contains("must be separated by '-', not '_'"));
    assert!(err.contains("Did you mean 'linux-x86_64'?"));

    let err = Platform::parse("darwin_arm64").unwrap_err();
    assert!(err.contains("must be separated by '-', not '_'"));
    assert!(err.contains("Did you mean 'macos-aarch64'?"));

    // Unknown OS / Arch
    let err = Platform::parse("windows-x86_64").unwrap_err();
    assert!(err.contains("'windows' is an invalid OS"));
    assert!(err.contains("Valid OS: 'linux', 'macos', 'any'"));

    let err = Platform::parse("linux-mips").unwrap_err();
    assert!(err.contains("'mips' is an invalid architecture"));
    assert!(err.contains("Valid arch: 'x86_64', 'aarch64', 'any'"));
}

#[test]
fn test_platform_host() {
    let host = Platform::host().expect("Platform::host() should succeed on test environment");
    assert!(host.os == Os::Linux || host.os == Os::Macos);
    assert!(host.arch == Arch::X86_64 || host.arch == Arch::Aarch64);
}

#[test]
fn test_platform_matching_precedence() {
    let host = Platform::new(Os::Linux, Arch::X86_64);
    let mut tool = ToolConfig::default();

    // 1. Only universal "any"
    tool.platforms.insert(
        "any".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/any".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(
        tool.get_platform(&host).unwrap().system_path.as_deref(),
        Some("/bin/any")
    );

    // 2. Add any-x86_64 (arch wildcard beats universal)
    tool.platforms.insert(
        "any-x86_64".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/any-x86_64".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(
        tool.get_platform(&host).unwrap().system_path.as_deref(),
        Some("/bin/any-x86_64")
    );

    // 3. Add linux-any (OS wildcard beats arch wildcard)
    tool.platforms.insert(
        "linux-any".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/linux-any".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(
        tool.get_platform(&host).unwrap().system_path.as_deref(),
        Some("/bin/linux-any")
    );

    // 4. Add linux-x86_64 (exact match beats OS wildcard)
    tool.platforms.insert(
        "linux-x86_64".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/linux-x86_64".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(
        tool.get_platform(&host).unwrap().system_path.as_deref(),
        Some("/bin/linux-x86_64")
    );

    // 5. Query for unconfigured platform
    let macos_aarch64 = Platform::new(Os::Macos, Arch::Aarch64);
    let mut tool2 = ToolConfig::default();
    tool2.platforms.insert(
        "linux-x86_64".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/linux-x86_64".to_string()),
            ..Default::default()
        },
    );
    assert!(tool2.get_platform(&macos_aarch64).is_none());

    // 6. any-any matching queries for any and vice-versa
    let mut tool_any_any = ToolConfig::default();
    tool_any_any.platforms.insert(
        "any-any".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/universal".to_string()),
            ..Default::default()
        },
    );
    let query_any = Platform::parse("any").unwrap();
    assert_eq!(
        tool_any_any.get_platform(&query_any).unwrap().system_path.as_deref(),
        Some("/bin/universal")
    );
    assert_eq!(
        tool_any_any.get_platform(&host).unwrap().system_path.as_deref(),
        Some("/bin/universal")
    );

    let mut tool_any = ToolConfig::default();
    tool_any.platforms.insert(
        "any".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/universal2".to_string()),
            ..Default::default()
        },
    );
    let query_any_any = Platform::parse("any-any").unwrap();
    assert_eq!(
        tool_any.get_platform(&query_any_any).unwrap().system_path.as_deref(),
        Some("/bin/universal2")
    );
}

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
    fs::write(
        &parent_config_path,
        "default_platform: linux-x86_64\ntelemetry: false\ntools:\n  foo: { system_path: '/bin/parent_foo' }\n  bar: { system_path: '/bin/parent_bar' }",
    )
    .unwrap();

    let child = root.path().join("child");
    fs::create_dir_all(&child).unwrap();

    // Child config
    let child_config_path = child.join("scrim.yaml");
    fs::write(
        &child_config_path,
        "default_platform: linux-x86_64\ntelemetry: true\ntools:\n  foo: { system_path: '/bin/child_foo' }",
    )
    .unwrap();

    let config = load_config(&child, Some(root.path()), None).unwrap();

    // Child telemetry: true adds default file sink (parent telemetry: false is a no-op)
    assert!(config.telemetry_enabled());
    assert_eq!(config.telemetry.file, vec![FileTelemetryConfig { path: None }]);

    // Child wins for foo
    let foo_plat = config.tools.get("foo").unwrap().platforms.get("linux-x86_64").unwrap();
    assert_eq!(foo_plat.system_path.as_deref(), Some("/bin/child_foo"));

    // Parent retained for bar
    let bar_plat = config.tools.get("bar").unwrap().platforms.get("linux-x86_64").unwrap();
    assert_eq!(bar_plat.system_path.as_deref(), Some("/bin/parent_bar"));
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
    fs::write(
        &config_path,
        "default_platform: linux-x86_64\ntools:\n  outside: { system_path: '/bin/out' }",
    )
    .unwrap();

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
fn test_parse_config_with_default_platform_desugaring() {
    let yaml = r#"
telemetry: true
default_platform: linux-x86_64
tools:
  node:
    system_path: /usr/local/bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
"#;
    let config = Config::parse_str(yaml).unwrap();
    assert!(config.telemetry_enabled());
    assert_eq!(config.telemetry.file, vec![FileTelemetryConfig { path: None }]);
    assert_eq!(config.default_platform, None); // default_platform does not leak

    let node = config.tools.get("node").unwrap();
    assert_eq!(
        node.platforms.get("linux-x86_64").unwrap().system_path.as_deref(),
        Some("/usr/local/bin/node")
    );

    let go = config.tools.get("go").unwrap();
    assert_eq!(
        go.platforms.get("linux-x86_64").unwrap().url.as_deref(),
        Some("https://go.dev/dl/go1.21.5.linux-amd64.tar.gz")
    );
    assert_eq!(
        go.platforms.get("linux-x86_64").unwrap().sha256.as_deref(),
        Some("285c1f0624022839446d32")
    );
}

#[test]
fn test_parse_config_rejects_unknown_fields() {
    let yaml = r#"
telemetry: true
default_platform: linux-x86_64
tools:
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
    acrhive_bin: go/bin/go
"#;
    let result: Result<Config, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err(), "Expected parsing to fail due to unknown field 'acrhive_bin'");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("unknown field `acrhive_bin`"));
}

#[test]
fn test_parse_config_rejects_unknown_field_in_platform() {
    let yaml = r#"
tools:
  go:
    platforms:
      linux-x86_64:
        url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
        sha256: 285c1f0624022839446d32
        bad_field: true
"#;
    let result: Result<Config, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err(), "Expected parsing to fail due to unknown field 'bad_field'");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("unknown field `bad_field`"));
}

#[test]
fn test_direct_fields_without_default_platform_rejected() {
    let yaml = r#"
tools:
  node:
    system_path: /bin/node
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("no 'default_platform' is declared"));
}

#[test]
fn test_no_mixing_rule() {
    let yaml = r#"
default_platform: linux-x86_64
tools:
  node:
    system_path: /bin/node
    platforms:
      macos-aarch64:
        system_path: /opt/homebrew/bin/node
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("mixes direct platform properties with an explicit 'platforms' block"));
}

#[test]
fn test_file_scoped_default_platform_across_imports() {
    let root = tempdir().unwrap();

    // sub.yaml has NO default_platform and defines a tool using direct fields: must fail!
    let sub_invalid = root.path().join("sub_invalid.yaml");
    fs::write(
        &sub_invalid,
        "tools:\n  sub_tool: { system_path: '/bin/sub' }",
    )
    .unwrap();

    let main_import_invalid = root.path().join("main_import_invalid.yaml");
    fs::write(
        &main_import_invalid,
        "default_platform: linux-x86_64\nimports:\n  - sub_invalid.yaml\ntools:\n  main_tool: { system_path: '/bin/main' }",
    )
    .unwrap();

    let config = read_config(&main_import_invalid).unwrap();
    // main_tool is valid and desugared
    assert!(config.tools.get("main_tool").unwrap().validate("main_tool").is_ok());
    // sub_tool fails validation with actionable error
    let sub_err = config.tools.get("sub_tool").unwrap().validate("sub_tool").unwrap_err();
    assert!(sub_err.contains("no 'default_platform' is declared for this file"));
    assert!(config.validate().unwrap_err().contains("no 'default_platform' is declared for this file"));

    // Now sub.yaml has its own default_platform (macos-aarch64)
    let sub_valid = root.path().join("sub_valid.yaml");
    fs::write(
        &sub_valid,
        "default_platform: macos-aarch64\ntools:\n  sub_tool: { system_path: '/bin/sub' }",
    )
    .unwrap();

    let main_valid = root.path().join("main_valid.yaml");
    fs::write(
        &main_valid,
        "default_platform: linux-x86_64\nimports:\n  - sub_valid.yaml\ntools:\n  main_tool: { system_path: '/bin/main' }",
    )
    .unwrap();

    let config = read_config(&main_valid).unwrap();
    // default_platform was not leaked into config
    assert_eq!(config.default_platform, None);

    // sub_tool desugared to macos-aarch64
    let sub_tool = config.tools.get("sub_tool").unwrap();
    assert!(sub_tool.platforms.contains_key("macos-aarch64"));
    assert!(!sub_tool.platforms.contains_key("linux-x86_64"));

    // main_tool desugared to linux-x86_64
    let main_tool = config.tools.get("main_tool").unwrap();
    assert!(main_tool.platforms.contains_key("linux-x86_64"));
    assert!(!main_tool.platforms.contains_key("macos-aarch64"));
}

#[test]
fn test_validation_empty_tool_rejected() {
    let yaml = r#"
tools:
  empty_tool: {}
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("Tool 'empty_tool' has an empty configuration"));
}

#[test]
fn test_validation_relative_system_path_rejected() {
    let yaml = r#"
default_platform: linux-x86_64
tools:
  rel_tool:
    system_path: ./rel/path
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("specifies relative system_path './rel/path'"));
}

#[test]
fn test_validation_digest_mutual_exclusivity() {
    let yaml = r#"
tools:
  multi_digest:
    platforms:
      linux-x86_64:
        url: https://example.com/tool.tar.gz
        sha256: 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef
        sha512: abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("specifies multiple digests"));
}

#[test]
fn test_validation_url_without_digest() {
    let yaml = r#"
tools:
  nodigest:
    platforms:
      linux-x86_64:
        url: https://example.com/tool.tar.gz
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("specifies 'url' without a digest"));
}

#[test]
fn test_validation_digest_without_url() {
    let yaml = r#"
tools:
  nourl:
    platforms:
      linux-x86_64:
        sha256: 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("specifies 'sha256' without 'url'"));
}

#[test]
fn test_validation_template_mutual_exclusivity() {
    let yaml = r#"
tools:
  templated_bad:
    template: other
    platforms:
      linux-x86_64:
        url: https://example.com/tool.tar.gz
        sha256: 1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("specifies 'template' which is mutually exclusive with 'url' and digests"));
}

#[test]
fn test_validation_archive_path_without_url_or_template() {
    let yaml = r#"
tools:
  local_archive:
    archive_path: bin/tool
    platforms:
      linux-x86_64:
        system_path: /bin/local
"#;
    let err = Config::parse_str(yaml).unwrap_err();
    assert!(err.to_string().contains("specifies 'archive_path' but provides no 'url' or 'template'"));
}

#[test]
fn test_validation_templated_tool_with_platform_archive_path() {
    let yaml = r#"
tools:
  gofmt:
    template: go
    platforms:
      linux-x86_64:
        archive_path: go/bin/gofmt-linux
      macos-aarch64:
        archive_path: go/bin/gofmt-darwin
"#;
    let config = Config::parse_str(yaml).expect("Templated tool with platform archive_path should pass validation");
    let gofmt = config.tools.get("gofmt").unwrap();
    assert_eq!(
        gofmt.platforms.get("linux-x86_64").unwrap().archive_path.as_deref(),
        Some("go/bin/gofmt-linux")
    );
    assert_eq!(
        gofmt.platforms.get("macos-aarch64").unwrap().archive_path.as_deref(),
        Some("go/bin/gofmt-darwin")
    );
}

#[test]
fn test_telemetry_disabled_by_default() {
    let yaml = r#"
default_platform: linux-x86_64
tools:
  node:
    system_path: /usr/local/bin/node
"#;
    let config = Config::parse_str(yaml).unwrap();
    assert_eq!(config.telemetry, TelemetryConfig::default());
    assert!(config.telemetry.is_empty());
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
        "default_platform: linux-x86_64\ntools:\n  from_xdg: { system_path: '/bin/from_xdg' }",
    )
    .unwrap();

    let config = load_config(&project_dir, Some(&fake_home), Some(&fake_xdg)).unwrap();
    let tool = config.tools.get("from_xdg").unwrap();
    assert_eq!(
        tool.platforms.get("linux-x86_64").unwrap().system_path.as_deref(),
        Some("/bin/from_xdg")
    );

    // 2. When XDG_CONFIG_HOME is relative, it falls back to $HOME/.config/scrim/scrim.yaml
    let home_scrim_dir = fake_home.join(".config/scrim");
    fs::create_dir_all(&home_scrim_dir).unwrap();
    fs::write(
        home_scrim_dir.join("scrim.yaml"),
        "default_platform: linux-x86_64\ntools:\n  from_home: { system_path: '/bin/from_home' }",
    )
    .unwrap();

    let config_rel = load_config(
        &project_dir,
        Some(&fake_home),
        Some(Path::new("relative/path")),
    )
    .unwrap();
    let tool_rel = config_rel.tools.get("from_home").unwrap();
    assert_eq!(
        tool_rel.platforms.get("linux-x86_64").unwrap().system_path.as_deref(),
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
    assert_eq!(resolve_config_dir(None, None), PathBuf::from(".config"));
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
    assert_eq!(resolve_cache_dir(None, None), PathBuf::from(".cache/scrim"));
}

#[test]
fn test_config_serialization_order() {
    let mut tools = BTreeMap::new();
    let make_tool = |path: &str| {
        let mut platforms = BTreeMap::new();
        platforms.insert(
            "linux-x86_64".to_string(),
            PlatformToolConfig {
                system_path: Some(path.to_string()),
                ..Default::default()
            },
        );
        ToolConfig {
            platforms,
            ..Default::default()
        }
    };

    tools.insert("zeta".to_string(), make_tool("/bin/zeta"));
    tools.insert("alpha".to_string(), make_tool("/bin/alpha"));
    tools.insert("beta".to_string(), make_tool("/bin/beta"));
    let config = Config {
        default_platform: None,
        telemetry: TelemetryConfig {
            file: vec![FileTelemetryConfig::default()],
        },
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
default_platform: linux-x86_64
imports:
  - base.yaml
  - path: overrides.yaml
    optional: true
    precedence: after
tools:
  node:
    system_path: /bin/node
"#;
    let config = Config::parse_str(yaml).unwrap();
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
default_platform: linux-x86_64
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
default_platform: linux-x86_64
tools:
  tool_b:
    system_path: /bin/override_b
"#,
    )
    .unwrap();

    let main_path = root.path().join("scrim.yaml");
    fs::write(
        &main_path,
        r#"
default_platform: linux-x86_64
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
    assert_eq!(
        config
            .tools
            .get("tool_a")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/main_a")
    );
    assert_eq!(
        config
            .tools
            .get("tool_b")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/override_b")
    );
    assert_eq!(
        config
            .tools
            .get("tool_c")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/base_c")
    );
}

#[test]
fn test_imports_sequential_order_within_same_precedence() {
    let root = tempdir().unwrap();

    let base1 = root.path().join("base1.yaml");
    fs::write(
        &base1,
        "default_platform: linux-x86_64\ntools:\n  tool1: { system_path: '/bin/base1' }",
    )
    .unwrap();

    let base2 = root.path().join("base2.yaml");
    fs::write(
        &base2,
        "default_platform: linux-x86_64\ntools:\n  tool1: { system_path: '/bin/base2' }",
    )
    .unwrap();

    let main_path = root.path().join("scrim.yaml");
    fs::write(
        &main_path,
        "imports:\n  - base1.yaml\n  - base2.yaml\n",
    )
    .unwrap();

    let config = read_config(&main_path).unwrap();
    assert_eq!(
        config
            .tools
            .get("tool1")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
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
        "default_platform: linux-x86_64\ntools:\n  helper: { system_path: '/bin/helper' }",
    )
    .unwrap();

    let sub_config = sub.join("scrim.yaml");
    fs::write(&sub_config, "imports:\n  - helper.yaml\n").unwrap();

    let config = read_config(&sub_config).unwrap();
    assert_eq!(
        config
            .tools
            .get("helper")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/helper")
    );
}

#[test]
fn test_imports_missing_required_fails() {
    let root = tempdir().unwrap();
    let main_path = root.path().join("scrim.yaml");
    fs::write(&main_path, "imports:\n  - nonexistent.yaml\n").unwrap();

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
default_platform: linux-x86_64
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
        config
            .tools
            .get("node")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
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
        "default_platform: linux-x86_64\nimports:\n  - b.yaml\ntools:\n  a: { system_path: '/bin/a' }",
    )
    .unwrap();

    fs::write(
        &file_b,
        "default_platform: linux-x86_64\nimports:\n  - a.yaml\ntools:\n  b: { system_path: '/bin/b' }",
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

    fs::write(
        &file_d,
        "default_platform: linux-x86_64\ntools:\n  shared_tool: { system_path: '/bin/shared_tool' }",
    )
    .unwrap();

    fs::write(
        &file_b,
        "imports:\n  - d.yaml\ntools:\n  tool_b: { platforms: { linux-x86_64: { system_path: '/bin/tool_b' } } }",
    )
    .unwrap();

    fs::write(
        &file_c,
        "imports:\n  - d.yaml\ntools:\n  tool_c: { platforms: { linux-x86_64: { system_path: '/bin/tool_c' } } }",
    )
    .unwrap();

    fs::write(&file_a, "imports:\n  - b.yaml\n  - c.yaml\n").unwrap();

    let config = read_config(&file_a).expect("Diamond imports should not be flagged as circular");
    assert_eq!(
        config
            .tools
            .get("shared_tool")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/shared_tool")
    );
    assert_eq!(
        config
            .tools
            .get("tool_b")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/tool_b")
    );
    assert_eq!(
        config
            .tools
            .get("tool_c")
            .unwrap()
            .platforms
            .get("linux-x86_64")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/tool_c")
    );
}

#[test]
fn test_format_yaml() {
    let mut tools = BTreeMap::new();
    let mut node_plat = BTreeMap::new();
    node_plat.insert(
        "linux-x86_64".to_string(),
        PlatformToolConfig {
            system_path: Some("/usr/local/bin/node".to_string()),
            ..Default::default()
        },
    );
    tools.insert(
        "node".to_string(),
        ToolConfig {
            platforms: node_plat,
            ..Default::default()
        },
    );

    let mut go_plat = BTreeMap::new();
    go_plat.insert(
        "linux-x86_64".to_string(),
        PlatformToolConfig {
            url: Some("https://go.dev/dl/go1.21.5.linux-amd64.tar.gz".to_string()),
            sha256: Some("285c1f0624022839446d32".to_string()),
            ..Default::default()
        },
    );
    tools.insert(
        "go".to_string(),
        ToolConfig {
            platforms: go_plat,
            ..Default::default()
        },
    );

    let config = Config {
        default_platform: None,
        telemetry: TelemetryConfig {
            file: vec![FileTelemetryConfig::default()],
        },
        imports: Vec::new(),
        tools,
    };

    // Full configuration
    let full_yaml = config.to_yaml().unwrap();
    assert!(full_yaml.contains("telemetry:"));
    assert!(full_yaml.contains("file:"));
    assert!(full_yaml.contains("- {}"));
    assert!(!full_yaml.contains("telemetry: true"));
    assert!(full_yaml.contains("node:"));
    assert!(full_yaml.contains("go:"));
    assert!(full_yaml.contains("platforms:"));

    // Specific tool configuration
    let node_yaml = config.tools.get("node").unwrap().to_yaml().unwrap();
    assert!(node_yaml.contains("linux-x86_64:"));
    assert!(node_yaml.contains("system_path: /usr/local/bin/node"));
}

#[test]
fn test_effective_for_platform() {
    let yaml = r#"
telemetry: false
tools:
  node:
    platforms:
      linux-x86_64:
        system_path: /bin/linux_node
      macos-aarch64:
        system_path: /opt/homebrew/bin/node
  macos_only:
    platforms:
      macos-aarch64:
        system_path: /opt/homebrew/bin/mac_tool
  go:
    archive_path: go/bin/go
    platforms:
      linux-x86_64:
        url: https://go.dev/dl/go.linux.tar.gz
        sha256: linux_hash
      macos-aarch64:
        url: https://go.dev/dl/go.mac.tar.gz
        sha256: mac_hash
  gofmt:
    template: go
    archive_path: go/bin/gofmt
"#;
    let config = Config::parse_str(yaml).unwrap();
    let linux_platform = Platform::parse("linux-x86_64").unwrap();
    let macos_platform = Platform::parse("macos-aarch64").unwrap();
    let arm_linux = Platform::parse("linux-aarch64").unwrap();

    // 1. Linux platform full config:
    // - node has only linux-x86_64
    // - go has only linux-x86_64
    // - gofmt is present
    // - macos_only is omitted
    let eff_linux = config.effective_for_platform(&linux_platform).unwrap();
    let linux_yaml = eff_linux.to_yaml().unwrap();
    assert!(!linux_yaml.contains("telemetry"));
    assert!(linux_yaml.contains("node:"));
    assert!(linux_yaml.contains("/bin/linux_node"));
    assert!(!linux_yaml.contains("/opt/homebrew/bin/node"));
    assert!(!linux_yaml.contains("macos_only:"));
    assert!(linux_yaml.contains("go:"));
    assert!(linux_yaml.contains("linux_hash"));
    assert!(!linux_yaml.contains("mac_hash"));
    assert!(linux_yaml.contains("gofmt:"));

    // Lexicographical ordering check: go < gofmt < node
    let go_pos = linux_yaml.find("go:").unwrap();
    let gofmt_pos = linux_yaml.find("gofmt:").unwrap();
    let node_pos = linux_yaml.find("node:").unwrap();
    assert!(go_pos < gofmt_pos);
    assert!(gofmt_pos < node_pos);

    // 2. Specific tool format for linux:
    let go_eff = config
        .tools
        .get("go")
        .unwrap()
        .effective_for_platform("go", &config, &linux_platform)
        .unwrap()
        .unwrap();
    let go_yaml = go_eff.to_yaml().unwrap();
    assert!(go_yaml.contains("linux-x86_64:"));
    assert!(go_yaml.contains("linux_hash"));
    assert!(!go_yaml.contains("macos-aarch64:"));

    // 3. Tool not configured for platform returns Ok(None)
    let mac_eff = config
        .tools
        .get("macos_only")
        .unwrap()
        .effective_for_platform("macos_only", &config, &linux_platform)
        .unwrap();
    assert!(mac_eff.is_none());

    // 4. Unconfigured platform (linux-aarch64) has empty tools list
    let empty_eff = config.effective_for_platform(&arm_linux).unwrap();
    let empty_yaml = empty_eff.to_yaml().unwrap();
    assert!(!empty_yaml.contains("node:"));
    assert!(!empty_yaml.contains("go:"));
    assert!(!empty_yaml.contains("gofmt:"));
    assert!(!empty_yaml.contains("macos_only:"));

    // 5. macOS platform full config
    let mac_full = config.effective_for_platform(&macos_platform).unwrap();
    let mac_yaml = mac_full.to_yaml().unwrap();
    assert!(mac_yaml.contains("macos_only:"));
    assert!(mac_yaml.contains("/opt/homebrew/bin/mac_tool"));
    assert!(mac_yaml.contains("mac_hash"));
    assert!(!mac_yaml.contains("linux_hash"));
}

#[test]
fn test_lazy_validation_deferred_until_evaluation() {
    let root = tempdir().unwrap();
    // A config with one valid tool and one semantically invalid tool (e.g. url without digest)
    let config_file = root.path().join("scrim.yaml");
    fs::write(
        &config_file,
        r#"
tools:
  valid_tool:
    platforms:
      linux-x86_64:
        system_path: /bin/valid
  broken_tool:
    platforms:
      linux-x86_64:
        url: https://example.com/tool.tar.gz
"#,
    )
    .unwrap();

    // Loading the config succeeds because validation is deferred
    let config = read_config(&config_file).expect("Loading config should succeed lazily");
    assert!(config.tools.contains_key("valid_tool"));
    assert!(config.tools.contains_key("broken_tool"));

    let linux_platform = Platform::parse("linux-x86_64").unwrap();

    // Evaluating valid_tool succeeds
    let valid_eff = config
        .tools
        .get("valid_tool")
        .unwrap()
        .effective_for_platform("valid_tool", &config, &linux_platform);
    assert!(valid_eff.is_ok());

    // Evaluating broken_tool fails with semantic validation error
    let broken_eff = config
        .tools
        .get("broken_tool")
        .unwrap()
        .effective_for_platform("broken_tool", &config, &linux_platform);
    assert!(broken_eff.is_err());
    assert!(broken_eff
        .unwrap_err()
        .contains("specifies 'url' without a digest"));
}

#[test]
fn test_validate_direct_platform_fields_with_archive_path() {
    let tool = ToolConfig {
        url: Some("https://example.com/tool.tar.gz".to_string()),
        sha256: Some("abc".to_string()),
        archive_path: Some("bin/tool".to_string()),
        ..Default::default()
    };
    assert!(
        tool.validate("mytool").is_ok(),
        "Expected tool with direct url, sha256, and archive_path to validate"
    );
}

#[test]
fn test_syntactic_parse_error_tolerance_across_layers() {
    let root = tempdir().unwrap();

    // 1. Lower layer (parent) has valid tool "parent_tool"
    let parent_config = root.path().join("scrim.yaml");
    fs::write(
        &parent_config,
        r#"
tools:
  parent_tool:
    platforms:
      linux-x86_64:
        system_path: /bin/parent
"#,
    )
    .unwrap();

    // 2. Child directory has a completely malformed scrim.yaml (syntax error)
    let child_dir = root.path().join("child");
    fs::create_dir_all(&child_dir).unwrap();
    let child_config = child_dir.join("scrim.yaml");
    fs::write(&child_config, ": this is invalid YAML: [unterminated").unwrap();

    // Loading from child_dir should succeed: child file is discarded with warning,
    // and parent_tool is retained!
    let config = load_config(&child_dir, Some(root.path()), None).expect("Should tolerate syntax error");
    assert!(config.tools.contains_key("parent_tool"));
}

#[test]
fn test_syntactic_parse_error_tolerance_in_imports() {
    let root = tempdir().unwrap();

    // Imported file has bad YAML syntax
    let bad_import = root.path().join("bad_import.yaml");
    fs::write(&bad_import, "not: { valid: yaml: [").unwrap();

    // Main config imports the bad file, but defines main_tool
    let main_config = root.path().join("scrim.yaml");
    fs::write(
        &main_config,
        r#"
imports:
  - bad_import.yaml
tools:
  main_tool:
    platforms:
      linux-x86_64:
        system_path: /bin/main
"#,
    )
    .unwrap();

    let config = read_config(&main_config).expect("Should tolerate syntax error in import");
    assert!(config.tools.contains_key("main_tool"));
}

#[test]
fn test_invalid_default_platform_tolerance() {
    let root = tempdir().unwrap();
    let config_file = root.path().join("scrim.yaml");
    fs::write(
        &config_file,
        r#"
default_platform: invalid_os_arch
tools:
  explicit_tool:
    platforms:
      linux-x86_64:
        system_path: /bin/explicit
  direct_tool:
    system_path: /bin/direct
"#,
    )
    .unwrap();

    let config = read_config(&config_file).expect("Should tolerate invalid default_platform");
    let linux_platform = Platform::parse("linux-x86_64").unwrap();

    // explicit_tool resolves without error
    let explicit_eff = config
        .tools
        .get("explicit_tool")
        .unwrap()
        .effective_for_platform("explicit_tool", &config, &linux_platform);
    assert!(explicit_eff.is_ok());

    // direct_tool fails validation because default_platform was invalid/ignored
    let direct_eff = config
        .tools
        .get("direct_tool")
        .unwrap()
        .effective_for_platform("direct_tool", &config, &linux_platform);
    assert!(direct_eff.is_err());
    assert!(direct_eff
        .unwrap_err()
        .contains("no 'default_platform' is declared"));
}

#[test]
fn test_tool_fallback_layering() {
    let mut lower = Config::default();
    let mut lower_tools = BTreeMap::new();
    lower_tools.insert(
        "node".to_string(),
        ToolConfig {
            system_path: Some("/bin/lower_node".to_string()),
            ..Default::default()
        },
    );
    lower.tools = lower_tools;

    let mut higher = Config::default();
    let mut higher_tools = BTreeMap::new();
    higher_tools.insert(
        "node".to_string(),
        ToolConfig {
            system_path: Some("/bin/higher_node".to_string()),
            ..Default::default()
        },
    );
    higher.tools = higher_tools;

    let merged = lower.merge(higher);
    let node_tool = merged.tools.get("node").unwrap();
    assert_eq!(node_tool.system_path.as_deref(), Some("/bin/higher_node"));
    assert!(node_tool.fallback.is_some());
    assert_eq!(
        node_tool.fallback.as_ref().unwrap().system_path.as_deref(),
        Some("/bin/lower_node")
    );

    let layers: Vec<_> = node_tool
        .layers()
        .map(|l| l.system_path.as_deref())
        .collect();
    assert_eq!(
        layers,
        vec![Some("/bin/higher_node"), Some("/bin/lower_node")]
    );
}

#[test]
fn test_telemetry_config_valid_forms_and_canonical_serialization() {
    // 1. Single-entry default object: `file: {}`
    let cfg_default = Config::parse_str("telemetry:\n  file: {}\n").unwrap();
    assert!(cfg_default.telemetry_enabled());
    assert_eq!(
        cfg_default.telemetry.file,
        vec![FileTelemetryConfig { path: None }]
    );
    let yaml_default = cfg_default.to_yaml().unwrap();
    assert!(yaml_default.contains("telemetry:"));
    assert!(yaml_default.contains("file:"));
    assert!(yaml_default.contains("- {}"));
    assert_eq!(Config::parse_str(&yaml_default).unwrap(), cfg_default);

    // 2. Single-entry string shorthand: `file: /var/log/scrim.log`
    let cfg_str = Config::parse_str("telemetry:\n  file: /var/log/scrim.log\n").unwrap();
    assert_eq!(
        cfg_str.telemetry.file,
        vec![FileTelemetryConfig {
            path: Some("/var/log/scrim.log".to_string())
        }]
    );
    let yaml_str = cfg_str.to_yaml().unwrap();
    assert!(yaml_str.contains("- path: /var/log/scrim.log"));
    assert_eq!(Config::parse_str(&yaml_str).unwrap(), cfg_str);

    // 3. Single-entry structured object: `file: { path: /var/log/scrim.log }`
    let cfg_obj =
        Config::parse_str("telemetry:\n  file:\n    path: /var/log/scrim.log\n").unwrap();
    assert_eq!(cfg_obj, cfg_str);

    // 4. Mixed sequence
    let cfg_seq = Config::parse_str(
        "telemetry:\n  file:\n    - {}\n    - /var/log/a.log\n    - path: /var/log/b.log\n",
    )
    .unwrap();
    assert_eq!(
        cfg_seq.telemetry.file,
        vec![
            FileTelemetryConfig { path: None },
            FileTelemetryConfig {
                path: Some("/var/log/a.log".to_string())
            },
            FileTelemetryConfig {
                path: Some("/var/log/b.log".to_string())
            },
        ]
    );
    let yaml_seq = cfg_seq.to_yaml().unwrap();
    assert_eq!(Config::parse_str(&yaml_seq).unwrap(), cfg_seq);

    // 5. Empty map / empty list -> disabled & omitted from serialization
    let cfg_empty_map = Config::parse_str("telemetry: {}\n").unwrap();
    assert!(!cfg_empty_map.telemetry_enabled());
    assert!(!cfg_empty_map.to_yaml().unwrap().contains("telemetry"));

    let cfg_empty_list = Config::parse_str("telemetry:\n  file: []\n").unwrap();
    assert!(!cfg_empty_list.telemetry_enabled());
    assert!(!cfg_empty_list.to_yaml().unwrap().contains("telemetry"));

    // 6. Deprecated boolean shorthands
    let cfg_true = Config::parse_str("telemetry: true\n").unwrap();
    assert_eq!(cfg_true, cfg_default);
    assert_eq!(cfg_true.to_yaml().unwrap(), yaml_default);

    let cfg_false = Config::parse_str("telemetry: false\n").unwrap();
    assert!(!cfg_false.telemetry_enabled());
    assert!(!cfg_false.to_yaml().unwrap().contains("telemetry"));
}

#[test]
fn test_telemetry_config_rejects_null_and_unknown_fields() {
    for invalid_yaml in [
        "telemetry:\n",
        "telemetry: null\n",
        "telemetry: []\n",
        "telemetry: \"foo\"\n",
        "telemetry: 123\n",
        "telemetry:\n  file:\n",
        "telemetry:\n  file: null\n",
        "telemetry:\n  file: true\n",
        "telemetry:\n  file: 123\n",
        "telemetry:\n  file:\n    -\n",
        "telemetry:\n  file:\n    - null\n",
        "telemetry:\n  file:\n    - 123\n",
        "telemetry:\n  enabled: true\n",
        "telemetry:\n  file: {}\n  file: /tmp/a.log\n",
        "telemetry:\n  file:\n    unknown_key: /tmp/x.log\n",
        "telemetry:\n  file:\n    path: null\n",
        "telemetry:\n  file:\n    path: ~\n",
        "telemetry:\n  file:\n    path:\n",
        "telemetry:\n  file:\n    path: 123\n",
        "telemetry:\n  file:\n    path: true\n",
        "telemetry:\n  file:\n    path: /tmp/a.log\n    path: /tmp/b.log\n",
    ] {
        let res: Result<Config, _> = serde_yaml::from_str(invalid_yaml);
        assert!(
            res.is_err(),
            "Expected YAML to be rejected:\n{}",
            invalid_yaml
        );
    }
}

#[test]
fn test_telemetry_additive_layering_and_imports_deduplication() {
    let root = tempdir().unwrap();

    // User layer sets default file sink
    let xdg_cfg = root.path().join("xdg_config");
    let user_dir = xdg_cfg.join("scrim");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(user_dir.join("scrim.yaml"), "telemetry:\n  file: {}\n").unwrap();

    // Repo root sets /var/log/repo.log and stops upward search via .git
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(
        repo.join("scrim.yaml"),
        "telemetry:\n  file: /var/log/repo.log\n",
    )
    .unwrap();

    // Child sets deprecated `telemetry: false` and `telemetry: {}` via imports; neither disables lower sinks
    let child = repo.join("sub");
    fs::create_dir_all(&child).unwrap();
    fs::write(
        child.join("before.yaml"),
        "telemetry:\n  file:\n    - /var/log/before.log\n    - /var/log/repo.log\n",
    )
    .unwrap();
    fs::write(
        child.join("after.yaml"),
        "telemetry:\n  file:\n    - {}\n    - /var/log/after.log\n",
    )
    .unwrap();
    fs::write(
        child.join("scrim.yaml"),
        "telemetry: false\nimports:\n  - before.yaml\n  - path: after.yaml\n    precedence: after\n",
    )
    .unwrap();

    let config = load_config(&child, Some(root.path()), Some(&xdg_cfg)).unwrap();
    assert_eq!(
        config.telemetry.file,
        vec![
            FileTelemetryConfig { path: None },
            FileTelemetryConfig {
                path: Some("/var/log/repo.log".to_string())
            },
            FileTelemetryConfig {
                path: Some("/var/log/before.log".to_string())
            },
            FileTelemetryConfig {
                path: Some("/var/log/after.log".to_string())
            },
        ]
    );

    // Also confirm `telemetry: {}` in a higher layer does not clear lower sinks
    let empty_override = Config::parse_str("telemetry: {}\n").unwrap();
    let merged = config.clone().merge(empty_override);
    assert_eq!(merged.telemetry, config.telemetry);
}

#[test]
fn test_telemetry_runtime_path_deduplication() {
    let cfg = Config::parse_str(
        "telemetry:\n  file:\n    - {}\n    - /custom/state/scrim/telemetry.log\n    - /var/log//a.log\n    - /var/log/a.log/\n    - /other/telemetry.log\n",
    )
    .unwrap();
    let paths = cfg.telemetry.resolve_file_paths(
        Some(Path::new("/custom/state/")),
        Some(Path::new("/home/user")),
    );
    assert_eq!(
        paths,
        vec![
            PathBuf::from("/custom/state/scrim/telemetry.log"),
            PathBuf::from("/var/log//a.log"),
            PathBuf::from("/other/telemetry.log"),
        ]
    );
}

#[test]
fn test_telemetry_relative_and_empty_paths_fault_tolerance() {
    let dir = tempdir().unwrap();
    let lower_yaml = dir.path().join("lower.yaml");
    let upper_yaml = dir.path().join("upper.yaml");

    fs::write(
        &lower_yaml,
        "telemetry:\n  file:\n    - relative/bad.log\n    - \"\"\n    - /var/log/good1.log\n",
    )
    .unwrap();
    fs::write(
        &upper_yaml,
        "imports:\n  - lower.yaml\ntelemetry:\n  file:\n    - ./also_bad.log\n    - \"null\"\n    - path: \"null\"\n    - \"   \"\n    - /var/log/good2.log\n",
    )
    .unwrap();

    let cfg = read_config(&upper_yaml).unwrap();
    assert_eq!(
        cfg.telemetry.file,
        vec![
            FileTelemetryConfig {
                path: Some("/var/log/good1.log".to_string())
            },
            FileTelemetryConfig {
                path: Some("/var/log/good2.log".to_string())
            },
        ]
    );

    // Also test Config::parse_str discards only invalid sinks
    let parsed = Config::parse_str(
        "telemetry:\n  file:\n    - rel.log\n    - \"\"\n    - \"null\"\n    - \"   \"\n    - {}\n",
    )
    .unwrap();
    assert_eq!(
        parsed.telemetry.file,
        vec![FileTelemetryConfig { path: None }]
    );
}

#[test]
fn test_telemetry_invalid_yaml_syntactic_tolerance_retains_lower_layer() {
    let root = tempdir().unwrap();
    let repo = root.path().join("repo");
    let child = repo.join("sub");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(&child).unwrap();

    fs::write(
        repo.join("scrim.yaml"),
        "telemetry:\n  file: /var/log/parent.log\ntools:\n  mytool:\n    system_path: /bin/echo\n",
    )
    .unwrap();

    // Child has bare-null `file:` -> whole child file is discarded with warning, parent retained
    fs::write(
        child.join("scrim.yaml"),
        "telemetry:\n  file:\ntools:\n  broken:\n    system_path: /bin/true\n",
    )
    .unwrap();

    let cfg = load_config(&child, Some(root.path()), None).unwrap();
    assert!(cfg.tools.contains_key("mytool"));
    assert!(!cfg.tools.contains_key("broken"));
    assert_eq!(
        cfg.telemetry.file,
        vec![FileTelemetryConfig {
            path: Some("/var/log/parent.log".to_string())
        }]
    );

    // Child has unknown key under telemetry -> whole child file discarded, parent retained
    fs::write(
        child.join("scrim.yaml"),
        "telemetry:\n  unknown: true\ntools:\n  broken:\n    system_path: /bin/true\n",
    )
    .unwrap();
    let cfg2 = load_config(&child, Some(root.path()), None).unwrap();
    assert!(cfg2.tools.contains_key("mytool"));
    assert!(!cfg2.tools.contains_key("broken"));
    assert_eq!(
        cfg2.telemetry.file,
        vec![FileTelemetryConfig {
            path: Some("/var/log/parent.log".to_string())
        }]
    );
}

#[test]
fn test_resolve_state_dir_and_default_telemetry_log_path() {
    let home = Path::new("/home/user");
    let xdg_abs = Path::new("/custom/xdg_state");
    let xdg_rel = Path::new("custom/xdg_state");
    let empty_path = Path::new("");

    assert_eq!(
        resolve_state_dir(Some(xdg_abs), Some(home)),
        PathBuf::from("/custom/xdg_state/scrim")
    );
    assert_eq!(
        resolve_default_telemetry_log_path(Some(xdg_abs), Some(home)),
        PathBuf::from("/custom/xdg_state/scrim/telemetry.log")
    );

    // Relative / empty XDG_STATE_HOME fall back to $HOME/.local/state
    assert_eq!(
        resolve_state_dir(Some(xdg_rel), Some(home)),
        PathBuf::from("/home/user/.local/state/scrim")
    );
    assert_eq!(
        resolve_default_telemetry_log_path(Some(empty_path), Some(home)),
        PathBuf::from("/home/user/.local/state/scrim/telemetry.log")
    );

    // None falls back to $HOME/.local/state
    assert_eq!(
        resolve_default_telemetry_log_path(None, Some(home)),
        PathBuf::from("/home/user/.local/state/scrim/telemetry.log")
    );

    // HOME absent falls back to .local/state
    assert_eq!(
        resolve_state_dir(None, None),
        PathBuf::from(".local/state/scrim")
    );
    assert_eq!(
        resolve_default_telemetry_log_path(None, None),
        PathBuf::from(".local/state/scrim/telemetry.log")
    );
}

use resolver::*;
use std::path::PathBuf;
use config::{Config, ToolConfig};
use std::collections::BTreeMap;

#[test]
fn test_resolve_from_config_path() {
    let mut tools = BTreeMap::new();
    tools.insert("node".to_string(), ToolConfig {
        system_path: Some("/usr/bin/node".to_string()),
        ..Default::default()
    });
    let config = Config { 
        tools,
        ..Default::default()
    };

    let resolved = resolve_tool("node", &config);
    assert_eq!(resolved, Ok(Some(Resolution::SystemPath {
        path: PathBuf::from("/usr/bin/node"),
        fallback: None,
    })));
}

#[test]
fn test_resolve_fetch_sha256() {
    let mut tools = BTreeMap::new();
    tools.insert("go".to_string(), ToolConfig {
        url: Some("https://go.dev/dl/go.tar.gz".to_string()),
        sha256: Some("abc12345".to_string()),
        ..Default::default()
    });
    let config = Config { tools, ..Default::default() };

    let resolved = resolve_tool("go", &config);
    assert_eq!(resolved, Ok(Some(Resolution::Fetch(FetchResolution { 
        url: "https://go.dev/dl/go.tar.gz".to_string(),
        digest: Digest { algorithm: DigestAlgorithm::Sha256, value: "abc12345".to_string() },
        archive_path: None,
    }))));
}

#[test]
fn test_resolve_fetch_sha512() {
    let mut tools = BTreeMap::new();
    tools.insert("go".to_string(), ToolConfig {
        url: Some("https://go.dev/dl/go.tar.gz".to_string()),
        sha512: Some("def67890".to_string()),
        ..Default::default()
    });
    let config = Config { tools, ..Default::default() };

    let resolved = resolve_tool("go", &config);
    assert_eq!(resolved, Ok(Some(Resolution::Fetch(FetchResolution { 
        url: "https://go.dev/dl/go.tar.gz".to_string(),
        digest: Digest { algorithm: DigestAlgorithm::Sha512, value: "def67890".to_string() },
        archive_path: None,
    }))));
}

#[test]
fn test_resolve_template() {
    let mut tools = BTreeMap::new();
    tools.insert("go".to_string(), ToolConfig {
        url: Some("https://go.dev/dl/go.tar.gz".to_string()),
        sha256: Some("abc12345".to_string()),
        archive_path: Some("go/bin/go".to_string()),
        ..Default::default()
    });
    tools.insert("gofmt".to_string(), ToolConfig {
        archive_path: Some("go/bin/gofmt".to_string()),
        template: Some("go".to_string()),
        ..Default::default()
    });
    let config = Config { tools, ..Default::default() };

    let resolved = resolve_tool("gofmt", &config);
    assert_eq!(resolved, Ok(Some(Resolution::Fetch(FetchResolution { 
        url: "https://go.dev/dl/go.tar.gz".to_string(),
        digest: Digest { algorithm: DigestAlgorithm::Sha256, value: "abc12345".to_string() },
        archive_path: Some("go/bin/gofmt".to_string()),
    }))));
}

#[test]
fn test_resolve_template_chain_disallowed() {
    let mut tools = BTreeMap::new();
    tools.insert("go".to_string(), ToolConfig {
        url: Some("https://go.dev/dl/go.tar.gz".to_string()),
        sha256: Some("abc12345".to_string()),
        archive_path: Some("go/bin/go".to_string()),
        ..Default::default()
    });
    tools.insert("gofmt".to_string(), ToolConfig {
        archive_path: Some("go/bin/gofmt".to_string()),
        template: Some("go".to_string()),
        ..Default::default()
    });
    tools.insert("go-chained".to_string(), ToolConfig {
        archive_path: Some("go/bin/go-chained".to_string()),
        template: Some("gofmt".to_string()),
        ..Default::default()
    });
    let config = Config { tools, ..Default::default() };

    let resolved = resolve_tool("go-chained", &config);
    assert_eq!(resolved, Err("Tool 'go-chained' references template 'gofmt', which is also a template. Template chaining is not allowed.".to_string()));
}

#[test]
fn test_resolve_validation_errors() {
    let mut tools = BTreeMap::new();
    // Path with invalid fetch property (url without digest)
    tools.insert("err_path_missing_digest".to_string(), ToolConfig {
        system_path: Some("/bin/node".to_string()),
        url: Some("http://example.com/node.tar.gz".to_string()),
        ..Default::default()
    });
    // Template with url
    tools.insert("err_template".to_string(), ToolConfig {
        url: Some("http://example.com".to_string()),
        template: Some("go".to_string()),
        ..Default::default()
    });
    // Missing digest
    tools.insert("err_sha".to_string(), ToolConfig {
        url: Some("http://example.com".to_string()),
        ..Default::default()
    });
    // Multiple digests
    tools.insert("err_multi_digest".to_string(), ToolConfig {
        url: Some("http://example.com".to_string()),
        sha256: Some("sha256val".to_string()),
        sha512: Some("sha512val".to_string()),
        ..Default::default()
    });
    // Digest without url
    tools.insert("err_digest_no_url".to_string(), ToolConfig {
        sha512: Some("sha512val".to_string()),
        ..Default::default()
    });
    // archive_path without url or template
    tools.insert("err_archive".to_string(), ToolConfig {
        archive_path: Some("bin/foo".to_string()),
        ..Default::default()
    });
    // archive_path with template that doesn't have url
    tools.insert("err_archive_template".to_string(), ToolConfig {
        archive_path: Some("bin/foo".to_string()),
        template: Some("local_tool".to_string()),
        ..Default::default()
    });
    tools.insert("local_tool".to_string(), ToolConfig {
        system_path: Some("/bin/local".to_string()),
        ..Default::default()
    });

    let config = Config { tools, ..Default::default() };

    assert_eq!(
        resolve_tool("err_path_missing_digest", &config),
        Err("Tool 'err_path_missing_digest' specifies 'url' without a digest. Both must be provided together.".to_string())
    );
    assert_eq!(
        resolve_tool("err_template", &config),
        Err("Tool 'err_template' specifies 'template' which is mutually exclusive with 'url' and digests.".to_string())
    );
    assert_eq!(
        resolve_tool("err_sha", &config),
        Err("Tool 'err_sha' specifies 'url' without a digest. Both must be provided together.".to_string())
    );
    assert_eq!(
        resolve_tool("err_multi_digest", &config),
        Err("Tool 'err_multi_digest' specifies multiple digests. Exactly one digest algorithm must be specified.".to_string())
    );
    assert_eq!(
        resolve_tool("err_digest_no_url", &config),
        Err("Tool 'err_digest_no_url' specifies 'sha512' without 'url'. Both must be provided together.".to_string())
    );
    assert_eq!(
        resolve_tool("err_archive", &config),
        Err("Tool 'err_archive' specifies 'archive_path' but provides no 'url' or 'template'.".to_string())
    );
    assert_eq!(
        resolve_tool("err_archive_template", &config),
        Err("Tool 'err_archive_template' specifies 'archive_path' but the referenced template does not provide a fetchable archive.".to_string())
    );

    let mut tools2 = BTreeMap::new();
    tools2.insert("rel_path_tool".to_string(), ToolConfig {
        system_path: Some("./bin/relative".to_string()),
        ..Default::default()
    });
    let config_rel = Config { tools: tools2, ..Default::default() };
    assert_eq!(
        resolve_tool("rel_path_tool", &config_rel),
        Err("Tool 'rel_path_tool' specifies relative system_path './bin/relative'. 'system_path' must be an absolute path.".to_string())
    );

    let mut tools3 = BTreeMap::new();
    tools3.insert("empty_tool".to_string(), ToolConfig {
        ..Default::default()
    });
    let config2 = Config { tools: tools3, ..Default::default() };
    
    assert_eq!(
        resolve_tool("empty_tool", &config2),
        Err("Tool 'empty_tool' has an empty configuration. Every tool must specify a 'system_path', 'url', or 'template'.".to_string())
    );
}

#[test]
fn test_resolve_system_path_with_fetch_fallback() {
    let mut tools = BTreeMap::new();
    // 1. Tool where both system_path and fetch are defined
    tools.insert("tool_with_fallback".to_string(), ToolConfig {
        system_path: Some("/bin/custom_tool".to_string()),
        url: Some("http://example.com/tool.tar.gz".to_string()),
        sha256: Some("sha256val".to_string()),
        archive_path: Some("bin/tool".to_string()),
        ..Default::default()
    });

    // 2. Tool where system_path is defined with a template
    tools.insert("tool_template_base".to_string(), ToolConfig {
        url: Some("http://example.com/base.tar.gz".to_string()),
        sha256: Some("base_sha256".to_string()),
        ..Default::default()
    });
    tools.insert("tool_template_fallback".to_string(), ToolConfig {
        system_path: Some("/bin/subtool".to_string()),
        archive_path: Some("bin/subtool".to_string()),
        template: Some("tool_template_base".to_string()),
        ..Default::default()
    });

    let config = Config { tools, ..Default::default() };

    // Verification 1: Both defined -> SystemPath with fallback
    assert_eq!(
        resolve_tool("tool_with_fallback", &config),
        Ok(Some(Resolution::SystemPath {
            path: PathBuf::from("/bin/custom_tool"),
            fallback: Some(FetchResolution {
                url: "http://example.com/tool.tar.gz".to_string(),
                digest: Digest {
                    algorithm: DigestAlgorithm::Sha256,
                    value: "sha256val".to_string(),
                },
                archive_path: Some("bin/tool".to_string()),
            }),
        }))
    );

    // Verification 2: System path with template -> SystemPath with fallback
    assert_eq!(
        resolve_tool("tool_template_fallback", &config),
        Ok(Some(Resolution::SystemPath {
            path: PathBuf::from("/bin/subtool"),
            fallback: Some(FetchResolution {
                url: "http://example.com/base.tar.gz".to_string(),
                digest: Digest {
                    algorithm: DigestAlgorithm::Sha256,
                    value: "base_sha256".to_string(),
                },
                archive_path: Some("bin/subtool".to_string()),
            }),
        }))
    );
}

#[test]
fn test_resolve_tool_per_platform() {
    let yaml = r#"
tools:
  node:
    platforms:
      linux-x86_64:
        system_path: /bin/linux_node
      macos-aarch64:
        system_path: /opt/homebrew/bin/node
  go:
    platforms:
      linux-x86_64:
        url: https://go.dev/dl/go.linux.tar.gz
        sha256: 1234
      macos-aarch64:
        url: https://go.dev/dl/go.darwin.tar.gz
        sha256: 5678
  gofmt:
    template: go
    platforms:
      linux-x86_64:
        archive_path: go/bin/gofmt-linux
      macos-aarch64:
        archive_path: go/bin/gofmt-darwin
"#;
    let config = Config::parse_str(yaml).unwrap();
    let linux_x86_64 = config::Platform::parse("linux-x86_64").unwrap();
    let macos_aarch64 = config::Platform::parse("macos-aarch64").unwrap();
    let linux_aarch64 = config::Platform::parse("linux-aarch64").unwrap();

    // 1. Exact platform resolution for node
    assert_eq!(
        resolve_tool_for_platform("node", &config, &linux_x86_64),
        Ok(Some(Resolution::SystemPath {
            path: PathBuf::from("/bin/linux_node"),
            fallback: None,
        }))
    );
    assert_eq!(
        resolve_tool_for_platform("node", &config, &macos_aarch64),
        Ok(Some(Resolution::SystemPath {
            path: PathBuf::from("/opt/homebrew/bin/node"),
            fallback: None,
        }))
    );
    // Unconfigured platform returns Ok(None)
    assert_eq!(
        resolve_tool_for_platform("node", &config, &linux_aarch64),
        Ok(None)
    );

    // 2. Templated tool with platform-level archive_path override
    assert_eq!(
        resolve_tool_for_platform("gofmt", &config, &linux_x86_64),
        Ok(Some(Resolution::Fetch(FetchResolution {
            url: "https://go.dev/dl/go.linux.tar.gz".to_string(),
            digest: Digest {
                algorithm: DigestAlgorithm::Sha256,
                value: "1234".to_string(),
            },
            archive_path: Some("go/bin/gofmt-linux".to_string()),
        })))
    );
    assert_eq!(
        resolve_tool_for_platform("gofmt", &config, &macos_aarch64),
        Ok(Some(Resolution::Fetch(FetchResolution {
            url: "https://go.dev/dl/go.darwin.tar.gz".to_string(),
            digest: Digest {
                algorithm: DigestAlgorithm::Sha256,
                value: "5678".to_string(),
            },
            archive_path: Some("go/bin/gofmt-darwin".to_string()),
        })))
    );
    assert_eq!(
        resolve_tool_for_platform("gofmt", &config, &linux_aarch64),
        Ok(None)
    );
}

#[test]
fn test_resolve_tool_validates_platform_syntax() {
    use config::PlatformToolConfig;
    let mut tools = BTreeMap::new();
    let mut platforms = BTreeMap::new();
    platforms.insert(
        "bad_platform_name".to_string(),
        PlatformToolConfig {
            system_path: Some("/bin/ls".to_string()),
            ..Default::default()
        },
    );
    tools.insert(
        "broken_tool".to_string(),
        ToolConfig {
            platforms,
            ..Default::default()
        },
    );
    let config = Config {
        tools,
        ..Default::default()
    };
    let host = config::Platform::host().unwrap();
    let result = resolve_tool_for_platform("broken_tool", &config, &host);
    assert!(
        result.is_err(),
        "Expected error for invalid platform syntax, got {:?}",
        result
    );
    assert!(result.unwrap_err().contains("Invalid platform"));
}

#[test]
fn test_resolve_tool_validates_referenced_template() {
    let mut tools = BTreeMap::new();
    // Template has missing digest (semantic defect)
    tools.insert(
        "tmpl".to_string(),
        ToolConfig {
            url: Some("https://example.com/tool.tar.gz".to_string()),
            ..Default::default()
        },
    );
    tools.insert(
        "app".to_string(),
        ToolConfig {
            template: Some("tmpl".to_string()),
            ..Default::default()
        },
    );
    let config = Config {
        tools,
        ..Default::default()
    };
    let host = config::Platform::host().unwrap();
    let result = resolve_tool_for_platform("app", &config, &host);
    assert!(
        result.is_err(),
        "Expected error validating broken template, got {:?}",
        result
    );
    assert!(result.unwrap_err().contains("specifies 'url' without a digest"));
}

#[test]
fn test_resolve_effective_tool_for_platform_with_fallback() {
    let mut lower_tools = BTreeMap::new();
    lower_tools.insert(
        "node".to_string(),
        ToolConfig {
            system_path: Some("/bin/lower_node".to_string()),
            ..Default::default()
        },
    );
    let lower = Config {
        tools: lower_tools,
        ..Default::default()
    };

    let mut higher_tools = BTreeMap::new();
    // Broken node in higher layer (missing digest)
    higher_tools.insert(
        "node".to_string(),
        ToolConfig {
            url: Some("https://example.com/broken.tar.gz".to_string()),
            ..Default::default()
        },
    );
    let higher = Config {
        tools: higher_tools,
        ..Default::default()
    };

    let merged = lower.merge(higher);
    let host = config::Platform::host().unwrap();

    // 1. Without allow_fallback: fails with semantic error
    let err = resolve_effective_tool_for_platform("node", &merged, &host, false);
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("specifies 'url' without a digest"));

    // 2. With allow_fallback: falls back to lower_node
    let res = resolve_effective_tool_for_platform("node", &merged, &host, true).unwrap();
    assert!(res.is_some());
    let eff = res.unwrap();
    assert_eq!(
        eff.platforms.get("any").unwrap().system_path.as_deref(),
        Some("/bin/lower_node")
    );
}

#[test]
fn test_resolve_effective_config_for_platform_with_fallback() {
    let mut lower_tools = BTreeMap::new();
    lower_tools.insert(
        "node".to_string(),
        ToolConfig {
            system_path: Some("/bin/lower_node".to_string()),
            ..Default::default()
        },
    );
    let lower = Config {
        tools: lower_tools,
        ..Default::default()
    };

    let mut higher_tools = BTreeMap::new();
    higher_tools.insert(
        "good_tool".to_string(),
        ToolConfig {
            system_path: Some("/bin/good".to_string()),
            ..Default::default()
        },
    );
    // Broken tool without fallback
    higher_tools.insert(
        "broken_tool".to_string(),
        ToolConfig {
            url: Some("https://example.com/bad.tar.gz".to_string()),
            ..Default::default()
        },
    );
    // Broken node with fallback in lower
    higher_tools.insert(
        "node".to_string(),
        ToolConfig {
            url: Some("https://example.com/bad_node.tar.gz".to_string()),
            ..Default::default()
        },
    );
    let higher = Config {
        tools: higher_tools,
        ..Default::default()
    };

    let merged = lower.merge(higher);
    let host = config::Platform::host().unwrap();

    // 1. Without allow_fallback: broken_tool and node report errors, good_tool is in config
    let res_no_fallback = resolve_effective_config_for_platform(&merged, &host, false);
    assert!(res_no_fallback.config.tools.contains_key("good_tool"));
    assert!(!res_no_fallback.config.tools.contains_key("broken_tool"));
    assert!(!res_no_fallback.config.tools.contains_key("node"));
    assert_eq!(res_no_fallback.errors.len(), 2);

    // 2. With allow_fallback: broken_tool omitted, node recovered from lower, errors is empty
    let res_fallback = resolve_effective_config_for_platform(&merged, &host, true);
    assert!(res_fallback.config.tools.contains_key("good_tool"));
    assert!(!res_fallback.config.tools.contains_key("broken_tool"));
    assert!(res_fallback.config.tools.contains_key("node"));
    assert_eq!(
        res_fallback
            .config
            .tools
            .get("node")
            .unwrap()
            .platforms
            .get("any")
            .unwrap()
            .system_path
            .as_deref(),
        Some("/bin/lower_node")
    );
    assert!(res_fallback.errors.is_empty());
}

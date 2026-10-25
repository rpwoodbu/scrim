use std::path::PathBuf;
use config::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestAlgorithm {
    Sha256,
    Sha512,
}

impl DigestAlgorithm {
    pub fn name(&self) -> &'static str {
        match self {
            DigestAlgorithm::Sha256 => "sha256",
            DigestAlgorithm::Sha512 => "sha512",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResolution {
    pub url: String,
    pub digest: Digest,
    pub archive_path: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Resolution {
    SystemPath {
        path: PathBuf,
        fallback: Option<FetchResolution>,
    },
    Fetch(FetchResolution),
}

pub fn resolve_tool(program_name: &str, config: &Config) -> Result<Option<Resolution>, String> {
    let host = config::Platform::host()?;
    resolve_tool_for_platform(program_name, config, &host)
}

pub fn resolve_tool_for_platform(
    program_name: &str,
    config: &Config,
    platform: &config::Platform,
) -> Result<Option<Resolution>, String> {
    if let Some(tool_config) = config.tools.get(program_name) {
        let plat_opt = tool_config.get_platform(platform);

        // If the tool has explicit platform blocks and none match the target platform (and no template),
        // it is unconfigured for this platform.
        if !tool_config.platforms.is_empty() && plat_opt.is_none() && tool_config.template.is_none() {
            return Ok(None);
        }

        let tool_system_path = plat_opt
            .and_then(|p| p.system_path.as_deref())
            .or(tool_config.system_path.as_deref());
        let mut resolved_url = plat_opt
            .and_then(|p| p.url.as_deref())
            .or(tool_config.url.as_deref())
            .map(|s| s.to_string());
        let mut resolved_sha256 = plat_opt
            .and_then(|p| p.sha256.as_deref())
            .or(tool_config.sha256.as_deref())
            .map(|s| s.to_string());
        let mut resolved_sha512 = plat_opt
            .and_then(|p| p.sha512.as_deref())
            .or(tool_config.sha512.as_deref())
            .map(|s| s.to_string());
        let mut resolved_archive_path = plat_opt
            .and_then(|p| p.archive_path.as_deref())
            .or(tool_config.archive_path.as_deref())
            .map(|s| s.to_string());

        let has_template = tool_config.template.is_some();

        // Mutually exclusive template with url and digests
        if has_template
            && (resolved_url.is_some() || resolved_sha256.is_some() || resolved_sha512.is_some())
        {
            return Err(format!(
                "Tool '{}' specifies 'template' which is mutually exclusive with 'url' and digests.",
                program_name
            ));
        }

        if let Some(template_name) = &tool_config.template {
            if let Some(template_config) = config.tools.get(template_name) {
                if template_config.template.is_none() {
                    let template_plat_opt = template_config.get_platform(platform);
                    if !template_config.platforms.is_empty() && template_plat_opt.is_none() {
                        return Ok(None);
                    }

                    let tmpl_url = template_plat_opt
                        .and_then(|p| p.url.as_deref())
                        .or(template_config.url.as_deref());
                    let tmpl_sha256 = template_plat_opt
                        .and_then(|p| p.sha256.as_deref())
                        .or(template_config.sha256.as_deref());
                    let tmpl_sha512 = template_plat_opt
                        .and_then(|p| p.sha512.as_deref())
                        .or(template_config.sha512.as_deref());
                    let tmpl_archive_path = template_plat_opt
                        .and_then(|p| p.archive_path.as_deref())
                        .or(template_config.archive_path.as_deref());

                    if resolved_url.is_none() {
                        resolved_url = tmpl_url.map(|s| s.to_string());
                    }
                    if resolved_sha256.is_none() {
                        resolved_sha256 = tmpl_sha256.map(|s| s.to_string());
                    }
                    if resolved_sha512.is_none() {
                        resolved_sha512 = tmpl_sha512.map(|s| s.to_string());
                    }
                    if resolved_archive_path.is_none() {
                        resolved_archive_path = tmpl_archive_path.map(|s| s.to_string());
                    }
                } else {
                    return Err(format!(
                        "Tool '{}' references template '{}', which is also a template. Template chaining is not allowed.",
                        program_name, template_name
                    ));
                }
            } else {
                return Err(format!(
                    "Tool '{}' references template '{}', which was not found.",
                    program_name, template_name
                ));
            }
        }

        let has_system_path = tool_system_path.is_some();
        let has_url = resolved_url.is_some();
        let has_sha256 = resolved_sha256.is_some();
        let has_sha512 = resolved_sha512.is_some();
        let has_digest = has_sha256 || has_sha512;
        let has_archive_path = resolved_archive_path.is_some();

        // Rule 3: digests are mutually exclusive
        if has_sha256 && has_sha512 {
            return Err(format!(
                "Tool '{}' specifies multiple digests. Exactly one digest algorithm must be specified.",
                program_name
            ));
        }

        // Rule 4: `url` and digest must both be present or both absent
        if has_url != has_digest {
            if has_url {
                return Err(format!(
                    "Tool '{}' specifies 'url' without a digest. Both must be provided together.",
                    program_name
                ));
            } else {
                let digest_name = if has_sha256 { "sha256" } else { "sha512" };
                return Err(format!(
                    "Tool '{}' specifies '{}' without 'url'. Both must be provided together.",
                    program_name, digest_name
                ));
            }
        }

        // Rule 5: `archive_path` requires `url` or `template`
        if has_archive_path && !has_url && !has_template {
            return Err(format!(
                "Tool '{}' specifies 'archive_path' but provides no 'url' or 'template'.",
                program_name
            ));
        }

        // Rule 6: `system_path` must be an absolute path
        if let Some(path) = tool_system_path {
            if !std::path::Path::new(path).is_absolute() {
                return Err(format!(
                    "Tool '{}' specifies relative system_path '{}'. 'system_path' must be an absolute path.",
                    program_name, path
                ));
            }
        }

        // Rule 7: Empty tool configurations are invalid
        if !has_system_path && !has_url && !has_template {
            return Err(format!(
                "Tool '{}' has an empty configuration. Every tool must specify a 'system_path', 'url', or 'template'.",
                program_name
            ));
        }

        // If only system_path is specified, resolve to SystemPath with no fallback
        if let (Some(path), false) = (tool_system_path, has_url) {
            return Ok(Some(Resolution::SystemPath {
                path: PathBuf::from(path),
                fallback: None,
            }));
        }

        let resolved_digest = if let Some(v) = resolved_sha256 {
            Some(Digest {
                algorithm: DigestAlgorithm::Sha256,
                value: v,
            })
        } else if let Some(v) = resolved_sha512 {
            Some(Digest {
                algorithm: DigestAlgorithm::Sha512,
                value: v,
            })
        } else {
            None
        };

        if let (Some(url), Some(digest)) = (resolved_url, resolved_digest) {
            let fetch = FetchResolution {
                url,
                digest,
                archive_path: resolved_archive_path,
            };
            if let Some(path) = tool_system_path {
                return Ok(Some(Resolution::SystemPath {
                    path: PathBuf::from(path),
                    fallback: Some(fetch),
                }));
            } else {
                return Ok(Some(Resolution::Fetch(fetch)));
            }
        } else if has_archive_path {
            return Err(format!(
                "Tool '{}' specifies 'archive_path' but the referenced template does not provide a fetchable archive.",
                program_name
            ));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            telemetry: Some(true),
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
        let config = Config { tools, telemetry: Some(true), ..Default::default() };

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
        let config = Config { tools, telemetry: Some(true), ..Default::default() };

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
        let config = Config { tools, telemetry: Some(true), ..Default::default() };

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
        let config = Config { tools, telemetry: Some(true), ..Default::default() };

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

        let config = Config { tools, telemetry: Some(true), ..Default::default() };

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
        let config_rel = Config { tools: tools2, telemetry: Some(true), ..Default::default() };
        assert_eq!(
            resolve_tool("rel_path_tool", &config_rel),
            Err("Tool 'rel_path_tool' specifies relative system_path './bin/relative'. 'system_path' must be an absolute path.".to_string())
        );

        let mut tools3 = BTreeMap::new();
        tools3.insert("empty_tool".to_string(), ToolConfig {
            ..Default::default()
        });
        let config2 = Config { tools: tools3, telemetry: Some(true), ..Default::default() };
        
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

        let config = Config { tools, telemetry: Some(false), ..Default::default() };

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
}

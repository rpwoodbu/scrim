use std::path::{Path, PathBuf};
use crate::config::Config;

#[derive(Debug, PartialEq)]
pub enum Resolution {
    LocalPath(PathBuf),
    Fetch { url: String, sha256: String, archive_path: Option<String> },
}

pub fn resolve_tool(program_name: &str, config: Option<&Config>, path_env: &str, current_exe: &Path) -> Result<Option<Resolution>, String> {
    if let Some(config) = config {
        if let Some(tool_config) = config.tools.get(program_name) {
            let has_system_path = tool_config.system_path.is_some();
            let has_url = tool_config.url.is_some();
            let has_sha256 = tool_config.sha256.is_some();
            let has_template = tool_config.template.is_some();
            let has_archive_path = tool_config.archive_path.is_some();

            // Rule 1: `system_path` is mutually exclusive with all fetch-related properties
            if has_system_path && (has_url || has_sha256 || has_template || has_archive_path) {
                return Err(format!("Tool '{}' specifies 'system_path' which is mutually exclusive with fetch-related properties.", program_name));
            }

            // Rule 2: `template` is mutually exclusive with `url` and `sha256`
            if has_template && (has_url || has_sha256) {
                return Err(format!("Tool '{}' specifies 'template' which is mutually exclusive with 'url' and 'sha256'.", program_name));
            }

            // Rule 3: `url` and `sha256` must both be present or both absent
            if has_url != has_sha256 {
                return Err(format!("Tool '{}' specifies '{}' without '{}'. Both must be provided together.", 
                    program_name, 
                    if has_url { "url" } else { "sha256" },
                    if has_url { "sha256" } else { "url" }
                ));
            }

            // Rule 4: `archive_path` requires `url` or `template`
            if has_archive_path && !has_url && !has_template {
                return Err(format!("Tool '{}' specifies 'archive_path' but provides no 'url' or 'template'.", program_name));
            }

            if let Some(path) = &tool_config.system_path {
                return Ok(Some(Resolution::LocalPath(PathBuf::from(path))));
            }

            let mut resolved_url = tool_config.url.clone();
            let mut resolved_sha256 = tool_config.sha256.clone();

            if let Some(template_name) = &tool_config.template {
                if let Some(template_config) = config.tools.get(template_name) {
                    if template_config.template.is_none() {
                        resolved_url = template_config.url.clone();
                        resolved_sha256 = template_config.sha256.clone();
                    } else {
                        return Err(format!("Tool '{}' references template '{}', which is also a template. Template chaining is not allowed.", program_name, template_name));
                    }
                } else {
                    return Err(format!("Tool '{}' references template '{}', which was not found.", program_name, template_name));
                }
            }

            if let (Some(url), Some(sha256)) = (resolved_url, resolved_sha256) {
                return Ok(Some(Resolution::Fetch { 
                    url, 
                    sha256,
                    archive_path: tool_config.archive_path.clone(),
                }));
            } else if has_archive_path {
                return Err(format!("Tool '{}' specifies 'archive_path' but the referenced template does not provide a fetchable archive.", program_name));
            }
        }
    }

    // Fallback: Search PATH
    Ok(find_in_path(program_name, path_env, current_exe).map(Resolution::LocalPath))
}

fn find_in_path(program_name: &str, path_env: &str, current_exe: &Path) -> Option<PathBuf> {
    for dir in path_env.split(':') {
        if dir.is_empty() { continue; }
        let path = Path::new(dir).join(program_name);
        if path.is_file() {
            // Check if this is the same file as the current executable (via device/inode)
            if is_same_file(&path, current_exe) {
                continue;
            }
            return Some(path);
        }
    }
    None
}

fn is_same_file(path1: &Path, path2: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(m1), Ok(m2)) = (path1.metadata(), path2.metadata()) {
            return m1.dev() == m2.dev() && m1.ino() == m2.ino();
        }
    }
    // Fallback to canonical path comparison
    if let (Ok(p1), Ok(p2)) = (path1.canonicalize(), path2.canonicalize()) {
        return p1 == p2;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ToolConfig};
    use std::collections::HashMap;

    #[test]
    fn test_resolve_from_config_path() {
        let mut tools = HashMap::new();
        tools.insert("node".to_string(), ToolConfig {
            system_path: Some("/usr/bin/node".to_string()),
            url: None,
            sha256: None,
            archive_path: None,
            template: None,
        });
        let config = Config { 
            tools,
            telemetry: true,
        };

        let resolved = resolve_tool("node", Some(&config), "", Path::new("/bin/scrim"));
        assert_eq!(resolved, Ok(Some(Resolution::LocalPath(PathBuf::from("/usr/bin/node")))));
    }

    #[test]
    fn test_resolve_fetch() {
        let mut tools = HashMap::new();
        tools.insert("go".to_string(), ToolConfig {
            system_path: None,
            url: Some("https://go.dev/dl/go.tar.gz".to_string()),
            sha256: Some("abc12345".to_string()),
            archive_path: None,
            template: None,
        });
        let config = Config { tools, telemetry: true };

        let resolved = resolve_tool("go", Some(&config), "", Path::new("/bin/scrim"));
        assert_eq!(resolved, Ok(Some(Resolution::Fetch { 
            url: "https://go.dev/dl/go.tar.gz".to_string(),
            sha256: "abc12345".to_string(),
            archive_path: None,
        })));
    }

    #[test]
    fn test_resolve_template() {
        let mut tools = HashMap::new();
        tools.insert("go".to_string(), ToolConfig {
            system_path: None,
            url: Some("https://go.dev/dl/go.tar.gz".to_string()),
            sha256: Some("abc12345".to_string()),
            archive_path: Some("go/bin/go".to_string()),
            template: None,
        });
        tools.insert("gofmt".to_string(), ToolConfig {
            system_path: None,
            url: None,
            sha256: None,
            archive_path: Some("go/bin/gofmt".to_string()),
            template: Some("go".to_string()),
        });
        let config = Config { tools, telemetry: true };

        let resolved = resolve_tool("gofmt", Some(&config), "", Path::new("/bin/scrim"));
        assert_eq!(resolved, Ok(Some(Resolution::Fetch { 
            url: "https://go.dev/dl/go.tar.gz".to_string(),
            sha256: "abc12345".to_string(),
            archive_path: Some("go/bin/gofmt".to_string()),
        })));
    }

    #[test]
    fn test_resolve_template_chain_disallowed() {
        let mut tools = HashMap::new();
        tools.insert("go".to_string(), ToolConfig {
            system_path: None,
            url: Some("https://go.dev/dl/go.tar.gz".to_string()),
            sha256: Some("abc12345".to_string()),
            archive_path: Some("go/bin/go".to_string()),
            template: None,
        });
        tools.insert("gofmt".to_string(), ToolConfig {
            system_path: None,
            url: None,
            sha256: None,
            archive_path: Some("go/bin/gofmt".to_string()),
            template: Some("go".to_string()),
        });
        tools.insert("go-chained".to_string(), ToolConfig {
            system_path: None,
            url: None,
            sha256: None,
            archive_path: Some("go/bin/go-chained".to_string()),
            template: Some("gofmt".to_string()),
        });
        let config = Config { tools, telemetry: true };

        let resolved = resolve_tool("go-chained", Some(&config), "", Path::new("/bin/scrim"));
        assert_eq!(resolved, Err("Tool 'go-chained' references template 'gofmt', which is also a template. Template chaining is not allowed.".to_string()));
    }

    #[test]
    fn test_resolve_validation_errors() {
        let mut tools = HashMap::new();
        // Path with fetch-related property
        tools.insert("err_path".to_string(), ToolConfig {
            system_path: Some("/bin/node".to_string()),
            url: Some("http://example.com/node.tar.gz".to_string()),
            sha256: None,
            archive_path: None,
            template: None,
        });
        // Template with url
        tools.insert("err_template".to_string(), ToolConfig {
            system_path: None,
            url: Some("http://example.com".to_string()),
            sha256: None,
            archive_path: None,
            template: Some("go".to_string()),
        });
        // Missing sha256
        tools.insert("err_sha".to_string(), ToolConfig {
            system_path: None,
            url: Some("http://example.com".to_string()),
            sha256: None,
            archive_path: None,
            template: None,
        });
        // archive_path without url or template
        tools.insert("err_archive".to_string(), ToolConfig {
            system_path: None,
            url: None,
            sha256: None,
            archive_path: Some("bin/foo".to_string()),
            template: None,
        });
        // archive_path with template that doesn't have url
        tools.insert("err_archive_template".to_string(), ToolConfig {
            system_path: None,
            url: None,
            sha256: None,
            archive_path: Some("bin/foo".to_string()),
            template: Some("local_tool".to_string()),
        });
        tools.insert("local_tool".to_string(), ToolConfig {
            system_path: Some("/bin/local".to_string()),
            url: None,
            sha256: None,
            archive_path: None,
            template: None,
        });

        let config = Config { tools, telemetry: true };

        assert_eq!(
            resolve_tool("err_path", Some(&config), "", Path::new("/bin/scrim")),
            Err("Tool 'err_path' specifies 'system_path' which is mutually exclusive with fetch-related properties.".to_string())
        );
        assert_eq!(
            resolve_tool("err_template", Some(&config), "", Path::new("/bin/scrim")),
            Err("Tool 'err_template' specifies 'template' which is mutually exclusive with 'url' and 'sha256'.".to_string())
        );
        assert_eq!(
            resolve_tool("err_sha", Some(&config), "", Path::new("/bin/scrim")),
            Err("Tool 'err_sha' specifies 'url' without 'sha256'. Both must be provided together.".to_string())
        );
        assert_eq!(
            resolve_tool("err_archive", Some(&config), "", Path::new("/bin/scrim")),
            Err("Tool 'err_archive' specifies 'archive_path' but provides no 'url' or 'template'.".to_string())
        );
        assert_eq!(
            resolve_tool("err_archive_template", Some(&config), "", Path::new("/bin/scrim")),
            Err("Tool 'err_archive_template' specifies 'archive_path' but the referenced template does not provide a fetchable archive.".to_string())
        );
    }
}

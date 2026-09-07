use std::path::{Path, PathBuf};
use crate::config::Config;

#[derive(Debug, PartialEq)]
pub enum Resolution {
    LocalPath(PathBuf),
    Fetch { url: String, sha256: String, archive_bin: Option<String> },
}

pub fn resolve_tool(program_name: &str, config: Option<&Config>, path_env: &str, current_exe: &Path) -> Result<Option<Resolution>, String> {
    if let Some(config) = config {
        if let Some(tool_config) = config.tools.get(program_name) {
            if let Some(path) = &tool_config.path {
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
                        return Err(format!("Error: Tool '{}' references template '{}', which is also a template. Template chaining is not allowed.", program_name, template_name));
                    }
                } else {
                    return Err(format!("Error: Tool '{}' references template '{}', which was not found.", program_name, template_name));
                }
            }

            if let (Some(url), Some(sha256)) = (resolved_url, resolved_sha256) {
                return Ok(Some(Resolution::Fetch { 
                    url, 
                    sha256,
                    archive_bin: tool_config.archive_bin.clone(),
                }));
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
            path: Some("/usr/bin/node".to_string()),
            url: None,
            sha256: None,
            archive_bin: None,
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
            path: None,
            url: Some("https://go.dev/dl/go.tar.gz".to_string()),
            sha256: Some("abc12345".to_string()),
            archive_bin: None,
            template: None,
        });
        let config = Config { tools, telemetry: true };

        let resolved = resolve_tool("go", Some(&config), "", Path::new("/bin/scrim"));
        assert_eq!(resolved, Ok(Some(Resolution::Fetch { 
            url: "https://go.dev/dl/go.tar.gz".to_string(),
            sha256: "abc12345".to_string(),
            archive_bin: None,
        })));
    }

    #[test]
    fn test_resolve_template() {
        let mut tools = HashMap::new();
        tools.insert("go".to_string(), ToolConfig {
            path: None,
            url: Some("https://go.dev/dl/go.tar.gz".to_string()),
            sha256: Some("abc12345".to_string()),
            archive_bin: Some("go/bin/go".to_string()),
            template: None,
        });
        tools.insert("gofmt".to_string(), ToolConfig {
            path: None,
            url: None,
            sha256: None,
            archive_bin: Some("go/bin/gofmt".to_string()),
            template: Some("go".to_string()),
        });
        let config = Config { tools, telemetry: true };

        let resolved = resolve_tool("gofmt", Some(&config), "", Path::new("/bin/scrim"));
        assert_eq!(resolved, Ok(Some(Resolution::Fetch { 
            url: "https://go.dev/dl/go.tar.gz".to_string(),
            sha256: "abc12345".to_string(),
            archive_bin: Some("go/bin/gofmt".to_string()),
        })));
    }

    #[test]
    fn test_resolve_template_chain_disallowed() {
        let mut tools = HashMap::new();
        tools.insert("go".to_string(), ToolConfig {
            path: None,
            url: Some("https://go.dev/dl/go.tar.gz".to_string()),
            sha256: Some("abc12345".to_string()),
            archive_bin: Some("go/bin/go".to_string()),
            template: None,
        });
        tools.insert("gofmt".to_string(), ToolConfig {
            path: None,
            url: None,
            sha256: None,
            archive_bin: Some("go/bin/gofmt".to_string()),
            template: Some("go".to_string()),
        });
        tools.insert("go-chained".to_string(), ToolConfig {
            path: None,
            url: None,
            sha256: None,
            archive_bin: Some("go/bin/go-chained".to_string()),
            template: Some("gofmt".to_string()),
        });
        let config = Config { tools, telemetry: true };

        let resolved = resolve_tool("go-chained", Some(&config), "", Path::new("/bin/scrim"));
        assert_eq!(resolved, Err("Error: Tool 'go-chained' references template 'gofmt', which is also a template. Template chaining is not allowed.".to_string()));
    }
}

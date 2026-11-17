use std::collections::BTreeMap;
use std::path::PathBuf;
use config::{Config, ToolConfig};
use logger::scrim_warn;

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
    resolve_tool_for_platform_with_fallback(program_name, config, platform, false)
}

fn evaluate_with_fallback<T, F>(
    tool_name: &str,
    tool_config: &ToolConfig,
    allow_fallback: bool,
    mut evaluate: F,
) -> Result<Option<T>, String>
where
    F: FnMut(&ToolConfig) -> Result<Option<T>, String>,
{
    for layer in tool_config.layers() {
        match evaluate(layer) {
            Ok(res) => return Ok(res),
            Err(err) => {
                if allow_fallback {
                    scrim_warn!(
                        "Tool '{}' has an invalid configuration: {}. Falling back to lower-precedence configuration.",
                        tool_name,
                        err
                    );
                } else {
                    return Err(err);
                }
            }
        }
    }
    Ok(None)
}

pub fn resolve_tool_for_platform_with_fallback(
    program_name: &str,
    config: &Config,
    platform: &config::Platform,
    allow_fallback: bool,
) -> Result<Option<Resolution>, String> {
    let Some(tool_config) = config.tools.get(program_name) else {
        return Ok(None);
    };

    evaluate_with_fallback(program_name, tool_config, allow_fallback, |layer| {
        resolve_single_tool(program_name, layer, config, platform)
    })
}

pub fn resolve_effective_tool_for_platform(
    tool_name: &str,
    config: &Config,
    platform: &config::Platform,
    allow_fallback: bool,
) -> Result<Option<ToolConfig>, String> {
    let Some(tool_config) = config.tools.get(tool_name) else {
        return Ok(None);
    };

    evaluate_with_fallback(tool_name, tool_config, allow_fallback, |layer| {
        layer.effective_for_platform(tool_name, config, platform)
    })
}

#[derive(Debug, PartialEq)]
pub struct EffectiveConfigResolution {
    pub config: Config,
    pub errors: Vec<String>,
}

pub fn resolve_effective_config_for_platform(
    config: &Config,
    platform: &config::Platform,
    allow_fallback: bool,
) -> EffectiveConfigResolution {
    let mut effective_tools = BTreeMap::new();
    let mut errors = Vec::new();

    for (tool_name, tool_config) in &config.tools {
        match evaluate_with_fallback(tool_name, tool_config, allow_fallback, |layer| {
            layer.effective_for_platform(tool_name, config, platform)
        }) {
            Ok(Some(eff_tool)) => {
                effective_tools.insert(tool_name.clone(), eff_tool);
            }
            Ok(None) => {}
            Err(e) => {
                errors.push(e);
            }
        }
    }

    EffectiveConfigResolution {
        config: Config {
            default_platform: None,
            telemetry: config.telemetry,
            imports: Vec::new(),
            tools: effective_tools,
        },
        errors,
    }
}

fn resolve_single_tool(
    program_name: &str,
    tool_config: &ToolConfig,
    config: &Config,
    platform: &config::Platform,
) -> Result<Option<Resolution>, String> {
    tool_config.validate(program_name)?;

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
                template_config.validate(template_name)?;
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

    Ok(None)
}


pub use platform::{Arch, Os, Platform};

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
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlatformToolConfig {
    pub system_path: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>,
    pub archive_path: Option<String>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    pub template: Option<String>,
    pub archive_path: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub platforms: BTreeMap<String, PlatformToolConfig>,

    // Direct platform fields for desugaring (and backwards compatibility):
    #[serde(default, skip_serializing)]
    pub system_path: Option<String>,
    #[serde(default, skip_serializing)]
    pub url: Option<String>,
    #[serde(default, skip_serializing)]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing)]
    pub sha512: Option<String>,
}

impl ToolConfig {
    pub fn has_direct_platform_fields(&self) -> bool {
        self.system_path.is_some()
            || self.url.is_some()
            || self.sha256.is_some()
            || self.sha512.is_some()
    }

    pub fn get_platform(&self, target: &Platform) -> Option<&PlatformToolConfig> {
        for candidate in target.matching_candidates() {
            if let Some(config) = self.platforms.get(&candidate) {
                return Some(config);
            }
            if candidate == "any" {
                if let Some(config) = self.platforms.get("any-any") {
                    return Some(config);
                }
            }
        }
        None
    }

    pub fn effective_for_platform(
        &self,
        tool_name: &str,
        config: &Config,
        platform: &Platform,
    ) -> Result<Option<ToolConfig>, String> {
        // If this tool specifies a template, validate the template reference
        if let Some(template_name) = &self.template {
            if let Some(template_config) = config.tools.get(template_name) {
                if template_config.template.is_some() {
                    return Err(format!(
                        "Tool '{}' references template '{}', which is also a template. Template chaining is not allowed.",
                        tool_name, template_name
                    ));
                }
                // Check if the template tool has platforms and none match
                if !template_config.platforms.is_empty() && template_config.get_platform(platform).is_none() {
                    return Ok(None);
                }
            } else {
                return Err(format!(
                    "Tool '{}' references template '{}', which was not found.",
                    tool_name, template_name
                ));
            }
        }

        if !self.platforms.is_empty() {
            let mut matched = None;
            for candidate in platform.matching_candidates() {
                if let Some(plat_cfg) = self.platforms.get(&candidate) {
                    matched = Some((candidate, plat_cfg.clone()));
                    break;
                }
                if candidate == "any" {
                    if let Some(plat_cfg) = self.platforms.get("any-any") {
                        matched = Some(("any-any".to_string(), plat_cfg.clone()));
                        break;
                    }
                }
            }

            match matched {
                Some((plat_key, plat_cfg)) => {
                    let mut platforms = BTreeMap::new();
                    platforms.insert(plat_key, plat_cfg);
                    Ok(Some(ToolConfig {
                        template: self.template.clone(),
                        archive_path: self.archive_path.clone(),
                        platforms,
                        system_path: None,
                        url: None,
                        sha256: None,
                        sha512: None,
                    }))
                }
                None => Ok(None),
            }
        } else if self.template.is_some() {
            Ok(Some(ToolConfig {
                template: self.template.clone(),
                archive_path: self.archive_path.clone(),
                platforms: BTreeMap::new(),
                system_path: None,
                url: None,
                sha256: None,
                sha512: None,
            }))
        } else if self.has_direct_platform_fields() {
            let mut platforms = BTreeMap::new();
            platforms.insert(
                "any".to_string(),
                PlatformToolConfig {
                    system_path: self.system_path.clone(),
                    url: self.url.clone(),
                    sha256: self.sha256.clone(),
                    sha512: self.sha512.clone(),
                    archive_path: None,
                },
            );
            Ok(Some(ToolConfig {
                template: None,
                archive_path: self.archive_path.clone(),
                platforms,
                system_path: None,
                url: None,
                sha256: None,
                sha512: None,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn validate(&self, tool_name: &str) -> Result<(), String> {
        // 1. Empty tool configuration
        if self.template.is_none() && self.platforms.is_empty() {
            return Err(format!(
                "Tool '{}' has an empty configuration. Every tool must specify a 'system_path', 'url', or 'template'.",
                tool_name
            ));
        }

        // 2. Mutually exclusive template with url and digests
        if self.template.is_some() {
            for plat in self.platforms.values() {
                if plat.url.is_some() || plat.sha256.is_some() || plat.sha512.is_some() {
                    return Err(format!(
                        "Tool '{}' specifies 'template' which is mutually exclusive with 'url' and digests.",
                        tool_name
                    ));
                }
            }
        }

        // 3. Validate platforms
        for (platform_str, plat_config) in &self.platforms {
            // Validate platform string format
            Platform::parse(platform_str)?;

            // Digest mutual exclusivity
            if plat_config.sha256.is_some() && plat_config.sha512.is_some() {
                return Err(format!(
                    "Tool '{}' specifies multiple digests. Exactly one digest algorithm must be specified.",
                    tool_name
                ));
            }

            // url and digest pairing
            let has_url = plat_config.url.is_some();
            let has_digest = plat_config.sha256.is_some() || plat_config.sha512.is_some();
            if has_url != has_digest {
                if has_url {
                    return Err(format!(
                        "Tool '{}' specifies 'url' without a digest. Both must be provided together.",
                        tool_name
                    ));
                } else {
                    let digest_name = if plat_config.sha256.is_some() {
                        "sha256"
                    } else {
                        "sha512"
                    };
                    return Err(format!(
                        "Tool '{}' specifies '{}' without 'url'. Both must be provided together.",
                        tool_name, digest_name
                    ));
                }
            }

            // Absolute system_path
            if let Some(path) = &plat_config.system_path {
                if !Path::new(path).is_absolute() {
                    return Err(format!(
                        "Tool '{}' specifies relative system_path '{}'. 'system_path' must be an absolute path.",
                        tool_name, path
                    ));
                }
            }

            // Platform-level archive_path requires url or template
            if plat_config.archive_path.is_some() && !has_url && self.template.is_none() {
                return Err(format!(
                    "Tool '{}' specifies 'archive_path' but provides no 'url' or 'template'.",
                    tool_name
                ));
            }

            // Empty platform definition
            let is_templated_with_archive_path = self.template.is_some() && plat_config.archive_path.is_some();
            if plat_config.system_path.is_none() && !has_url && !is_templated_with_archive_path {
                return Err(format!(
                    "Tool '{}' has an empty configuration for platform '{}'. Every platform must specify a 'system_path' or 'url' (or 'archive_path' for templated tools).",
                    tool_name, platform_str
                ));
            }
        }

        // 4. Tool-level archive_path requires url or template
        if self.archive_path.is_some() && self.template.is_none() {
            let has_url = self.platforms.values().any(|p| p.url.is_some());
            if !has_url {
                return Err(format!(
                    "Tool '{}' specifies 'archive_path' but provides no 'url' or 'template'.",
                    tool_name
                ));
            }
        }

        Ok(())
    }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing)]
    pub default_platform: Option<String>,
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
        self.default_platform = None;
        self
    }

    pub fn effective_for_platform(&self, platform: &Platform) -> Result<Config, String> {
        let mut effective_tools = BTreeMap::new();
        for (tool_name, tool_config) in &self.tools {
            if let Some(eff_tool) = tool_config.effective_for_platform(tool_name, self, platform)? {
                effective_tools.insert(tool_name.clone(), eff_tool);
            }
        }
        Ok(Config {
            default_platform: None,
            telemetry: self.telemetry,
            imports: Vec::new(),
            tools: effective_tools,
        })
    }

    pub fn format_yaml_for_platform(
        &self,
        tool: Option<&str>,
        platform: &Platform,
    ) -> Result<String, String> {
        if let Some(tool_name) = tool {
            match self.tools.get(tool_name) {
                Some(tool_config) => {
                    let effective = tool_config.effective_for_platform(tool_name, self, platform)?;
                    match effective {
                        Some(eff) => serde_yaml::to_string(&eff)
                            .map_err(|e| format!("Failed to serialize configuration: {}", e)),
                        None => Err(format!(
                            "Tool '{}' is not configured for platform '{}'",
                            tool_name, platform
                        )),
                    }
                }
                None => Err(format!("Tool '{}' not found in config", tool_name)),
            }
        } else {
            let effective_config = self.effective_for_platform(platform)?;
            serde_yaml::to_string(&effective_config)
                .map_err(|e| format!("Failed to serialize configuration: {}", e))
        }
    }

    pub fn format_yaml(&self, tool: Option<&str>) -> Result<String, String> {
        let host = Platform::host()?;
        self.format_yaml_for_platform(tool, &host)
    }

    pub fn validate(&self) -> Result<(), String> {
        for (tool_name, tool_config) in &self.tools {
            tool_config.validate(tool_name)?;
        }
        Ok(())
    }

    pub fn parse_str(yaml_str: &str) -> Result<Config, Box<dyn std::error::Error>> {
        let mut parsed: Config = serde_yaml::from_str(yaml_str)
            .map_err(|e| format!("Failed to parse configuration: {}", e))?;

        desugar_and_validate(&mut parsed, None)?;
        Ok(parsed)
    }
}

fn desugar_and_validate(
    config: &mut Config,
    file_context: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let default_platform = config.default_platform.take();
    if let Some(ref dp) = default_platform {
        Platform::parse(dp).map_err(|e| {
            if let Some(path) = file_context {
                format!("Invalid default_platform in '{}': {}", path.display(), e)
            } else {
                format!("Invalid default_platform: {}", e)
            }
        })?;
    }

    for (tool_name, tool_config) in &mut config.tools {
        let has_direct = tool_config.has_direct_platform_fields();
        if has_direct && !tool_config.platforms.is_empty() {
            let msg = if let Some(path) = file_context {
                format!(
                    "Tool '{}' in '{}' mixes direct platform properties with an explicit 'platforms' block. A tool must define platform properties either directly via 'default_platform' or inside 'platforms', but not both.",
                    tool_name,
                    path.display()
                )
            } else {
                format!(
                    "Tool '{}' mixes direct platform properties with an explicit 'platforms' block. A tool must define platform properties either directly via 'default_platform' or inside 'platforms', but not both.",
                    tool_name
                )
            };
            return Err(msg.into());
        }

        if has_direct {
            if let Some(ref dp) = default_platform {
                let plat_cfg = PlatformToolConfig {
                    system_path: tool_config.system_path.take(),
                    url: tool_config.url.take(),
                    sha256: tool_config.sha256.take(),
                    sha512: tool_config.sha512.take(),
                    archive_path: None,
                };
                tool_config.platforms.insert(dp.clone(), plat_cfg);
            } else {
                let msg = if let Some(path) = file_context {
                    format!(
                        "Tool '{}' in '{}' specifies platform-specific properties directly, but no 'default_platform' is declared for this file. Either declare a top-level 'default_platform: <os>-<arch>' or place these properties under a 'platforms' map.",
                        tool_name,
                        path.display()
                    )
                } else {
                    format!(
                        "Tool '{}' specifies platform-specific properties directly, but no 'default_platform' is declared for this file. Either declare a top-level 'default_platform: <os>-<arch>' or place these properties under a 'platforms' map.",
                        tool_name
                    )
                };
                return Err(msg.into());
            }
        }
    }

    config
        .validate()
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    Ok(())
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
        format!(
            "Failed to resolve configuration path '{}': {}",
            path.display(),
            e
        )
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
        format!(
            "Failed to read configuration file '{}': {}",
            path.display(),
            e
        )
    })?;

    let mut parsed: Config = serde_yaml::from_str(&content).map_err(|e| {
        format!(
            "Failed to parse configuration file '{}': {}",
            path.display(),
            e
        )
    })?;

    desugar_and_validate(&mut parsed, Some(path))?;

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

    let current_file_clean = Config {
        default_platform: None,
        telemetry: parsed.telemetry,
        imports: Vec::new(),
        tools: parsed.tools,
    };

    let result = before_config.merge(current_file_clean).merge(after_config);
    Ok(result)
}

fn resolve_xdg_base_dir(
    xdg_env: Option<&Path>,
    home_dir: Option<&Path>,
    default_subdir: &str,
) -> PathBuf {
    if let Some(xdg_path) = xdg_env {
        if xdg_path.is_absolute() {
            return xdg_path.to_path_buf();
        }
    }

    if let Some(home) = home_dir {
        home.join(default_subdir)
    } else {
        PathBuf::from(default_subdir)
    }
}

pub fn resolve_config_dir(xdg_config_env: Option<&Path>, home_dir: Option<&Path>) -> PathBuf {
    resolve_xdg_base_dir(xdg_config_env, home_dir, ".config")
}

pub fn resolve_cache_dir(
    xdg_cache_env: Option<&Path>,
    home_dir: Option<&Path>,
) -> PathBuf {
    resolve_xdg_base_dir(xdg_cache_env, home_dir, ".cache").join("scrim")
}

#[cfg(test)]
mod tests {
    use super::*;
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

        // Child wins for telemetry
        assert_eq!(config.telemetry, Some(true));

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
        assert_eq!(config.telemetry, Some(true));
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

        let err = read_config(&main_import_invalid).unwrap_err();
        assert!(err.to_string().contains("no 'default_platform' is declared for this file"));

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
            telemetry: Some(true),
            imports: Vec::new(),
            tools,
        };

        // Full configuration
        let full_yaml = config.format_yaml(None).unwrap();
        assert!(full_yaml.contains("telemetry: true"));
        assert!(full_yaml.contains("node:"));
        assert!(full_yaml.contains("go:"));
        assert!(full_yaml.contains("platforms:"));

        // Specific tool configuration
        let node_yaml = config.format_yaml(Some("node")).unwrap();
        assert!(node_yaml.contains("linux-x86_64:"));
        assert!(node_yaml.contains("system_path: /usr/local/bin/node"));

        // Unknown tool error
        let err = config.format_yaml(Some("nonexistent")).unwrap_err();
        assert_eq!(err, "Tool 'nonexistent' not found in config");
    }

    #[test]
    fn test_format_yaml_for_platform() {
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
        let linux_yaml = config.format_yaml_for_platform(None, &linux_platform).unwrap();
        assert!(linux_yaml.contains("telemetry: false"));
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
        let go_yaml = config.format_yaml_for_platform(Some("go"), &linux_platform).unwrap();
        assert!(go_yaml.contains("linux-x86_64:"));
        assert!(go_yaml.contains("linux_hash"));
        assert!(!go_yaml.contains("macos-aarch64:"));

        // 3. Tool not configured for platform error
        let err = config.format_yaml_for_platform(Some("macos_only"), &linux_platform).unwrap_err();
        assert_eq!(err, "Tool 'macos_only' is not configured for platform 'linux-x86_64'");

        // 4. Unknown tool error
        let err2 = config.format_yaml_for_platform(Some("unknown"), &linux_platform).unwrap_err();
        assert_eq!(err2, "Tool 'unknown' not found in config");

        // 5. Unconfigured platform (linux-aarch64) has empty tools list
        let empty_yaml = config.format_yaml_for_platform(None, &arm_linux).unwrap();
        assert!(!empty_yaml.contains("node:"));
        assert!(!empty_yaml.contains("go:"));
        assert!(!empty_yaml.contains("gofmt:"));
        assert!(!empty_yaml.contains("macos_only:"));

        // 6. macOS platform full config
        let mac_yaml = config.format_yaml_for_platform(None, &macos_platform).unwrap();
        assert!(mac_yaml.contains("macos_only:"));
        assert!(mac_yaml.contains("/opt/homebrew/bin/mac_tool"));
        assert!(mac_yaml.contains("mac_hash"));
        assert!(!mac_yaml.contains("linux_hash"));
    }
}

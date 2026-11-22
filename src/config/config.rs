pub use platform::{Arch, Os, Platform};

use logger::scrim_warn;
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

    #[serde(skip)]
    pub fallback: Option<Box<ToolConfig>>,
    #[serde(skip)]
    pub desugar_error: Option<String>,
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
        self.validate(tool_name)?;

        // If this tool specifies a template, validate the template reference
        if let Some(template_name) = &self.template {
            if let Some(template_config) = config.tools.get(template_name) {
                template_config.validate(template_name)?;
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
                        fallback: None,
                        desugar_error: None,
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
                fallback: None,
                desugar_error: None,
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
                fallback: None,
                desugar_error: None,
            }))
        } else {
            Ok(None)
        }
    }

    /// Iterates through this configuration layer and all lower-precedence fallback layers.
    pub fn layers(&self) -> impl Iterator<Item = &ToolConfig> {
        std::iter::successors(Some(self), |tc| tc.fallback.as_deref())
    }

    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml::to_string(self).map_err(|e| format!("Failed to serialize configuration: {}", e))
    }

    pub fn validate(&self, tool_name: &str) -> Result<(), String> {
        if let Some(ref err) = self.desugar_error {
            return Err(err.clone());
        }

        // Tool-level archive_path requires url or template
        if self.archive_path.is_some() && self.template.is_none() {
            let has_url = self.platforms.values().any(|p| p.url.is_some()) || self.url.is_some();
            if !has_url {
                return Err(format!(
                    "Tool '{}' specifies 'archive_path' but provides no 'url' or 'template'.",
                    tool_name
                ));
            }
        }

        // 1. Empty tool configuration
        if self.template.is_none() && self.platforms.is_empty() && !self.has_direct_platform_fields() {
            return Err(format!(
                "Tool '{}' has an empty configuration. Every tool must specify a 'system_path', 'url', or 'template'.",
                tool_name
            ));
        }

        // 2. Mutually exclusive template with url and digests
        if self.template.is_some() {
            if self.url.is_some() || self.sha256.is_some() || self.sha512.is_some() {
                return Err(format!(
                    "Tool '{}' specifies 'template' which is mutually exclusive with 'url' and digests.",
                    tool_name
                ));
            }
            for plat in self.platforms.values() {
                if plat.url.is_some() || plat.sha256.is_some() || plat.sha512.is_some() {
                    return Err(format!(
                        "Tool '{}' specifies 'template' which is mutually exclusive with 'url' and digests.",
                        tool_name
                    ));
                }
            }
        }

        // Direct platform fields validation (for programmatic tool configs)
        if self.has_direct_platform_fields() {
            if self.sha256.is_some() && self.sha512.is_some() {
                return Err(format!(
                    "Tool '{}' specifies multiple digests. Exactly one digest algorithm must be specified.",
                    tool_name
                ));
            }

            let has_url = self.url.is_some();
            let has_digest = self.sha256.is_some() || self.sha512.is_some();
            if has_url != has_digest {
                if has_url {
                    return Err(format!(
                        "Tool '{}' specifies 'url' without a digest. Both must be provided together.",
                        tool_name
                    ));
                } else {
                    let digest_name = if self.sha256.is_some() {
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

            if let Some(path) = &self.system_path {
                if !Path::new(path).is_absolute() {
                    return Err(format!(
                        "Tool '{}' specifies relative system_path '{}'. 'system_path' must be an absolute path.",
                        tool_name, path
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

        Ok(())
    }
}

struct StrictString(String);

impl<'de> Deserialize<'de> for StrictString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StrictStringVisitor;

        impl<'de> serde::de::Visitor<'de> for StrictStringVisitor {
            type Value = StrictString;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictString(v.to_string()))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictString(v))
            }
        }

        deserializer.deserialize_any(StrictStringVisitor)
    }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileTelemetryConfig {
    pub path: Option<String>,
}

impl<'de> Deserialize<'de> for FileTelemetryConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FileTelemetryVisitor;

        impl<'de> serde::de::Visitor<'de> for FileTelemetryVisitor {
            type Value = FileTelemetryConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a file telemetry object or path string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(FileTelemetryConfig {
                    path: Some(v.to_string()),
                })
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(FileTelemetryConfig { path: Some(v) })
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut path: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "path" => {
                            if path.is_some() {
                                return Err(serde::de::Error::duplicate_field("path"));
                            }
                            path = Some(map.next_value::<StrictString>()?.0);
                        }
                        other => {
                            return Err(serde::de::Error::unknown_field(other, &["path"]));
                        }
                    }
                }
                Ok(FileTelemetryConfig { path })
            }
        }

        deserializer.deserialize_any(FileTelemetryVisitor)
    }
}

fn deserialize_file_sinks<'de, D>(deserializer: D) -> Result<Vec<FileTelemetryConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct FileSinksVisitor;

    impl<'de> serde::de::Visitor<'de> for FileSinksVisitor {
        type Value = Vec<FileTelemetryConfig>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a file telemetry sink or sequence of file telemetry sinks")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![FileTelemetryConfig {
                path: Some(v.to_string()),
            }])
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![FileTelemetryConfig { path: Some(v) }])
        }

        fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let sink = FileTelemetryConfig::deserialize(
                serde::de::value::MapAccessDeserializer::new(map),
            )?;
            Ok(vec![sink])
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut sinks = Vec::new();
            while let Some(item) = seq.next_element::<FileTelemetryConfig>()? {
                sinks.push(item);
            }
            Ok(sinks)
        }
    }

    deserializer.deserialize_any(FileSinksVisitor)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TelemetryConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file: Vec<FileTelemetryConfig>,
}

impl TelemetryConfig {
    pub fn is_empty(&self) -> bool {
        self.file.is_empty()
    }

    /// Merges telemetry sinks additively while preserving order and deduplicating identical entries.
    pub fn merge(&mut self, other: TelemetryConfig) {
        for sink in other.file {
            if !self.file.contains(&sink) {
                self.file.push(sink);
            }
        }
    }

    /// Resolves configured file sinks to unique output paths, defaulting omitted paths to the XDG state log.
    pub fn resolve_file_paths(
        &self,
        xdg_state_env: Option<&Path>,
        home_dir: Option<&Path>,
    ) -> Vec<PathBuf> {
        if self.file.is_empty() {
            return Vec::new();
        }
        let mut default_path: Option<PathBuf> = None;
        let mut paths = Vec::with_capacity(self.file.len());
        for sink in &self.file {
            let resolved = match &sink.path {
                Some(p) => PathBuf::from(p),
                None => default_path
                    .get_or_insert_with(|| {
                        resolve_default_telemetry_log_path(xdg_state_env, home_dir)
                    })
                    .clone(),
            };
            if !paths.contains(&resolved) {
                paths.push(resolved);
            }
        }
        paths
    }
}

impl<'de> Deserialize<'de> for TelemetryConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FileSinks(Vec<FileTelemetryConfig>);

        impl<'de> Deserialize<'de> for FileSinks {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserialize_file_sinks(deserializer).map(FileSinks)
            }
        }

        struct TelemetryConfigVisitor;

        impl<'de> serde::de::Visitor<'de> for TelemetryConfigVisitor {
            type Value = TelemetryConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a telemetry configuration mapping or boolean")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                // Legacy boolean shorthand (deprecated): true maps to default XDG file sink, false is a no-op.
                if v {
                    Ok(TelemetryConfig {
                        file: vec![FileTelemetryConfig { path: None }],
                    })
                } else {
                    Ok(TelemetryConfig::default())
                }
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut file: Option<Vec<FileTelemetryConfig>> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "file" => {
                            if file.is_some() {
                                return Err(serde::de::Error::duplicate_field("file"));
                            }
                            file = Some(map.next_value::<FileSinks>()?.0);
                        }
                        other => {
                            return Err(serde::de::Error::unknown_field(other, &["file"]));
                        }
                    }
                }
                Ok(TelemetryConfig {
                    file: file.unwrap_or_default(),
                })
            }
        }

        deserializer.deserialize_any(TelemetryConfigVisitor)
    }
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing)]
    pub default_platform: Option<String>,
    #[serde(default, skip_serializing_if = "TelemetryConfig::is_empty")]
    pub telemetry: TelemetryConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<ImportEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, ToolConfig>,
}

impl Config {
    pub fn telemetry_enabled(&self) -> bool {
        !self.telemetry.is_empty()
    }

    pub fn merge(mut self, other: Config) -> Self {
        self.telemetry.merge(other.telemetry);
        for (tool_name, mut other_tool) in other.tools {
            if let Some(prev_tool) = self.tools.remove(&tool_name) {
                let mut curr = &mut other_tool;
                while let Some(ref mut next) = curr.fallback {
                    curr = next;
                }
                curr.fallback = Some(Box::new(prev_tool));
            }
            self.tools.insert(tool_name, other_tool);
        }
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
            telemetry: self.telemetry.clone(),
            imports: Vec::new(),
            tools: effective_tools,
        })
    }

    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml::to_string(self).map_err(|e| format!("Failed to serialize configuration: {}", e))
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

        desugar_file_tools(&mut parsed, None)?;
        parsed
            .validate()
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        Ok(parsed)
    }
}

fn sanitize_telemetry(telemetry: &mut TelemetryConfig, file_context: Option<&Path>) {
    let mut sanitized_file_sinks = Vec::with_capacity(telemetry.file.len());
    for sink in std::mem::take(&mut telemetry.file) {
        if let Some(ref p) = sink.path {
            if p.is_empty() || !Path::new(p).is_absolute() {
                let msg = if let Some(path) = file_context {
                    format!(
                        "Invalid telemetry file path '{}' in '{}': 'path' must be a non-empty absolute path",
                        p,
                        path.display()
                    )
                } else {
                    format!(
                        "Invalid telemetry file path '{}': 'path' must be a non-empty absolute path",
                        p
                    )
                };
                scrim_warn!("{}. Ignoring telemetry sink.", msg);
                continue;
            }
        }
        if !sanitized_file_sinks.contains(&sink) {
            sanitized_file_sinks.push(sink);
        }
    }
    telemetry.file = sanitized_file_sinks;
}

pub fn desugar_file_tools(
    config: &mut Config,
    file_context: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    sanitize_telemetry(&mut config.telemetry, file_context);

    let default_platform = config.default_platform.take();
    let valid_dp = if let Some(ref dp) = default_platform {
        match Platform::parse(dp) {
            Ok(_) => Some(dp.clone()),
            Err(e) => {
                let msg = if let Some(path) = file_context {
                    format!("Invalid default_platform in '{}': {}", path.display(), e)
                } else {
                    format!("Invalid default_platform: {}", e)
                };
                scrim_warn!("{}. Ignoring default_platform.", msg);
                None
            }
        }
    } else {
        None
    };

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
            tool_config.desugar_error = Some(msg);
        } else if has_direct {
            if let Some(ref dp) = valid_dp {
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
                tool_config.desugar_error = Some(msg);
            }
        }
    }

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

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            scrim_warn!(
                "Failed to read configuration file '{}': {}. Ignoring file.",
                path.display(),
                e
            );
            import_stack.pop();
            return Ok(Config::default());
        }
    };

    let mut parsed: Config = match serde_yaml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            scrim_warn!(
                "Failed to parse configuration file '{}': {}. Ignoring file.",
                path.display(),
                e
            );
            import_stack.pop();
            return Ok(Config::default());
        }
    };

    desugar_file_tools(&mut parsed, Some(path))?;

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

pub fn resolve_state_dir(
    xdg_state_env: Option<&Path>,
    home_dir: Option<&Path>,
) -> PathBuf {
    resolve_xdg_base_dir(xdg_state_env, home_dir, ".local/state").join("scrim")
}

pub fn resolve_default_telemetry_log_path(
    xdg_state_env: Option<&Path>,
    home_dir: Option<&Path>,
) -> PathBuf {
    resolve_state_dir(xdg_state_env, home_dir).join("telemetry.log")
}


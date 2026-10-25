use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Os {
    Linux,
    Macos,
    Any,
}

impl Os {
    pub fn as_str(&self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Any => "any",
        }
    }
}

impl std::fmt::Display for Os {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
    Any,
}

impl Arch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
            Arch::Any => "any",
        }
    }
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.os == Os::Any && self.arch == Arch::Any {
            write!(f, "any")
        } else {
            write!(f, "{}-{}", self.os.as_str(), self.arch.as_str())
        }
    }
}

impl Serialize for Platform {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Platform {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Platform::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl std::str::FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Platform::parse(s)
    }
}

enum OsParseResult {
    Ok(Os),
    Synonym { synonym: &'static str, canonical: &'static str },
    Unknown(String),
}

fn parse_os_part(s: &str) -> OsParseResult {
    match s {
        "linux" => OsParseResult::Ok(Os::Linux),
        "macos" => OsParseResult::Ok(Os::Macos),
        "any" => OsParseResult::Ok(Os::Any),
        "darwin" => OsParseResult::Synonym { synonym: "darwin", canonical: "macos" },
        "osx" => OsParseResult::Synonym { synonym: "osx", canonical: "macos" },
        "mac" => OsParseResult::Synonym { synonym: "mac", canonical: "macos" },
        other => OsParseResult::Unknown(other.to_string()),
    }
}

enum ArchParseResult {
    Ok(Arch),
    Synonym { synonym: &'static str, canonical: &'static str },
    Unknown(String),
}

fn parse_arch_part(s: &str) -> ArchParseResult {
    match s {
        "x86_64" => ArchParseResult::Ok(Arch::X86_64),
        "aarch64" => ArchParseResult::Ok(Arch::Aarch64),
        "any" => ArchParseResult::Ok(Arch::Any),
        "arm64" => ArchParseResult::Synonym { synonym: "arm64", canonical: "aarch64" },
        "amd64" => ArchParseResult::Synonym { synonym: "amd64", canonical: "x86_64" },
        "x86-64" => ArchParseResult::Synonym { synonym: "x86-64", canonical: "x86_64" },
        "x64" => ArchParseResult::Synonym { synonym: "x64", canonical: "x86_64" },
        "aarch-64" => ArchParseResult::Synonym { synonym: "aarch-64", canonical: "aarch64" },
        "aarch_64" => ArchParseResult::Synonym { synonym: "aarch_64", canonical: "aarch64" },
        other => ArchParseResult::Unknown(other.to_string()),
    }
}

impl Platform {
    pub fn new(os: Os, arch: Arch) -> Self {
        Self { os, arch }
    }

    pub fn host() -> Result<Self, String> {
        let os = match std::env::consts::OS {
            "linux" => Os::Linux,
            "macos" => Os::Macos,
            other => {
                return Err(format!(
                    "Unsupported host operating system '{}'. Supported: 'linux', 'macos'.",
                    other
                ))
            }
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            other => {
                return Err(format!(
                    "Unsupported host architecture '{}'. Supported: 'x86_64', 'aarch64'.",
                    other
                ))
            }
        };
        Ok(Platform { os, arch })
    }

    pub fn matches(&self, target: &Platform) -> bool {
        (self.os == Os::Any || target.os == Os::Any || self.os == target.os)
            && (self.arch == Arch::Any || target.arch == Arch::Any || self.arch == target.arch)
    }

    pub fn matching_candidates(&self) -> Vec<String> {
        let mut candidates = Vec::with_capacity(5);
        if self.os != Os::Any && self.arch != Arch::Any {
            candidates.push(format!("{}-{}", self.os.as_str(), self.arch.as_str()));
            candidates.push(format!("{}-any", self.os.as_str()));
            candidates.push(format!("any-{}", self.arch.as_str()));
            candidates.push("any".to_string());
            candidates.push("any-any".to_string());
        } else if self.os != Os::Any && self.arch == Arch::Any {
            candidates.push(format!("{}-any", self.os.as_str()));
            candidates.push("any".to_string());
            candidates.push("any-any".to_string());
        } else if self.os == Os::Any && self.arch != Arch::Any {
            candidates.push(format!("any-{}", self.arch.as_str()));
            candidates.push("any".to_string());
            candidates.push("any-any".to_string());
        } else {
            candidates.push("any".to_string());
            candidates.push("any-any".to_string());
        }
        candidates
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let trimmed = s.trim();

        if trimmed == "any" || trimmed == "any-any" {
            return Ok(Platform {
                os: Os::Any,
                arch: Arch::Any,
            });
        }

        // Check for underscore delimiter instead of hyphen delimiter
        // Note: x86_64 contains an underscore, so check if delimiter between os and arch is '_'
        // e.g. linux_x86_64, macos_aarch64, darwin_arm64, etc.
        if !trimmed.contains('-') && trimmed.contains('_') {
            // Check if it starts with an OS or OS synonym followed by '_'
            let known_os_prefixes = ["linux_", "macos_", "darwin_", "osx_", "mac_", "any_"];
            if let Some(prefix) = known_os_prefixes.iter().find(|p| trimmed.starts_with(*p)) {
                let os_part = &prefix[..prefix.len() - 1];
                let arch_part = &trimmed[prefix.len()..];
                let canonical_os = match os_part {
                    "darwin" | "osx" | "mac" => "macos",
                    other => other,
                };
                let canonical_arch = match arch_part {
                    "arm64" | "aarch-64" | "aarch_64" => "aarch64",
                    "amd64" | "x86-64" | "x64" => "x86_64",
                    other => other,
                };
                return Err(format!(
                    "Invalid platform string '{}': platform components must be separated by '-', not '_'. Did you mean '{}-{}'? Valid format is '<os>-<arch>' (valid OS: 'linux', 'macos', 'any'; valid arch: 'x86_64', 'aarch64', 'any').",
                    trimmed, canonical_os, canonical_arch
                ));
            }
        }

        // Handle splitting into os and arch
        // Special case: "x86-64" contains a hyphen within arch
        let (os_part, arch_part) = if trimmed.ends_with("-x86-64") {
            let split_idx = trimmed.len() - "-x86-64".len();
            (&trimmed[..split_idx], "x86-64")
        } else if let Some((os_p, arch_p)) = trimmed.split_once('-') {
            if arch_p.contains('-') {
                return Err(format!(
                    "Invalid platform string '{}': platform format is '<os>-<arch>' or 'any'. Valid OS: 'linux', 'macos', 'any'. Valid arch: 'x86_64', 'aarch64', 'any'.",
                    trimmed
                ));
            }
            (os_p, arch_p)
        } else {
            // Single token without hyphen
            let os_check = parse_os_part(trimmed);
            let arch_check = parse_arch_part(trimmed);
            match (os_check, arch_check) {
                (OsParseResult::Ok(_), _) => {
                    return Err(format!(
                        "Invalid platform string '{}': missing architecture component. Platform format is '<os>-<arch>' or 'any'. Valid OS: 'linux', 'macos', 'any'. Valid arch: 'x86_64', 'aarch64', 'any'.",
                        trimmed
                    ));
                }
                (OsParseResult::Synonym { canonical, .. }, _) => {
                    return Err(format!(
                        "Invalid platform string '{}': missing architecture component and '{}' is an invalid OS (did you mean '{}'?). Platform format is '<os>-<arch>' or 'any'. Valid OS: 'linux', 'macos', 'any'. Valid arch: 'x86_64', 'aarch64', 'any'.",
                        trimmed, trimmed, canonical
                    ));
                }
                (_, ArchParseResult::Ok(_)) => {
                    return Err(format!(
                        "Invalid platform string '{}': missing OS component. Platform format is '<os>-<arch>' or 'any'. Valid OS: 'linux', 'macos', 'any'. Valid arch: 'x86_64', 'aarch64', 'any'.",
                        trimmed
                    ));
                }
                (_, ArchParseResult::Synonym { canonical, .. }) => {
                    return Err(format!(
                        "Invalid platform string '{}': missing OS component and '{}' is an invalid architecture (did you mean '{}'?). Platform format is '<os>-<arch>' or 'any'. Valid OS: 'linux', 'macos', 'any'. Valid arch: 'x86_64', 'aarch64', 'any'.",
                        trimmed, trimmed, canonical
                    ));
                }
                _ => {
                    return Err(format!(
                        "Invalid platform string '{}': platform format is '<os>-<arch>' or 'any'. Valid OS: 'linux', 'macos', 'any'. Valid arch: 'x86_64', 'aarch64', 'any'.",
                        trimmed
                    ));
                }
            }
        };

        let os_res = parse_os_part(os_part);
        let arch_res = parse_arch_part(arch_part);

        match (os_res, arch_res) {
            (OsParseResult::Ok(os), ArchParseResult::Ok(arch)) => Ok(Platform { os, arch }),
            (os_err, arch_err) => {
                let mut issues = Vec::new();
                let mut suggested_os = None;
                let mut suggested_arch = None;

                match os_err {
                    OsParseResult::Ok(os) => {
                        suggested_os = Some(os.as_str());
                    }
                    OsParseResult::Synonym { synonym, canonical } => {
                        issues.push(format!("'{}' is an invalid OS (did you mean '{}'?)", synonym, canonical));
                        suggested_os = Some(canonical);
                    }
                    OsParseResult::Unknown(other) => {
                        issues.push(format!("'{}' is an invalid OS", other));
                    }
                }

                match arch_err {
                    ArchParseResult::Ok(arch) => {
                        suggested_arch = Some(arch.as_str());
                    }
                    ArchParseResult::Synonym { synonym, canonical } => {
                        issues.push(format!("'{}' is an invalid architecture (did you mean '{}'?)", synonym, canonical));
                        suggested_arch = Some(canonical);
                    }
                    ArchParseResult::Unknown(other) => {
                        issues.push(format!("'{}' is an invalid architecture", other));
                    }
                }

                let issues_str = issues.join(" and ");
                let suggestion_str = match (suggested_os, suggested_arch) {
                    (Some(o), Some(a)) => format!(" Did you mean '{}-{}'?", o, a),
                    _ => String::new(),
                };

                Err(format!(
                    "Invalid platform string '{}': {}.{} Valid OS: 'linux', 'macos', 'any'. Valid arch: 'x86_64', 'aarch64', 'any'.",
                    trimmed, issues_str, suggestion_str
                ))
            }
        }
    }
}

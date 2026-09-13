pub const VERSION: &str = "0.3.2";

pub fn format_version(commit: Option<&str>) -> String {
    match commit.map(str::trim).filter(|h| !h.is_empty() && !h.starts_with('{')) {
        Some(hash) => format!("scrim {} ({})", VERSION, hash),
        None => format!("scrim {}", VERSION),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_version() {
        assert_eq!(format_version(None), format!("scrim {}", VERSION));
        assert_eq!(format_version(Some("")), format!("scrim {}", VERSION));
        assert_eq!(format_version(Some("   ")), format!("scrim {}", VERSION));
        assert_eq!(format_version(Some("{STABLE_GIT_COMMIT}")), format!("scrim {}", VERSION));
        assert_eq!(
            format_version(Some("e259af2039b3f2ade3bb24dcd413467e229418f2")),
            format!("scrim {} (e259af2039b3f2ade3bb24dcd413467e229418f2)", VERSION)
        );
        assert_eq!(
            format_version(Some("  e259af2039b3f2ade3bb24dcd413467e229418f2  ")),
            format!("scrim {} (e259af2039b3f2ade3bb24dcd413467e229418f2)", VERSION)
        );
    }
}

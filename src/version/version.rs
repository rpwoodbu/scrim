pub const VERSION: &str = "0.14.0";

pub fn format_version(commit: Option<&str>) -> String {
    match commit.map(str::trim).filter(|h| !h.is_empty() && !h.starts_with('{')) {
        Some(hash) => format!("scrim {} ({})", VERSION, hash),
        None => format!("scrim {}", VERSION),
    }
}

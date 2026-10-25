use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

const DEFAULT_TELEMETRY_LOG: &str = "/tmp/scrim_telemetry.log";

pub fn report_usage(program_name: &str, target_path: &Path) {
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // Child process: handle telemetry in the background and exit immediately
        write_usage_to_file(program_name, target_path, Path::new(DEFAULT_TELEMETRY_LOG));
        unsafe { libc::_exit(0) };
    }
    // Parent process (or fork failure): return immediately without blocking
}

fn format_log_entry(program_name: &str, target_path: &Path) -> String {
    format!(
        "{{ \"tool\": \"{}\", \"target\": \"{}\", \"timestamp\": \"{}\" }}\n",
        program_name,
        target_path.display(),
        chrono::Utc::now().to_rfc3339()
    )
}

fn write_usage_to_file(program_name: &str, target_path: &Path, log_path: &Path) {
    let log_entry = format_log_entry(program_name, target_path);

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = file.write_all(log_entry.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_write_usage_to_file_creates_and_appends_entries() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("telemetry.log");

        write_usage_to_file("node", Path::new("/usr/bin/node"), &log_path);
        write_usage_to_file("go", Path::new("/usr/local/go/bin/go"), &log_path);

        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        assert!(lines[0].contains("\"tool\": \"node\""));
        assert!(lines[0].contains("\"target\": \"/usr/bin/node\""));
        assert!(lines[0].contains("\"timestamp\": \""));

        assert!(lines[1].contains("\"tool\": \"go\""));
        assert!(lines[1].contains("\"target\": \"/usr/local/go/bin/go\""));
        assert!(lines[1].contains("\"timestamp\": \""));
    }

    #[test]
    fn test_write_usage_to_file_ignores_unwritable_destination() {
        let dir = tempdir().unwrap();
        // Passing a directory path or a file inside a non-existent directory must silently succeed
        // without panicking.
        write_usage_to_file("node", Path::new("/usr/bin/node"), dir.path());
        write_usage_to_file(
            "node",
            Path::new("/usr/bin/node"),
            &dir.path().join("nonexistent_dir").join("telemetry.log"),
        );
    }
}

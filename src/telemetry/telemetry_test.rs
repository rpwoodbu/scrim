use std::fs;
use std::path::Path;
use telemetry::write_usage_to_file;
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

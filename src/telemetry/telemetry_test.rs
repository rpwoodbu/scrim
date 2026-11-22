use std::fs;
use std::path::Path;
use telemetry::{write_usage_to_file, write_usage_to_files};
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
fn test_write_usage_to_file_creates_missing_parent_directories() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("nested/state/scrim/telemetry.log");

    assert!(!log_path.parent().unwrap().exists());
    write_usage_to_file("node", Path::new("/usr/bin/node"), &log_path);

    let content = fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("\"tool\": \"node\""));
    assert!(content.contains("\"target\": \"/usr/bin/node\""));
}

#[test]
fn test_write_usage_to_file_ignores_unwritable_destination() {
    let dir = tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "not a directory").unwrap();

    // Passing a directory path or a path whose ancestor is a regular file must silently succeed
    // without panicking.
    write_usage_to_file("node", Path::new("/usr/bin/node"), dir.path());
    write_usage_to_file(
        "node",
        Path::new("/usr/bin/node"),
        &blocker.join("telemetry.log"),
    );
}

#[test]
fn test_write_usage_to_files_writes_multiple_sinks_and_ignores_errors() {
    let dir = tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "not a directory").unwrap();

    let bad_path = blocker.join("telemetry.log");
    let log_a = dir.path().join("a/telemetry.log");
    let log_b = dir.path().join("b/nested/telemetry.log");

    write_usage_to_files(
        "bazel",
        Path::new("/opt/bin/bazel"),
        &[bad_path, log_a.clone(), log_b.clone()],
    );

    let content_a = fs::read_to_string(&log_a).unwrap();
    let content_b = fs::read_to_string(&log_b).unwrap();
    assert_eq!(content_a, content_b);
    assert!(content_a.contains("\"tool\": \"bazel\""));
    assert!(content_a.contains("\"target\": \"/opt/bin/bazel\""));
}

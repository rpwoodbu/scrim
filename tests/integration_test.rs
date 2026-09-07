use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn find_scrim_bin() -> PathBuf {
    // 1. Try RUNFILES_DIR environment variable (Bazel standard)
    if let Ok(runfiles_dir) = env::var("RUNFILES_DIR") {
        let path = Path::new(&runfiles_dir).join("scrim").join("scrim");
        if path.exists() {
            return path;
        }
        let path = Path::new(&runfiles_dir).join("__main__").join("scrim");
        if path.exists() {
            return path;
        }
    }
    // 2. Try relative to current executable
    if let Ok(current_exe) = env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let path = dir.join("scrim");
            if path.exists() {
                return path;
            }
        }
    }
    // 3. Fallback to standard bazel-bin
    let path = PathBuf::from("bazel-bin/scrim");
    if path.exists() {
        return path;
    }
    panic!("Could not locate 'scrim' binary for integration test");
}

fn compute_sha256(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("Failed to run sha256sum");
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout.split_whitespace().next().unwrap().to_string()
}

#[test]
fn test_e2e_symlink_and_fetch() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a mock tool source binary/script
    let mock_tool_src = temp_path.join("mock_tool_src");
    fs::write(
        &mock_tool_src,
        "#!/bin/sh\necho \"Mock Tool Executed: args=$*\"\n",
    )
    .unwrap();
    
    // Make sure the mock source is readable (and executable just to be safe)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mock_tool_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_tool_src, perms).unwrap();
    }

    let sha256 = compute_sha256(&mock_tool_src);

    // 2. Create scrim.yaml in the temp directory
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  demotool:
    url: "file://{}"
    sha256: "{}"
"#,
        mock_tool_src.to_str().unwrap(),
        sha256
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // 3. Create a symlink named "demotool" pointing to "scrim"
    let shim_path = temp_path.join("demotool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // Override HOME to isolate the cache
    let cache_home = temp_path.join("fake_home");
    fs::create_dir_all(&cache_home).unwrap();

    // 4. Execute the symlink shim
    let output = Command::new(&shim_path)
        .arg("hello")
        .arg("world")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to execute demotool shim");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    println!("STDOUT: {}", stdout);
    println!("STDERR: {}", stderr);

    assert!(output.status.success(), "Shim execution failed!");
    assert!(
        stdout.contains("Mock Tool Executed: args=hello world"),
        "Stdout did not contain expected output: {:?}",
        stdout
    );

    // 5. Verify the tool was cached
    let expected_cache_path = cache_home
        .join(".cache/scrim/tools/demotool")
        .join(&sha256)
        .join("demotool");
    assert!(expected_cache_path.exists(), "Cache path not populated!");

    // 6. Execute again to verify cache hit
    let output_cached = Command::new(&shim_path)
        .arg("second")
        .arg("run")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to execute demotool shim on cache hit");

    let stdout_cached = String::from_utf8(output_cached.stdout).unwrap();
    assert!(output_cached.status.success());
    assert!(stdout_cached.contains("Mock Tool Executed: args=second run"));
}

#[test]
fn test_e2e_local_path_resolution() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a local mock tool
    let local_tool = temp_path.join("my_local_tool");
    fs::write(
        &local_tool,
        "#!/bin/sh\necho \"Local tool run with: $*\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&local_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&local_tool, perms).unwrap();
    }

    // 2. Create scrim.yaml pointing to the local tool
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  node:
    path: "{}"
"#,
        local_tool.to_str().unwrap()
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // 3. Create symlink node -> scrim
    let shim_path = temp_path.join("node");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // 4. Run the shim
    let output = Command::new(&shim_path)
        .arg("app.js")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run local path shim");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success());
    assert!(stdout.contains("Local tool run with: app.js"));
}

#[test]
fn test_e2e_telemetry_logging() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a local mock tool
    let local_tool = temp_path.join("my_local_tool");
    fs::write(
        &local_tool,
        "#!/bin/sh\necho \"Telemetry test run\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&local_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&local_tool, perms).unwrap();
    }

    // 2. Create scrim.yaml with telemetry enabled
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: true
tools:
  node:
    path: "{}"
"#,
        local_tool.to_str().unwrap()
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // Clear previous log if any
    let log_path = Path::new("/tmp/scrim_telemetry.log");
    if log_path.exists() {
        let _ = fs::remove_file(log_path);
    }

    // 3. Create symlink node -> scrim
    let shim_path = temp_path.join("node");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // 4. Run the shim
    let output = Command::new(&shim_path)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run local path shim");

    assert!(output.status.success());

    // 5. Poll the log file up to a 1-second timeout (checking every 5ms)
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(1);
    let poll_interval = std::time::Duration::from_millis(5);
    let mut log_content = String::new();
    let mut success = false;

    while start.elapsed() < timeout {
        if log_path.exists() {
            if let Ok(content) = fs::read_to_string(log_path) {
                if content.contains("\"tool\": \"node\"") {
                    log_content = content;
                    success = true;
                    break;
                }
            }
        }
        std::thread::sleep(poll_interval);
    }

    // Clean up before asserting (so we don't leave artifacts if assertion fails)
    if log_path.exists() {
        let _ = fs::remove_file(log_path);
    }

    assert!(
        success,
        "Telemetry log did not write correctly within 1s timeout! Captured content: {:?}",
        log_content
    );
}

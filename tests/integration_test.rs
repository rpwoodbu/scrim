use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn find_scrim_bin() -> PathBuf {
    if let Ok(runfiles_dir) = env::var("RUNFILES_DIR") {
        let path = Path::new(&runfiles_dir).join("_main").join("src").join("scrim");
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
    let path = PathBuf::from("bazel-bin/src/scrim");
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

    // Clear previous log if any
    let log_path = Path::new("/tmp/scrim_telemetry.log");
    if log_path.exists() {
        let _ = fs::remove_file(log_path);
    }

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
        .join(".cache/scrim/tools")
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

    // Verify telemetry did not write any log entries (since telemetry: false is set)
    std::thread::sleep(std::time::Duration::from_millis(50));
    if log_path.exists() {
        let content = fs::read_to_string(log_path).unwrap();
        assert!(
            !content.contains("\"tool\": \"demotool\""),
            "Telemetry reported usage even though telemetry: false was set! Log content: {:?}",
            content
        );
    }
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
    system_path: "{}"
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
    system_path: "{}"
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
    let timeout = std::time::Duration::from_secs(5);
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
        "Telemetry log did not write correctly within 5s timeout! Captured content: {:?}",
        log_content
    );
}

#[test]
fn test_e2e_telemetry_disabled_by_default() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a local mock tool
    let local_tool = temp_path.join("my_local_tool");
    fs::write(
        &local_tool,
        "#!/bin/sh\necho \"Telemetry test run (disabled by default)\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&local_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&local_tool, perms).unwrap();
    }

    // 2. Create scrim.yaml WITHOUT specifying telemetry (should default to false)
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
tools:
  node:
    system_path: "{}"
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

    // 5. Verify telemetry did not write any log entries (since it defaults to false)
    std::thread::sleep(std::time::Duration::from_millis(50));
    if log_path.exists() {
        let content = fs::read_to_string(log_path).unwrap();
        assert!(
            !content.contains("\"tool\": \"node\""),
            "Telemetry reported usage even though telemetry is not specified (should default to false)! Log content: {:?}",
            content
        );
    }
}

#[test]
fn test_e2e_http_download() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create the mock tool file locally to compute its hash and get its bytes
    let temp_src = temp_path.join("temp_src");
    let mock_content = "#!/bin/sh\necho \"HTTP Mock Executed: args=$*\"\n";
    fs::write(&temp_src, mock_content).unwrap();

    let sha256 = compute_sha256(&temp_src);
    let mock_bytes = fs::read(&temp_src).unwrap();

    // 2. Start our inline mock HTTP server on an ephemeral port
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind TcpListener");
    let port = listener.local_addr().unwrap().port();
    let mock_server_url = format!("http://127.0.0.1:{}/demotool", port);

    let server_thread = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);

            // Respond with HTTP/1.1 OK and static file content
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                mock_bytes.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&mock_bytes);
            let _ = stream.flush();
        }
    });

    // 3. Create scrim.yaml in the temp directory pointing to the localhost mock URL
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  demotool:
    url: "{}"
    sha256: "{}"
"#,
        mock_server_url,
        sha256
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // 4. Create a symlink named "demotool" pointing to "scrim"
    let shim_path = temp_path.join("demotool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // Override HOME to isolate the cache
    let cache_home = temp_path.join("fake_home");
    fs::create_dir_all(&cache_home).unwrap();

    // 5. Execute the symlink shim
    let output = Command::new(&shim_path)
        .arg("hello")
        .arg("network")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to execute demotool shim via HTTP");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    println!("STDOUT: {}", stdout);
    println!("STDERR: {}", stderr);

    assert!(output.status.success(), "HTTP download execution failed!");
    assert!(
        stdout.contains("HTTP Mock Executed: args=hello network"),
        "Stdout did not contain expected network output: {:?}",
        stdout
    );
    assert!(
        stderr.contains("[Scrim] Downloading demotool..."),
        "Stderr did not contain downloading UX message. Stderr: {:?}",
        stderr
    );

    // Wait for the server thread to finish cleanly
    server_thread.join().expect("HTTP mock server thread panicked");

    // 6. Verify cache population
    let expected_cache_path = cache_home
        .join(".cache/scrim/tools")
        .join(&sha256)
        .join("demotool");
    assert!(expected_cache_path.exists(), "HTTP cached path not populated!");
}

#[test]
fn test_e2e_unpack_output() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a mock tool script inside a directory to be archived
    let archive_src_dir = temp_path.join("archive_src");
    fs::create_dir_all(&archive_src_dir).unwrap();
    let mock_tool = archive_src_dir.join("demotool");
    fs::write(&mock_tool, "#!/bin/sh\necho \"Archived Tool\"\n").unwrap();
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mock_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_tool, perms).unwrap();
    }

    // 2. Create the tar.gz archive
    let archive_path = temp_path.join("mock.tar.gz");
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&archive_src_dir)
        .arg("demotool")
        .status()
        .expect("Failed to create mock tar.gz");
    assert!(status.success());

    let sha256 = compute_sha256(&archive_path);

    // 3. Create scrim.yaml pointing to the file:// URL of the archive
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  demotool:
    url: "file://{}"
    sha256: "{}"
"#,
        archive_path.to_str().unwrap(),
        sha256
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // 4. Create symlink demotool -> scrim
    let shim_path = temp_path.join("demotool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    let cache_home = temp_path.join("fake_home");
    fs::create_dir_all(&cache_home).unwrap();

    // 5. Run the shim and capture output
    let output = Command::new(&shim_path)
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to run shim");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "Shim execution failed! Stderr: {}", stderr);
    assert!(stdout.contains("Archived Tool"));
    
    // 6. Verify UX requirement: Output unpacking progress
    assert!(
        stderr.contains("[Scrim] Unpacking demotool archive..."),
        "Stderr did not contain the attributed unpacking UX message. Stderr: {:?}",
        stderr
    );
}

#[test]
fn test_e2e_error_attribution() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create an invalid scrim.yaml
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = r#"
tools:
  badtool:
    system_path: /bin/ls
    url: https://example.com/ls
"#;
    fs::write(&scrim_yaml, config_content).unwrap();

    // 2. Create symlink badtool -> scrim
    let shim_path = temp_path.join("badtool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // 3. Run the shim and capture output
    let output = Command::new(&shim_path)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run shim");

    let stderr = String::from_utf8(output.stderr).unwrap();

    // 4. Verify UX requirement: Error is explicit and attributed
    assert!(!output.status.success());
    assert!(
        stderr.starts_with("[Scrim] Error:"),
        "Stderr did not start with the attributed [Scrim] Error prefix. Stderr: {:?}",
        stderr
    );
}

#[test]
fn test_e2e_management_cli() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Run without arguments
    let output = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim without args");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Run 'scrim help' for usage."));

    // 2. Run with 'help'
    let output = Command::new(&scrim_bin)
        .arg("help")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim help");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Scrim: The transparent tool proxy."));
    assert!(stdout.contains("Commands:"));

    // 3. Run with 'version'
    let output = Command::new(&scrim_bin)
        .arg("version")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim version");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("scrim 0.1.0"));

    // 4. Run with 'config'
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = r#"telemetry: false
tools:
  demotool:
    system_path: /bin/echo"#;
    std::fs::write(&scrim_yaml, config_content).unwrap();

    let output = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("telemetry: false"));
    assert!(stdout.contains("system_path: /bin/echo"));
    assert!(!stdout.contains("null"), "Output should not contain null entries");
    assert!(!stdout.contains("url:"), "Unspecified url should be omitted");
    assert!(!stdout.contains("sha256:"), "Unspecified sha256 should be omitted");
    assert!(!stdout.contains("archive_path:"), "Unspecified archive_path should be omitted");
    assert!(!stdout.contains("template:"), "Unspecified template should be omitted");

    // 4.5 Run with 'config' on an empty config to verify empty collections are omitted
    std::fs::write(&scrim_yaml, "{}").unwrap();
    let output = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config on empty config");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("tools:"), "Empty tools collection should be omitted");

    // 5. Run with invalid argument
    let output = Command::new(&scrim_bin)
        .arg("unknown_command")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim unknown");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Run 'scrim help' for usage."));
}

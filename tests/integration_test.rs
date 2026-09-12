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
    assert!(stdout.contains(&format!("scrim {}", scrim_lib::VERSION)));

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

#[test]
fn test_e2e_links_command() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Missing directory argument should fail with attributed error
    let output_no_args = Command::new(&scrim_bin)
        .arg("links")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links");
    let stderr_no_args = String::from_utf8(output_no_args.stderr).unwrap();
    assert!(!output_no_args.status.success());
    assert!(
        stderr_no_args.contains("[Scrim] Error: directory argument required for links command"),
        "Unexpected error: {}",
        stderr_no_args
    );

    // 2. Non-existent directory should fail with attributed error
    let non_existent_dir = temp_path.join("missing_dir");
    let output_missing = Command::new(&scrim_bin)
        .arg("links")
        .arg(&non_existent_dir)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links missing_dir");
    let stderr_missing = String::from_utf8(output_missing.stderr).unwrap();
    assert!(!output_missing.status.success());
    assert!(
        stderr_missing.contains("[Scrim] Error: Target directory"),
        "Unexpected error: {}",
        stderr_missing
    );

    // 3. Create target directory and mock local tool
    let target_bin = temp_path.join("bin");
    fs::create_dir_all(&target_bin).unwrap();

    let local_tool = temp_path.join("my_tool");
    fs::write(
        &local_tool,
        "#!/bin/sh\necho \"Tool run: $*\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&local_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&local_tool, perms).unwrap();
    }

    // Create scrim.yaml defining tool "tool1"
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
tools:
  tool1:
    system_path: "{}"
"#,
        local_tool.to_str().unwrap()
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // Create a stale symlink in target_bin pointing to scrim_bin
    let stale_link = target_bin.join("obsolete_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &stale_link).unwrap();

    // Create an unrelated file in target_bin
    let unrelated_file = target_bin.join("unrelated.txt");
    fs::write(&unrelated_file, "keep me").unwrap();

    // 4. Run `scrim links <target_bin>`
    let output_links = Command::new(&scrim_bin)
        .arg("links")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links <target_bin>");
    let stderr_links = String::from_utf8(output_links.stderr).unwrap();

    assert!(output_links.status.success(), "scrim links command failed: {}", stderr_links);
    assert!(stderr_links.contains("[Scrim] Linked tool1"));
    assert!(stderr_links.contains("[Scrim] Removed obsolete link obsolete_tool"));

    // Verify tool1 link was created and works
    let tool1_link = target_bin.join("tool1");
    assert!(tool1_link.exists());
    assert!(!stale_link.exists());
    assert!(unrelated_file.exists());

    // 5. Execute the created tool1 link as a shim
    let output_shim = Command::new(&tool1_link)
        .arg("arg1")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run created tool1 shim");
    let stdout_shim = String::from_utf8(output_shim.stdout).unwrap();
    assert!(output_shim.status.success());
    assert!(stdout_shim.contains("Tool run: arg1"));

    // 6. Run `scrim links <target_bin>` again; should not log that it linked already-existing correct links
    let output_links_rerun = Command::new(&scrim_bin)
        .arg("links")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links <target_bin> second time");
    let stderr_links_rerun = String::from_utf8(output_links_rerun.stderr).unwrap();
    assert!(output_links_rerun.status.success());
    assert!(
        !stderr_links_rerun.contains("[Scrim] Linked tool1"),
        "Expected no logging for already existing link, got: {}",
        stderr_links_rerun
    );
    assert!(
        stderr_links_rerun.contains("[Scrim] All links are correct"),
        "Expected 'All links are correct' report, got: {}",
        stderr_links_rerun
    );

    // 7. Add tools with conflicts (one regular file, one symlink pointing elsewhere, one valid new tool)
    let config_conflicts = format!(
        r#"
tools:
  tool1:
    system_path: "{}"
  regular_conflict:
    system_path: "{}"
  symlink_conflict:
    system_path: "{}"
  valid_new_tool:
    system_path: "{}"
"#,
        local_tool.to_str().unwrap(),
        local_tool.to_str().unwrap(),
        local_tool.to_str().unwrap(),
        local_tool.to_str().unwrap(),
    );
    fs::write(&scrim_yaml, config_conflicts).unwrap();

    let reg_file = target_bin.join("regular_conflict");
    fs::write(&reg_file, "existing file content").unwrap();

    let sym_conflict = target_bin.join("symlink_conflict");
    #[cfg(unix)]
    std::os::unix::fs::symlink(Path::new("/bin/echo"), &sym_conflict).unwrap();

    let output_conflicts = Command::new(&scrim_bin)
        .arg("links")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links with conflicts");

    let stderr_conflicts = String::from_utf8(output_conflicts.stderr).unwrap();
    // Exits non-zero because conflicts occurred
    assert!(!output_conflicts.status.success());
    // Reports errors for both conflicting items
    assert!(stderr_conflicts.contains("[Scrim] Error:"));
    assert!(stderr_conflicts.contains("regular_conflict' already exists"));
    assert!(stderr_conflicts.contains("symlink_conflict' already exists and points elsewhere"));
    assert!(stderr_conflicts.contains("One or more links could not be created"));
    // Continues processing and successfully links valid_new_tool
    assert!(stderr_conflicts.contains("[Scrim] Linked valid_new_tool"));
    // Existing files must NOT have been overwritten
    assert_eq!(fs::read_to_string(&reg_file).unwrap(), "existing file content");
    assert_eq!(fs::read_link(&sym_conflict).unwrap(), Path::new("/bin/echo"));
    assert!(target_bin.join("valid_new_tool").exists());
}


use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn find_scrim_bin() -> PathBuf {
    if let Ok(runfiles_dir) = env::var("RUNFILES_DIR") {
        let path = Path::new(&runfiles_dir).join("_main").join("src").join("bin").join("scrim");
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
    let path = PathBuf::from("bazel-bin/src/bin/scrim");
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

fn compute_sha512(path: &Path) -> String {
    let output = Command::new("sha512sum")
        .arg(path)
        .output()
        .expect("Failed to run sha512sum");
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
        .join(format!("sha256:{}", sha256))
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
        .join(format!("sha256:{}", sha256))
        .join("demotool");
    assert!(expected_cache_path.exists(), "HTTP cached path not populated!");
}

#[test]
fn test_e2e_http_ipv6_download() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create the mock tool file locally to compute its hash and get its bytes
    let temp_src = temp_path.join("temp_src_ipv6");
    let mock_content = "#!/bin/sh\necho \"HTTP IPv6 Mock Executed: args=$*\"\n";
    fs::write(&temp_src, mock_content).unwrap();

    let sha256 = compute_sha256(&temp_src);
    let mock_bytes = fs::read(&temp_src).unwrap();

    // 2. Start our inline mock IPv6 HTTP server on an ephemeral port
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    let listener = match TcpListener::bind("[::1]:0") {
        Ok(l) => l,
        Err(_) => return, // Gracefully skip if IPv6 loopback is unavailable in environment
    };
    let port = listener.local_addr().unwrap().port();
    let mock_server_url = format!("http://[::1]:{}/demotool_ipv6", port);

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

    // 3. Create scrim.yaml pointing to the IPv6 loopback URL
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  demotool_ipv6:
    url: "{}"
    sha256: "{}"
"#,
        mock_server_url,
        sha256
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // 4. Create a symlink named "demotool_ipv6" pointing to "scrim"
    let shim_path = temp_path.join("demotool_ipv6");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // Override HOME to isolate the cache
    let cache_home = temp_path.join("fake_home");
    fs::create_dir_all(&cache_home).unwrap();

    // 5. Execute the symlink shim
    let output = Command::new(&shim_path)
        .arg("hello")
        .arg("ipv6")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to execute demotool_ipv6 shim via HTTP");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    println!("STDOUT: {}", stdout);
    println!("STDERR: {}", stderr);

    assert!(output.status.success(), "HTTP IPv6 download execution failed!");
    assert!(
        stdout.contains("HTTP IPv6 Mock Executed: args=hello ipv6"),
        "Stdout did not contain expected network output: {:?}",
        stdout
    );
    assert!(
        stderr.contains("[Scrim] Downloading demotool_ipv6..."),
        "Stderr did not contain downloading UX message. Stderr: {:?}",
        stderr
    );

    // Wait for the server thread to finish cleanly
    server_thread.join().expect("HTTP mock server thread panicked");

    // 6. Verify cache population
    let expected_cache_path = cache_home
        .join(".cache/scrim/tools")
        .join(format!("sha256:{}", sha256))
        .join("demotool_ipv6");
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
fn test_e2e_tar_xz_unpack_output() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a mock tool script inside a directory to be archived
    let archive_src_dir = temp_path.join("archive_src");
    fs::create_dir_all(&archive_src_dir).unwrap();
    let mock_tool = archive_src_dir.join("demotool");
    fs::write(&mock_tool, "#!/bin/sh\necho \"Archived XZ Tool\"\n").unwrap();
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mock_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_tool, perms).unwrap();
    }

    // 2. Create the tar.xz archive
    let archive_path = temp_path.join("mock.tar.xz");
    let status = Command::new("tar")
        .arg("-cJf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&archive_src_dir)
        .arg("demotool")
        .status()
        .expect("Failed to create mock tar.xz");
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
    assert!(stdout.contains("Archived XZ Tool"));
    
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
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Scrim: The transparent tool proxy."));
    assert!(stdout.contains("Commands:"));

    // 2. Run with 'help'
    let output = Command::new(&scrim_bin)
        .arg("help")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim help");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Scrim: The transparent tool proxy."));
    assert!(stdout.contains("Commands:"));
    assert!(stdout.contains("run      Runs a defined tool directly without requiring a link."));

    // 2.1 Run with '--help' and '-h'
    let output_help_flag = Command::new(&scrim_bin)
        .arg("--help")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim --help");
    assert!(output_help_flag.status.success());
    let stdout_help_flag = String::from_utf8(output_help_flag.stdout).unwrap();
    assert!(stdout_help_flag.contains("Scrim: The transparent tool proxy."));
    assert!(stdout_help_flag.contains("Commands:"));
    assert!(stdout_help_flag.contains("Environment Variables:"));
    assert!(stdout_help_flag.contains("XDG_CACHE_HOME"));
    assert!(stdout_help_flag.contains("XDG_CONFIG_HOME"));

    let output_h_flag = Command::new(&scrim_bin)
        .arg("-h")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim -h");
    assert!(output_h_flag.status.success());
    let stdout_h_flag = String::from_utf8(output_h_flag.stdout).unwrap();
    assert!(stdout_h_flag.contains("Scrim: The transparent tool proxy."));

    // 3. Run with 'version', '--version', and '-V'
    for ver_arg in ["version", "--version", "-V"] {
        let output = Command::new(&scrim_bin)
            .arg(ver_arg)
            .current_dir(temp_path)
            .output()
            .unwrap_or_else(|_| panic!("Failed to run scrim {}", ver_arg));
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let trimmed = stdout.trim();
        assert!(trimmed.starts_with(&format!("scrim {}", version::VERSION)));
        if trimmed != format!("scrim {}", version::VERSION) {
            assert!(trimmed.starts_with(&format!("scrim {} (", version::VERSION)));
            assert!(trimmed.ends_with(')'));
        }
    }

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

    // 4.1 Run with 'config demotool' to print only demotool's configuration
    let output_tool = Command::new(&scrim_bin)
        .arg("config")
        .arg("demotool")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config demotool");
    assert!(output_tool.status.success());
    let stdout_tool = String::from_utf8(output_tool.stdout).unwrap();
    assert!(stdout_tool.contains("system_path: /bin/echo"));
    assert!(!stdout_tool.contains("telemetry"), "Telemetry should not appear in tool config");
    assert!(!stdout_tool.contains("demotool:"), "Tool name key should not wrap the tool config");
    assert!(!stdout_tool.contains("tools:"), "Tools collection key should not appear in tool config");
    assert!(!stdout_tool.contains("null"), "Output should not contain null entries");
    assert!(!stdout_tool.contains("url:"), "Unspecified url should be omitted");

    // 4.2 Run with 'config nonexistent' to verify error handling
    let output_missing = Command::new(&scrim_bin)
        .arg("config")
        .arg("nonexistent")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config nonexistent");
    assert!(!output_missing.status.success());
    let stderr_missing = String::from_utf8(output_missing.stderr).unwrap();
    assert!(stderr_missing.contains("[Scrim] Error: Tool 'nonexistent' not found in config"));

    // 4.3 Run with 'config demotool extra' to verify unexpected argument error
    let output_extra = Command::new(&scrim_bin)
        .arg("config")
        .arg("demotool")
        .arg("extra")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config demotool extra");
    assert!(!output_extra.status.success());
    let stderr_extra = String::from_utf8(output_extra.stderr).unwrap();
    assert!(stderr_extra.contains("[Scrim] Error: unexpected argument 'extra'"));

    // 4.4 Run with 'config --undefined-flag' to verify undefined flag error
    let output_conf_flag = Command::new(&scrim_bin)
        .arg("config")
        .arg("--undefined-flag")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config --undefined-flag");
    assert!(!output_conf_flag.status.success());
    let stderr_conf_flag = String::from_utf8(output_conf_flag.stderr).unwrap();
    assert!(stderr_conf_flag.contains("[Scrim] Error: unexpected argument '--undefined-flag'"));

    // 4.5 Run with 'config' on an empty config to verify empty collections are omitted
    std::fs::write(&scrim_yaml, "{}").unwrap();
    let output = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config on empty config");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("tools:"), "Empty tools collection should be omitted");

    // 4.6 Run with 'config' on multiple tools to verify output is sorted by tool name
    let sorted_test_config = r#"telemetry: true
tools:
  zebra:
    system_path: /bin/echo
  apple:
    system_path: /bin/cat
  mango:
    system_path: /bin/ls
"#;
    std::fs::write(&scrim_yaml, sorted_test_config).unwrap();
    let output = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config for sorting test");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let apple_pos = stdout.find("apple:").expect("apple should be in output");
    let mango_pos = stdout.find("mango:").expect("mango should be in output");
    let zebra_pos = stdout.find("zebra:").expect("zebra should be in output");
    assert!(apple_pos < mango_pos, "apple must precede mango in output");
    assert!(mango_pos < zebra_pos, "mango must precede zebra in output");

    // 5. Run with invalid argument
    let output = Command::new(&scrim_bin)
        .arg("unknown_command")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim unknown");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!output.status.success());
    assert!(stderr.contains("[Scrim] Error: unrecognized subcommand 'unknown_command'"));

    // 6. Run with undefined flag to scrim
    let output_flag = Command::new(&scrim_bin)
        .arg("--undefined-flag")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim with undefined flag");
    let stderr_flag = String::from_utf8(output_flag.stderr).unwrap();
    assert!(!output_flag.status.success());
    assert!(stderr_flag.contains("[Scrim] Error: unexpected argument '--undefined-flag'"));
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

#[test]
fn test_e2e_links_dry_run() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Missing directory argument with -n should fail with exit code 1
    let output_no_args = Command::new(&scrim_bin)
        .arg("links")
        .arg("-n")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links -n");
    assert_eq!(output_no_args.status.code(), Some(1));
    let stderr_no_args = String::from_utf8(output_no_args.stderr).unwrap();
    assert!(stderr_no_args.contains("[Scrim] Error: directory argument required for links command"));

    // 2. Non-existent directory with -n should fail with exit code 1
    let non_existent_dir = temp_path.join("missing_dir");
    let output_missing = Command::new(&scrim_bin)
        .arg("links")
        .arg("-n")
        .arg(&non_existent_dir)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links -n missing_dir");
    assert_eq!(output_missing.status.code(), Some(1));
    let stderr_missing = String::from_utf8(output_missing.stderr).unwrap();
    assert!(stderr_missing.contains("[Scrim] Error: Target directory"));

    // 3. Setup target directory, config, and stale link
    let target_bin = temp_path.join("bin");
    fs::create_dir_all(&target_bin).unwrap();

    let local_tool = temp_path.join("my_tool");
    fs::write(&local_tool, "#!/bin/sh\necho \"Tool run: $*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&local_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&local_tool, perms).unwrap();
    }

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

    let stale_link = target_bin.join("obsolete_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &stale_link).unwrap();

    // 4. Run `scrim links -n <target_bin>`: changes would be made -> exit code 2
    let output_dry_run_short = Command::new(&scrim_bin)
        .arg("links")
        .arg("-n")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links -n <target_bin>");
    assert_eq!(output_dry_run_short.status.code(), Some(2));
    let stderr_dry_run_short = String::from_utf8(output_dry_run_short.stderr).unwrap();
    assert!(stderr_dry_run_short.to_lowercase().contains("dry-run"));
    assert!(stderr_dry_run_short.contains("[Scrim] Would link tool1"));
    assert!(stderr_dry_run_short.contains("[Scrim] Would remove obsolete link obsolete_tool"));

    // Verify filesystem was NOT modified
    let tool1_link = target_bin.join("tool1");
    assert!(!tool1_link.exists());
    assert!(stale_link.exists());

    // 5. Test with `--dry-run` long flag as well
    let output_dry_run_long = Command::new(&scrim_bin)
        .arg("links")
        .arg("--dry-run")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links --dry-run <target_bin>");
    assert_eq!(output_dry_run_long.status.code(), Some(2));
    let stderr_dry_run_long = String::from_utf8(output_dry_run_long.stderr).unwrap();
    assert!(stderr_dry_run_long.contains("[Scrim] Would link tool1"));
    assert!(stderr_dry_run_long.contains("[Scrim] Would remove obsolete link obsolete_tool"));

    // 6. Run actual `scrim links <target_bin>` to apply changes
    let output_actual = Command::new(&scrim_bin)
        .arg("links")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run actual scrim links");
    assert_eq!(output_actual.status.code(), Some(0));
    let stderr_actual = String::from_utf8(output_actual.stderr).unwrap();
    assert!(!stderr_actual.to_lowercase().contains("dry-run"));
    assert!(tool1_link.exists());
    assert!(!stale_link.exists());

    // 7. Run `scrim links -n <target_bin>` when all links are already correct -> exit code 0
    let output_dry_clean = Command::new(&scrim_bin)
        .arg("links")
        .arg("-n")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links -n second time");
    assert_eq!(output_dry_clean.status.code(), Some(0));
    let stderr_dry_clean = String::from_utf8(output_dry_clean.stderr).unwrap();
    assert!(stderr_dry_clean.contains("[Scrim] All links are correct"));
    assert!(!stderr_dry_clean.contains("[Scrim] Would link tool1"));

    // 8. Run `scrim links -n <target_bin>` with conflict -> exit code 1
    let config_conflicts = format!(
        r#"
tools:
  tool1:
    system_path: "{}"
  conflicting_tool:
    system_path: "{}"
"#,
        local_tool.to_str().unwrap(),
        local_tool.to_str().unwrap(),
    );
    fs::write(&scrim_yaml, config_conflicts).unwrap();
    let conflict_file = target_bin.join("conflicting_tool");
    fs::write(&conflict_file, "conflict content").unwrap();

    let output_dry_conflict = Command::new(&scrim_bin)
        .arg("links")
        .arg("-n")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links -n with conflicts");
    assert_eq!(output_dry_conflict.status.code(), Some(1));
    let stderr_dry_conflict = String::from_utf8(output_dry_conflict.stderr).unwrap();
    assert!(stderr_dry_conflict.contains("[Scrim] Error: '"));
    assert!(stderr_dry_conflict.contains("conflicting_tool' already exists"));
    assert!(stderr_dry_conflict.contains("One or more links could not be created"));

    // 9. Verify help message documents exit codes
    let output_help = Command::new(&scrim_bin)
        .arg("links")
        .arg("--help")
        .output()
        .expect("Failed to run scrim links --help");
    assert!(output_help.status.success());
    let stdout_help = String::from_utf8(output_help.stdout).unwrap();
    assert!(stdout_help.to_lowercase().contains("exit code"));
}

#[test]
fn test_e2e_run_command() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Missing tool argument should fail with attributed error
    let output_no_tool = Command::new(&scrim_bin)
        .arg("run")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run without tool");
    let stderr_no_tool = String::from_utf8(output_no_tool.stderr).unwrap();
    assert!(!output_no_tool.status.success());
    assert!(stderr_no_tool.contains("[Scrim] Error: tool name argument required for run command"));

    // 2. Missing tool argument after '--' should fail with attributed error
    let output_dash_only = Command::new(&scrim_bin)
        .arg("run")
        .arg("--")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run --");
    let stderr_dash_only = String::from_utf8(output_dash_only.stderr).unwrap();
    assert!(!output_dash_only.status.success());
    assert!(stderr_dash_only.contains("[Scrim] Error: tool name argument required for run command"));

    // 3. Create a local mock tool script
    let mock_tool = temp_path.join("my_runner_tool");
    fs::write(
        &mock_tool,
        "#!/bin/sh\necho \"Executed via run: args=$*\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mock_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_tool, perms).unwrap();
    }

    // Create scrim.yaml defining 'mytool'
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  mytool:
    system_path: "{}"
"#,
        mock_tool.to_str().unwrap()
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // 4. Unknown tool should fail with attributed error
    let output_unknown = Command::new(&scrim_bin)
        .arg("run")
        .arg("nonexistent_tool")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run nonexistent_tool");
    let stderr_unknown = String::from_utf8(output_unknown.stderr).unwrap();
    assert!(!output_unknown.status.success());
    assert!(stderr_unknown.contains("[Scrim] Error: Tool 'nonexistent_tool' not found in config"));

    // 5. Successful run without link (positional arguments)
    let output_success = Command::new(&scrim_bin)
        .arg("run")
        .arg("mytool")
        .arg("arg1")
        .arg("arg2")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run mytool arg1 arg2");
    let stdout_success = String::from_utf8(output_success.stdout).unwrap();
    assert!(output_success.status.success());
    assert!(stdout_success.contains("Executed via run: args=arg1 arg2"));

    // 6. Passing flags to scrim run without '--' should fail because scrim defines no flags
    let output_flag_without_dash = Command::new(&scrim_bin)
        .arg("run")
        .arg("mytool")
        .arg("-v")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run mytool -v");
    let stderr_flag_without_dash = String::from_utf8(output_flag_without_dash.stderr).unwrap();
    assert!(!output_flag_without_dash.status.success());
    assert!(stderr_flag_without_dash.contains("[Scrim] Error: unexpected argument '-v'"));

    let output_long_flag_without_dash = Command::new(&scrim_bin)
        .arg("run")
        .arg("mytool")
        .arg("--my-flag")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run mytool --my-flag");
    let stderr_long_flag_without_dash = String::from_utf8(output_long_flag_without_dash.stderr).unwrap();
    assert!(!output_long_flag_without_dash.status.success());
    assert!(stderr_long_flag_without_dash.contains("[Scrim] Error: unexpected argument '--my-flag'"));

    // 7. Passing flags to tool with '--' preceding tool name
    let output_flags_dash_before = Command::new(&scrim_bin)
        .arg("run")
        .arg("--")
        .arg("mytool")
        .arg("-v")
        .arg("--my-flag=value")
        .arg("positional")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run -- mytool flags");
    let stdout_flags_dash_before = String::from_utf8(output_flags_dash_before.stdout).unwrap();
    assert!(output_flags_dash_before.status.success());
    assert!(stdout_flags_dash_before.contains("Executed via run: args=-v --my-flag=value positional"));

    // 8. Passing flags to tool with '--' succeeding tool name
    let output_flags_dash_after = Command::new(&scrim_bin)
        .arg("run")
        .arg("mytool")
        .arg("--")
        .arg("-v")
        .arg("--another-flag")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run mytool -- flags");
    let stdout_flags_dash_after = String::from_utf8(output_flags_dash_after.stdout).unwrap();
    assert!(output_flags_dash_after.status.success());
    assert!(stdout_flags_dash_after.contains("Executed via run: args=-v --another-flag"));

    // 8.1 Passing -h to tool via '--'
    let output_h_with_dash = Command::new(&scrim_bin)
        .arg("run")
        .arg("mytool")
        .arg("--")
        .arg("-h")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run mytool -- -h");
    let stdout_h_with_dash = String::from_utf8(output_h_with_dash.stdout).unwrap();
    assert!(output_h_with_dash.status.success());
    assert!(stdout_h_with_dash.contains("Executed via run: args=-h"));

    let output_h_dash_before = Command::new(&scrim_bin)
        .arg("run")
        .arg("--")
        .arg("mytool")
        .arg("-h")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run -- mytool -h");
    let stdout_h_dash_before = String::from_utf8(output_h_dash_before.stdout).unwrap();
    assert!(output_h_dash_before.status.success());
    assert!(stdout_h_dash_before.contains("Executed via run: args=-h"));

    // 8.2 Calling help on run subcommand directly
    let output_run_h = Command::new(&scrim_bin)
        .arg("run")
        .arg("-h")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run -h");
    assert!(output_run_h.status.success());
    let stdout_run_h = String::from_utf8(output_run_h.stdout).unwrap();
    assert!(stdout_run_h.contains("Runs a defined tool directly without requiring a link."));

    // 9. Passing '--' intended for the tool itself
    let output_tool_double_dash = Command::new(&scrim_bin)
        .arg("run")
        .arg("--")
        .arg("mytool")
        .arg("--")
        .arg("tool_arg")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim run -- mytool -- tool_arg");
    let stdout_tool_double_dash = String::from_utf8(output_tool_double_dash.stdout).unwrap();
    assert!(output_tool_double_dash.status.success());
    assert!(stdout_tool_double_dash.contains("Executed via run: args=-- tool_arg"));

    // 10. Running a fetched tool via run command
    let fetch_src = temp_path.join("fetch_src");
    fs::write(&fetch_src, "#!/bin/sh\necho \"Fetched via run: args=$*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fetch_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fetch_src, perms).unwrap();
    }
    let fetch_sha256 = compute_sha256(&fetch_src);

    let config_fetch = format!(
        r#"
telemetry: false
tools:
  fetched_tool:
    url: "file://{}"
    sha256: "{}"
"#,
        fetch_src.to_str().unwrap(),
        fetch_sha256
    );
    fs::write(&scrim_yaml, config_fetch).unwrap();

    let cache_home = temp_path.join("fake_home");
    fs::create_dir_all(&cache_home).unwrap();

    let output_fetch = Command::new(&scrim_bin)
        .arg("run")
        .arg("fetched_tool")
        .arg("downloaded_arg")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to run scrim run fetched_tool");
    let stdout_fetch = String::from_utf8(output_fetch.stdout).unwrap();
    assert!(output_fetch.status.success());
    assert!(stdout_fetch.contains("Fetched via run: args=downloaded_arg"));

    let expected_cached_binary = cache_home
        .join(".cache/scrim/tools")
        .join(format!("sha256:{}", fetch_sha256))
        .join("fetched_tool");
    assert!(expected_cached_binary.exists());
}

#[test]
fn test_e2e_url_aware_cache_invalidation_and_update_facilitation() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create v1 tool source
    let v1_src = temp_path.join("v1_tool_src");
    fs::write(&v1_src, "#!/bin/sh\necho \"Tool Version 1: args=$*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&v1_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&v1_src, perms).unwrap();
    }
    let v1_sha256 = compute_sha256(&v1_src);
    let v1_url = format!("file://{}", v1_src.to_str().unwrap());

    // 2. Configure scrim.yaml with v1
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_v1 = format!(
        r#"
telemetry: false
tools:
  mytool:
    url: "{}"
    sha256: "{}"
"#,
        v1_url, v1_sha256
    );
    fs::write(&scrim_yaml, config_v1).unwrap();

    let shim_path = temp_path.join("mytool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    let cache_home = temp_path.join("fake_home");
    fs::create_dir_all(&cache_home).unwrap();

    // 3. First execution: downloads and caches v1
    let output_v1 = Command::new(&shim_path)
        .arg("hello")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to execute shim for v1");
    assert!(output_v1.status.success());
    let stdout_v1 = String::from_utf8(output_v1.stdout).unwrap();
    assert!(stdout_v1.contains("Tool Version 1: args=hello"));

    let cache_tools = cache_home.join(".cache/scrim/tools");
    let v1_cache_bin = cache_tools.join(format!("sha256:{}", v1_sha256)).join("mytool");
    let v1_url_file = cache_tools.join(format!("sha256:{}.url", v1_sha256));
    assert!(v1_cache_bin.exists());
    assert!(v1_url_file.exists());
    assert!(fs::read_to_string(&v1_url_file).unwrap().contains(&v1_url));

    // 4. Second execution: hits cache for v1
    let output_v1_cache = Command::new(&shim_path)
        .arg("second")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to execute cached shim for v1");
    assert!(output_v1_cache.status.success());
    let stdout_v1_cache = String::from_utf8(output_v1_cache.stdout).unwrap();
    assert!(stdout_v1_cache.contains("Tool Version 1: args=second"));

    // 5. Create v2 tool source
    let v2_src = temp_path.join("v2_tool_src");
    fs::write(&v2_src, "#!/bin/sh\necho \"Tool Version 2: args=$*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&v2_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&v2_src, perms).unwrap();
    }
    let v2_sha256 = compute_sha256(&v2_src);
    let v2_url = format!("file://{}", v2_src.to_str().unwrap());

    // 6. User updates URL in scrim.yaml to v2, but does NOT update sha256 (still has v1_sha256)
    let config_v2_stale_hash = format!(
        r#"
telemetry: false
tools:
  mytool:
    url: "{}"
    sha256: "{}"
"#,
        v2_url, v1_sha256
    );
    fs::write(&scrim_yaml, config_v2_stale_hash).unwrap();

    // 7. Execution must NOT return the cached v1 binary.
    // It must download v2, find that hash does not match v1_sha256, and report expected vs actual hash.
    let output_mismatch = Command::new(&shim_path)
        .arg("test")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to run shim on URL bump with stale hash");
    assert!(!output_mismatch.status.success());
    let stderr_mismatch = String::from_utf8(output_mismatch.stderr).unwrap();
    let stdout_mismatch = String::from_utf8(output_mismatch.stdout).unwrap();

    // Crucially: Must NOT have run the cached v1 binary!
    assert!(!stdout_mismatch.contains("Tool Version 1"));
    // Error must report SHA256 mismatch with expected (v1) and actual (v2) hashes
    assert!(stderr_mismatch.contains("SHA256 verification failed for mytool."));
    assert!(stderr_mismatch.contains(&format!("Expected: {}", v1_sha256)));
    assert!(stderr_mismatch.contains(&format!("Actual:   {}", v2_sha256)));

    // 8. User updates sha256 in scrim.yaml to actual hash v2_sha256
    let config_v2_correct = format!(
        r#"
telemetry: false
tools:
  mytool:
    url: "{}"
    sha256: "{}"
"#,
        v2_url, v2_sha256
    );
    fs::write(&scrim_yaml, config_v2_correct).unwrap();

    // 9. Execution succeeds with v2
    let output_v2 = Command::new(&shim_path)
        .arg("v2_arg")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to execute shim after updating sha256 to v2");
    assert!(output_v2.status.success());
    let stdout_v2 = String::from_utf8(output_v2.stdout).unwrap();
    assert!(stdout_v2.contains("Tool Version 2: args=v2_arg"));

    let v2_cache_bin = cache_tools.join(format!("sha256:{}", v2_sha256)).join("mytool");
    let v2_url_file = cache_tools.join(format!("sha256:{}.url", v2_sha256));
    assert!(v2_cache_bin.exists());
    assert!(v2_url_file.exists());
    assert!(fs::read_to_string(&v2_url_file).unwrap().contains(&v2_url));
}

#[test]
fn test_e2e_config_tool_argument() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = r#"telemetry: true
tools:
  node:
    system_path: /bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
    archive_path: go/bin/go
  gofmt:
    template: go
    archive_path: go/bin/gofmt
"#;
    fs::write(&scrim_yaml, config_content).unwrap();

    // 1. Full config
    let output_all = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config");
    assert!(output_all.status.success());
    let stdout_all = String::from_utf8(output_all.stdout).unwrap();
    assert!(stdout_all.contains("telemetry: true"));
    assert!(stdout_all.contains("node:"));
    assert!(stdout_all.contains("go:"));
    assert!(stdout_all.contains("gofmt:"));

    // 2. Specific tool: node
    let output_node = Command::new(&scrim_bin)
        .arg("config")
        .arg("node")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config node");
    assert!(output_node.status.success());
    let stdout_node = String::from_utf8(output_node.stdout).unwrap();
    assert!(stdout_node.contains("system_path: /bin/node"));
    assert!(!stdout_node.contains("telemetry"));
    assert!(!stdout_node.contains("node:"));
    assert!(!stdout_node.contains("go:"));
    assert!(!stdout_node.contains("gofmt:"));

    // 3. Specific tool: go
    let output_go = Command::new(&scrim_bin)
        .arg("config")
        .arg("go")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config go");
    assert!(output_go.status.success());
    let stdout_go = String::from_utf8(output_go.stdout).unwrap();
    assert!(stdout_go.contains("url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz"));
    assert!(stdout_go.contains("sha256: 285c1f0624022839446d32"));
    assert!(stdout_go.contains("archive_path: go/bin/go"));
    assert!(!stdout_go.contains("system_path"));
    assert!(!stdout_go.contains("node:"));
    assert!(!stdout_go.contains("gofmt:"));

    // 4. Specific tool: gofmt
    let output_gofmt = Command::new(&scrim_bin)
        .arg("config")
        .arg("gofmt")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config gofmt");
    assert!(output_gofmt.status.success());
    let stdout_gofmt = String::from_utf8(output_gofmt.stdout).unwrap();
    assert!(stdout_gofmt.contains("template: go"));
    assert!(stdout_gofmt.contains("archive_path: go/bin/gofmt"));
    assert!(!stdout_gofmt.contains("url:"));
    assert!(!stdout_gofmt.contains("sha256:"));
    assert!(!stdout_gofmt.contains("node:"));

    // 5. Nonexistent tool
    let output_missing = Command::new(&scrim_bin)
        .arg("config")
        .arg("python")
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config python");
    assert!(!output_missing.status.success());
    let stderr_missing = String::from_utf8(output_missing.stderr).unwrap();
    assert!(stderr_missing.contains("[Scrim] Error: Tool 'python' not found in config"));
}

#[test]
fn test_e2e_xdg_base_directory_config_and_cache() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create mock tool
    let tool_src = temp_path.join("tool_src");
    fs::write(&tool_src, "#!/bin/sh\necho \"Tool Ran: args=$*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tool_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tool_src, perms).unwrap();
    }
    let sha256 = compute_sha256(&tool_src);
    let tool_url = format!("file://{}", tool_src.to_str().unwrap());

    let fake_home = temp_path.join("fake_home");
    let custom_xdg_config = temp_path.join("custom_xdg_config");
    let custom_xdg_cache = temp_path.join("custom_xdg_cache");
    fs::create_dir_all(&fake_home).unwrap();

    // 2. Put user config under <custom_xdg_config>/scrim/scrim.yaml
    let custom_scrim_dir = custom_xdg_config.join("scrim");
    fs::create_dir_all(&custom_scrim_dir).unwrap();
    fs::write(
        custom_scrim_dir.join("scrim.yaml"),
        format!(
            "telemetry: false\ntools:\n  mytool:\n    url: \"{}\"\n    sha256: \"{}\"\n",
            tool_url, sha256
        ),
    )
    .unwrap();

    let shim_path = temp_path.join("mytool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // Execute with custom XDG_CONFIG_HOME and XDG_CACHE_HOME
    let output = Command::new(&shim_path)
        .arg("arg_test")
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .env("XDG_CONFIG_HOME", &custom_xdg_config)
        .env("XDG_CACHE_HOME", &custom_xdg_cache)
        .output()
        .expect("Failed to execute shim with custom XDG dirs");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Tool Ran: args=arg_test"));

    // Tool should be cached under custom_xdg_cache/scrim/tools/<sha256>/mytool
    let cached_bin = custom_xdg_cache.join("scrim/tools").join(format!("sha256:{}", sha256)).join("mytool");
    assert!(cached_bin.exists(), "Binary was not cached under XDG_CACHE_HOME/scrim!");

    // Neither fake_home/.cache nor fake_home/.config should exist
    assert!(!fake_home.join(".cache").exists());
    assert!(!fake_home.join(".config").exists());
}

#[test]
fn test_e2e_xdg_cache_home_fallback() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create mock tool
    let tool_src = temp_path.join("tool_src");
    fs::write(&tool_src, "#!/bin/sh\necho \"Tool Ran: args=$*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tool_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tool_src, perms).unwrap();
    }
    let sha256 = compute_sha256(&tool_src);
    let tool_url = format!("file://{}", tool_src.to_str().unwrap());

    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        format!(
            "telemetry: false\ntools:\n  mytool:\n    url: \"{}\"\n    sha256: \"{}\"\n",
            tool_url, sha256
        ),
    )
    .unwrap();

    let shim_path = temp_path.join("mytool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    let fake_home = temp_path.join("fake_home");
    fs::create_dir_all(&fake_home).unwrap();

    let xdg_cache = temp_path.join("xdg_cache_root");

    // Case 1: XDG_CACHE_HOME set -> XDG_CACHE_HOME/scrim
    let output1 = Command::new(&shim_path)
        .arg("arg1")
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .env("XDG_CACHE_HOME", &xdg_cache)
        .output()
        .expect("Failed to execute shim with XDG_CACHE_HOME");

    assert!(output1.status.success());
    assert!(xdg_cache.join("scrim/tools").join(format!("sha256:{}", sha256)).join("mytool").exists());
    assert!(!fake_home.join(".cache").exists());

    // Case 2: XDG_CACHE_HOME relative -> ignored per spec, falls back to HOME/.cache/scrim
    let output2 = Command::new(&shim_path)
        .arg("arg2")
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .env("XDG_CACHE_HOME", "relative_xdg_cache")
        .output()
        .expect("Failed to execute shim with relative XDG_CACHE_HOME");

    assert!(output2.status.success());
    assert!(fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256)).join("mytool").exists());
}

#[test]
fn test_e2e_fetch_with_sha512() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a mock tool source binary/script
    let mock_tool_src = temp_path.join("mock_tool_src_sha512");
    fs::write(
        &mock_tool_src,
        "#!/bin/sh\necho \"Sha512 Tool Executed: args=$*\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mock_tool_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_tool_src, perms).unwrap();
    }

    let sha512 = compute_sha512(&mock_tool_src);

    // 2. Create scrim.yaml in the temp directory specifying sha512
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  demotool512:
    url: "file://{}"
    sha512: "{}"
"#,
        mock_tool_src.to_str().unwrap(),
        sha512
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    let shim_path = temp_path.join("demotool512");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    let fake_home = temp_path.join("fake_home");
    fs::create_dir_all(&fake_home).unwrap();

    let output = Command::new(&shim_path)
        .arg("hello")
        .arg("sha512")
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .expect("Failed to execute demotool512 shim");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Sha512 Tool Executed: args=hello sha512"));

    // Check cache directory has sha512:<hash>
    let expected_cached_binary = fake_home
        .join(".cache/scrim/tools")
        .join(format!("sha512:{}", sha512))
        .join("demotool512");
    assert!(expected_cached_binary.exists(), "Cache path with sha512: not populated!");

    let url_file = fake_home
        .join(".cache/scrim/tools")
        .join(format!("sha512:{}.url", sha512));
    assert!(url_file.exists(), "URL file with sha512: not populated!");
}

#[test]
fn test_e2e_digest_validation_errors() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let scrim_yaml = temp_path.join("scrim.yaml");
    // Tool with both sha256 and sha512
    fs::write(
        &scrim_yaml,
        r#"
telemetry: false
tools:
  multi_digest:
    url: "https://example.com/tool.tar.gz"
    sha256: "abc"
    sha512: "def"
"#,
    )
    .unwrap();

    let shim_path = temp_path.join("multi_digest");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    let output = Command::new(&shim_path)
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute multi_digest shim");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Exactly one digest algorithm must be specified"));
}

#[test]
fn test_e2e_appimage_unpack_and_execute() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Construct a fake Type 2 AppImage with an ELF64 header stub and SquashFS payload
    let appimage_path = temp_path.join("demotool.AppImage");
    test_helpers::create_fake_appimage(
        &appimage_path,
        &[
            (
                "usr/bin/demotool",
                b"#!/bin/sh\necho \"AppRun Executed: args=$*\"\n",
                0o755,
            ),
            (
                "usr/bin/subtool",
                b"#!/bin/sh\necho \"Custom Bin Executed: args=$*\"\n",
                0o755,
            ),
        ],
        &[("AppRun", "usr/bin/demotool")],
    );

    let sha256 = compute_sha256(&appimage_path);

    // 2. Create scrim.yaml configuring both default entrypoint (AppRun) and custom archive_path
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  demotool:
    url: "file://{}"
    sha256: "{}"
  subtool:
    template: demotool
    archive_path: "usr/bin/subtool"
"#,
        appimage_path.to_str().unwrap(),
        sha256
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    let shim_demotool = temp_path.join("demotool");
    let shim_subtool = temp_path.join("subtool");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&scrim_bin, &shim_demotool).unwrap();
        std::os::unix::fs::symlink(&scrim_bin, &shim_subtool).unwrap();
    }

    let cache_home = temp_path.join("fake_home");
    fs::create_dir_all(&cache_home).unwrap();

    // 3. Run demotool (should default to AppRun)
    let output = Command::new(&shim_demotool)
        .arg("hello")
        .arg("appimage")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to run demotool AppImage shim");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success(), "Shim execution failed! Stderr: {}", stderr);
    assert!(stdout.contains("AppRun Executed: args=hello appimage"));
    assert!(stderr.contains("[Scrim] Unpacking demotool archive..."));

    // 4. Run subtool (using template + explicit archive_path)
    let output_sub = Command::new(&shim_subtool)
        .arg("custom_arg")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .output()
        .expect("Failed to run subtool AppImage shim");

    let stdout_sub = String::from_utf8(output_sub.stdout).unwrap();
    let stderr_sub = String::from_utf8(output_sub.stderr).unwrap();

    assert!(output_sub.status.success(), "Subtool execution failed! Stderr: {}", stderr_sub);
    assert!(stdout_sub.contains("Custom Bin Executed: args=custom_arg"));
    assert!(
        !stderr_sub.contains("Unpacking"),
        "Should have hit the already unpacked cache directory! Stderr: {}",
        stderr_sub
    );
}

#[test]
fn test_e2e_config_imports_with_precedence_and_cli() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create base config
    let base_yaml = temp_path.join("base-tools.yaml");
    fs::write(
        &base_yaml,
        r#"
tools:
  node:
    system_path: /bin/base_node
  shared_util:
    system_path: /bin/shared_util
"#,
    )
    .unwrap();

    // 2. Create override config
    let overrides_dir = temp_path.join("overrides");
    fs::create_dir_all(&overrides_dir).unwrap();
    let override_yaml = overrides_dir.join("tools.yaml");
    fs::write(
        &override_yaml,
        r#"
tools:
  python:
    system_path: /bin/override_python
"#,
    )
    .unwrap();

    // 3. Create root scrim.yaml
    let root_scrim = temp_path.join("scrim.yaml");
    fs::write(
        &root_scrim,
        r#"
telemetry: true
imports:
  - base-tools.yaml
  - path: overrides/tools.yaml
    precedence: after
  - path: optional-missing.yaml
    optional: true
tools:
  node:
    system_path: /bin/root_node
  python:
    system_path: /bin/root_python
"#,
    )
    .unwrap();

    // Test scrim config CLI output
    let output = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute scrim config");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    
    // node was defined in root, overriding base-tools.yaml (precedence: before)
    assert!(stdout.contains("node:"));
    assert!(stdout.contains("system_path: /bin/root_node"));
    
    // shared_util was imported from base-tools.yaml
    assert!(stdout.contains("shared_util:"));
    assert!(stdout.contains("system_path: /bin/shared_util"));
    
    // python was overridden by overrides/tools.yaml (precedence: after)
    assert!(stdout.contains("python:"));
    assert!(stdout.contains("system_path: /bin/override_python"));

    // imports key should not appear in final aggregated scrim config output
    assert!(!stdout.contains("imports:"));

    // Also verify `scrim config <tool>` for imported and overridden tools
    let output_python = Command::new(&scrim_bin)
        .args(["config", "python"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute scrim config python");
    assert!(output_python.status.success());
    let stdout_python = String::from_utf8(output_python.stdout).unwrap();
    assert!(stdout_python.contains("system_path: /bin/override_python"));
}

#[test]
fn test_e2e_config_imports_circular_error() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let a_yaml = temp_path.join("a.yaml");
    let b_yaml = temp_path.join("b.yaml");
    let scrim_yaml = temp_path.join("scrim.yaml");

    fs::write(&a_yaml, "imports:\n  - b.yaml\n").unwrap();
    fs::write(&b_yaml, "imports:\n  - a.yaml\n").unwrap();
    fs::write(&scrim_yaml, "imports:\n  - a.yaml\n").unwrap();

    let output = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute scrim config");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Circular import detected"));
}

#[test]
fn test_e2e_system_path_fallback_to_url() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a mock tool source binary to be fetched
    let fetched_tool_src = temp_path.join("fetched_tool_src");
    fs::write(
        &fetched_tool_src,
        "#!/bin/sh\necho \"Fetched Tool Executed: args=$*\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fetched_tool_src).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fetched_tool_src, perms).unwrap();
    }

    let sha256 = compute_sha256(&fetched_tool_src);

    // 2. Create an existing system tool binary
    let existing_system_tool = temp_path.join("existing_system_tool");
    fs::write(
        &existing_system_tool,
        "#!/bin/sh\necho \"System Tool Executed: args=$*\"\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&existing_system_tool).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&existing_system_tool, perms).unwrap();
    }

    let nonexistent_system_tool = temp_path.join("nonexistent_system_tool");

    // 3. Configure tools in scrim.yaml:
    //    - tool1: system_path exists -> prefers system_path
    //    - tool2: system_path does not exist -> falls back to url
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
tools:
  tool_exists:
    system_path: "{}"
    url: "file://{}"
    sha256: "{}"
  tool_fallback:
    system_path: "{}"
    url: "file://{}"
    sha256: "{}"
"#,
        existing_system_tool.to_str().unwrap(),
        fetched_tool_src.to_str().unwrap(),
        sha256,
        nonexistent_system_tool.to_str().unwrap(),
        fetched_tool_src.to_str().unwrap(),
        sha256
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    // Set isolated home for cache
    let fake_home = temp_path.join("fake_home");
    fs::create_dir_all(&fake_home).unwrap();

    // 4. Test tool_exists via `scrim run` -> executes system tool
    let output1 = Command::new(&scrim_bin)
        .args(["run", "tool_exists", "hello"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .expect("Failed to run tool_exists");
    assert!(output1.status.success());
    let stdout1 = String::from_utf8(output1.stdout).unwrap();
    assert!(stdout1.contains("System Tool Executed: args=hello"));

    // 5. Test tool_fallback via `scrim run` -> falls back to fetched tool
    let output2 = Command::new(&scrim_bin)
        .args(["run", "tool_fallback", "world"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .expect("Failed to run tool_fallback");
    assert!(output2.status.success());
    let stdout2 = String::from_utf8(output2.stdout).unwrap();
    assert!(stdout2.contains("Fetched Tool Executed: args=world"));
}




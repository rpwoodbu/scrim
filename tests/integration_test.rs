use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;
use test_helpers::{copy_executable, find_scrim_bin, write_executable};

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
    write_executable(
        &mock_tool_src,
        "#!/bin/sh\necho \"Mock Tool Executed: args=$*\"\n",
    );

    let sha256 = compute_sha256(&mock_tool_src);

    // 2. Create scrim.yaml in the temp directory
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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

    // Override HOME, XDG_CONFIG_HOME, and XDG_STATE_HOME to isolate config, cache, and telemetry
    fs::create_dir_all(temp_path.join(".git")).unwrap();
    let cache_home = temp_path.join("fake_home");
    let config_home = temp_path.join("xdg_config");
    let state_home = temp_path.join("fake_state");
    fs::create_dir_all(&cache_home).unwrap();
    fs::create_dir_all(&state_home).unwrap();
    let log_path = state_home.join("scrim/telemetry.log");

    // 4. Execute the symlink shim
    let output = Command::new(&shim_path)
        .arg("hello")
        .arg("world")
        .current_dir(temp_path)
        .env("HOME", &cache_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env_remove("XDG_CACHE_HOME")
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
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env_remove("XDG_CACHE_HOME")
        .output()
        .expect("Failed to execute demotool shim on cache hit");

    let stdout_cached = String::from_utf8(output_cached.stdout).unwrap();
    assert!(output_cached.status.success());
    assert!(stdout_cached.contains("Mock Tool Executed: args=second run"));

    // Verify telemetry did not write any log entries (since telemetry: false is set)
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        !log_path.exists(),
        "Telemetry log should not be created when telemetry: false is set"
    );
}

#[test]
fn test_e2e_local_path_resolution() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a local mock tool
    let local_tool = temp_path.join("my_local_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"Local tool run with: $*\"\n",);

    // 2. Create scrim.yaml pointing to the local tool
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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
    write_executable(&local_tool, "#!/bin/sh\necho \"Telemetry test run\"\n",);

    // 2. Create scrim.yaml with telemetry enabled
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: true
default_platform: linux-x86_64

tools:
  node:
    system_path: "{}"
"#,
        local_tool.to_str().unwrap()
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    let state_home = temp_path.join("state");
    let config_home = temp_path.join("xdg_config");
    fs::create_dir_all(temp_path.join(".git")).unwrap();
    let log_path = state_home.join("scrim/telemetry.log");

    // 3. Create symlink node -> scrim
    let shim_path = temp_path.join("node");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // 4. Run the shim
    let output = Command::new(&shim_path)
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .expect("Failed to run local path shim");

    assert!(output.status.success());

    // 5. Poll the log file up to a 5-second timeout
    let log_content = poll_log_contains(&log_path, "\"tool\": \"node\"");
    assert!(log_content.contains(&format!("\"target\": \"{}\"", local_tool.display())));
}

#[test]
fn test_e2e_telemetry_disabled_by_default() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a local mock tool
    let local_tool = temp_path.join("my_local_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"Telemetry test run (disabled by default)\"\n",);

    // 2. Create scrim.yaml WITHOUT specifying telemetry (should default to disabled)
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
default_platform: linux-x86_64

tools:
  node:
    system_path: "{}"
"#,
        local_tool.to_str().unwrap()
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    let state_home = temp_path.join("state");
    let config_home = temp_path.join("xdg_config");
    fs::create_dir_all(temp_path.join(".git")).unwrap();
    let log_path = state_home.join("scrim/telemetry.log");

    // 3. Create symlink node -> scrim
    let shim_path = temp_path.join("node");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // 4. Run the shim
    let output = Command::new(&shim_path)
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .expect("Failed to run local path shim");

    assert!(output.status.success());

    // 5. Verify telemetry did not write any log entries (since no sinks are configured)
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        !log_path.exists(),
        "Telemetry log should not exist when no sinks are configured"
    );
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
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
    write_executable(&mock_tool, "#!/bin/sh\necho \"Archived Tool\"\n");

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
default_platform: linux-x86_64

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
    write_executable(&mock_tool, "#!/bin/sh\necho \"Archived XZ Tool\"\n");

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
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
    assert!(stdout_help_flag.contains("XDG_STATE_HOME"));

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
    fs::create_dir_all(temp_path.join(".git")).unwrap();
    let config_home = temp_path.join("xdg_config");
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = r#"telemetry: false
default_platform: linux-x86_64

tools:
  demotool:
    system_path: /bin/echo"#;
    std::fs::write(&scrim_yaml, config_content).unwrap();

    let output = Command::new(&scrim_bin)
        .arg("config")
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim config");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("telemetry"), "Disabled telemetry should be omitted, got:\n{}", stdout);
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
default_platform: linux-x86_64

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
    write_executable(&local_tool, "#!/bin/sh\necho \"Tool run: $*\"\n",);

    // Create scrim.yaml defining tool "tool1"
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
default_platform: linux-x86_64

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

    // 4. Run `scrim links <target_bin>` without --prune: tool1 is linked, obsolete_tool is left intact
    let output_links = Command::new(&scrim_bin)
        .arg("links")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links <target_bin>");
    let stderr_links = String::from_utf8(output_links.stderr).unwrap();

    assert!(output_links.status.success(), "scrim links command failed: {}", stderr_links);
    assert!(stderr_links.contains("[Scrim] Linked tool1"));
    assert!(!stderr_links.contains("[Scrim] Removed obsolete link"));

    // Verify tool1 link was created and works, and stale_link is still present
    let tool1_link = target_bin.join("tool1");
    assert!(tool1_link.exists());
    assert!(stale_link.exists());
    assert!(unrelated_file.exists());

    // 4b. Run `scrim links --prune <target_bin>`: removes obsolete_tool
    let output_prune = Command::new(&scrim_bin)
        .arg("links")
        .arg("--prune")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links --prune <target_bin>");
    let stderr_prune = String::from_utf8(output_prune.stderr).unwrap();
    assert!(output_prune.status.success(), "scrim links --prune failed: {}", stderr_prune);
    assert!(stderr_prune.contains("[Scrim] Removed obsolete link obsolete_tool"));
    assert!(!stale_link.exists());

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
default_platform: linux-x86_64

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
    write_executable(&local_tool, "#!/bin/sh\necho \"Tool run: $*\"\n");

    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
default_platform: linux-x86_64

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

    // 4. Run `scrim links -n <target_bin>` (no prune): changes would be made (link tool1) -> exit code 2
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
    assert!(!stderr_dry_run_short.contains("[Scrim] Would remove obsolete link"));

    // Verify filesystem was NOT modified
    let tool1_link = target_bin.join("tool1");
    assert!(!tool1_link.exists());
    assert!(stale_link.exists());

    // 5. Test with `--dry-run --prune` flags: both link and remove are reported -> exit code 2
    let output_dry_run_prune = Command::new(&scrim_bin)
        .arg("links")
        .arg("--dry-run")
        .arg("--prune")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links --dry-run --prune <target_bin>");
    assert_eq!(output_dry_run_prune.status.code(), Some(2));
    let stderr_dry_run_prune = String::from_utf8(output_dry_run_prune.stderr).unwrap();
    assert!(stderr_dry_run_prune.contains("[Scrim] Would link tool1"));
    assert!(stderr_dry_run_prune.contains("[Scrim] Would remove obsolete link obsolete_tool"));

    // 6. Run actual `scrim links <target_bin>` (without prune)
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
    assert!(stale_link.exists()); // stale link remains without prune

    // 7. Run `scrim links -n <target_bin>` without prune: all configured links are correct -> exit code 0
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

    // 7b. Run `scrim links -n -p <target_bin>`: obsolete link exists and prune requested -> exit code 2
    let output_dry_prune_needed = Command::new(&scrim_bin)
        .arg("links")
        .arg("-n")
        .arg("-p")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links -n -p <target_bin>");
    assert_eq!(output_dry_prune_needed.status.code(), Some(2));
    let stderr_dry_prune_needed = String::from_utf8(output_dry_prune_needed.stderr).unwrap();
    assert!(stderr_dry_prune_needed.contains("[Scrim] Would remove obsolete link obsolete_tool"));

    // 7c. Run actual `scrim links -p <target_bin>`: removes obsolete_tool
    let output_actual_prune = Command::new(&scrim_bin)
        .arg("links")
        .arg("-p")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run actual scrim links -p");
    assert_eq!(output_actual_prune.status.code(), Some(0));
    assert!(!stale_link.exists());

    // 7d. Run `scrim links -n -p <target_bin>` now that all links are correct and pruned -> exit code 0
    let output_dry_prune_clean = Command::new(&scrim_bin)
        .arg("links")
        .arg("-n")
        .arg("-p")
        .arg(&target_bin)
        .current_dir(temp_path)
        .output()
        .expect("Failed to run scrim links -n -p after prune");
    assert_eq!(output_dry_prune_clean.status.code(), Some(0));
    let stderr_dry_prune_clean = String::from_utf8(output_dry_prune_clean.stderr).unwrap();
    assert!(stderr_dry_prune_clean.contains("[Scrim] All links are correct"));

    // 8. Run `scrim links -n <target_bin>` with conflict -> exit code 1
    let config_conflicts = format!(
        r#"
default_platform: linux-x86_64

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
    write_executable(&mock_tool, "#!/bin/sh\necho \"Executed via run: args=$*\"\n",);

    // Create scrim.yaml defining 'mytool'
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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
    write_executable(&fetch_src, "#!/bin/sh\necho \"Fetched via run: args=$*\"\n");
    let fetch_sha256 = compute_sha256(&fetch_src);

    let config_fetch = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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
    write_executable(&v1_src, "#!/bin/sh\necho \"Tool Version 1: args=$*\"\n");
    let v1_sha256 = compute_sha256(&v1_src);
    let v1_url = format!("file://{}", v1_src.to_str().unwrap());

    // 2. Configure scrim.yaml with v1
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_v1 = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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
    write_executable(&v2_src, "#!/bin/sh\necho \"Tool Version 2: args=$*\"\n");
    let v2_sha256 = compute_sha256(&v2_src);
    let v2_url = format!("file://{}", v2_src.to_str().unwrap());

    // 6. User updates URL in scrim.yaml to v2, but does NOT update sha256 (still has v1_sha256)
    let config_v2_stale_hash = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
    assert!(stdout_all.contains("telemetry:"));
    assert!(stdout_all.contains("file:"));
    assert!(stdout_all.contains("- {}"));
    assert!(!stdout_all.contains("telemetry: true"));
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
    write_executable(&tool_src, "#!/bin/sh\necho \"Tool Ran: args=$*\"\n");
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
            "default_platform: linux-x86_64\ntelemetry: false\ntools:\n  mytool:\n    url: \"{}\"\n    sha256: \"{}\"\n",
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
    write_executable(&tool_src, "#!/bin/sh\necho \"Tool Ran: args=$*\"\n");
    let sha256 = compute_sha256(&tool_src);
    let tool_url = format!("file://{}", tool_src.to_str().unwrap());

    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        format!(
            "default_platform: linux-x86_64\ntelemetry: false\ntools:\n  mytool:\n    url: \"{}\"\n    sha256: \"{}\"\n",
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
    write_executable(
        &mock_tool_src,
        "#!/bin/sh\necho \"Sha512 Tool Executed: args=$*\"\n",
    );

    let sha512 = compute_sha512(&mock_tool_src);

    // 2. Create scrim.yaml in the temp directory specifying sha512
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
default_platform: linux-x86_64

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
    write_executable(
        &fetched_tool_src,
        "#!/bin/sh\necho \"Fetched Tool Executed: args=$*\"\n",
    );

    let sha256 = compute_sha256(&fetched_tool_src);

    // 2. Create an existing system tool binary
    let existing_system_tool = temp_path.join("existing_system_tool");
    write_executable(
        &existing_system_tool,
        "#!/bin/sh\necho \"System Tool Executed: args=$*\"\n",
    );

    let nonexistent_system_tool = temp_path.join("nonexistent_system_tool");

    // 3. Configure tools in scrim.yaml:
    //    - tool1: system_path exists -> prefers system_path
    //    - tool2: system_path does not exist -> falls back to url
    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

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
    assert!(
        output1.status.success(),
        "output1 failed: status={:?}, stderr={}, stdout={}",
        output1.status,
        String::from_utf8_lossy(&output1.stderr),
        String::from_utf8_lossy(&output1.stdout)
    );
    let stdout1 = String::from_utf8(output1.stdout).unwrap();
    assert!(stdout1.contains("System Tool Executed: args=hello"));

    // 5. Test tool_fallback via `scrim run` -> falls back to fetched tool
    let output2 = Command::new(&scrim_bin)
        .args(["run", "tool_fallback", "world"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .expect("Failed to run tool_fallback");
    assert!(
        output2.status.success(),
        "output2 failed: status={:?}, stderr={}, stdout={}",
        output2.status,
        String::from_utf8_lossy(&output2.stderr),
        String::from_utf8_lossy(&output2.stdout)
    );
    let stdout2 = String::from_utf8(output2.stdout).unwrap();
    assert!(stdout2.contains("Fetched Tool Executed: args=world"));
}

#[test]
fn test_scrim_which_command() {
    let scrim_bin = find_scrim_bin();
    let temp_dir = tempdir().unwrap();
    let temp_path = temp_dir.path();

    let fake_home = temp_path.join("home");
    fs::create_dir_all(&fake_home).unwrap();

    let existing_system_tool = temp_path.join("existing_tool");
    fs::write(&existing_system_tool, "#!/bin/sh\n").unwrap();

    let nonexistent_system_tool = temp_path.join("nonexistent_tool");

    let scrim_yaml = temp_path.join("scrim.yaml");
    let config_content = format!(
        r#"
telemetry: false
default_platform: linux-x86_64

tools:
  sys_tool:
    system_path: "{}"
  sys_with_fallback_exists:
    system_path: "{}"
    url: "https://example.com/tool.tar.gz"
    sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    archive_path: "bin/tool"
  sys_with_fallback_missing:
    system_path: "{}"
    url: "https://example.com/tool.tar.gz"
    sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    archive_path: "bin/tool"
  fetch_tar_gz:
    url: "https://example.com/archive.tar.gz"
    sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
    archive_path: "bin/archive_bin"
  fetch_default_archive:
    url: "https://example.com/archive.tar.gz"
    sha256: "1111111111111111111111111111111111111111111111111111111111111111"
  fetch_appimage:
    url: "https://example.com/app.AppImage"
    sha256: "2222222222222222222222222222222222222222222222222222222222222222"
  fetch_binary:
    url: "https://example.com/single_bin"
    sha256: "3333333333333333333333333333333333333333333333333333333333333333"
  template_base:
    url: "https://example.com/base.tar.gz"
    sha256: "4444444444444444444444444444444444444444444444444444444444444444"
    archive_path: "bin/base"
  templated_tool:
    template: template_base
    archive_path: "bin/sub_tool"
"#,
        existing_system_tool.to_str().unwrap(),
        existing_system_tool.to_str().unwrap(),
        nonexistent_system_tool.to_str().unwrap(),
    );
    fs::write(&scrim_yaml, config_content).unwrap();

    let expected_cache_dir = fake_home.join(".cache/scrim/tools");

    // 1. sys_tool
    let out = Command::new(&scrim_bin)
        .args(["which", "sys_tool"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), existing_system_tool.to_str().unwrap());

    // 2. sys_with_fallback_exists -> points to system_path
    let out = Command::new(&scrim_bin)
        .args(["which", "sys_with_fallback_exists"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), existing_system_tool.to_str().unwrap());

    // 3. sys_with_fallback_missing -> points to cache binary path
    let out = Command::new(&scrim_bin)
        .args(["which", "sys_with_fallback_missing"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    let expected_path = expected_cache_dir
        .join("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        .join("bin/tool");
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), expected_path.to_str().unwrap());
    // Also verify that it did not actually create/fetch the file
    assert!(!expected_path.exists());

    // 4. fetch_tar_gz
    let out = Command::new(&scrim_bin)
        .args(["which", "fetch_tar_gz"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    let expected_path = expected_cache_dir
        .join("sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        .join("bin/archive_bin");
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), expected_path.to_str().unwrap());

    // 5. fetch_default_archive
    let out = Command::new(&scrim_bin)
        .args(["which", "fetch_default_archive"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    let expected_path = expected_cache_dir
        .join("sha256:1111111111111111111111111111111111111111111111111111111111111111")
        .join("fetch_default_archive");
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), expected_path.to_str().unwrap());

    // 6. fetch_appimage -> default AppRun
    let out = Command::new(&scrim_bin)
        .args(["which", "fetch_appimage"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    let expected_path = expected_cache_dir
        .join("sha256:2222222222222222222222222222222222222222222222222222222222222222")
        .join("AppRun");
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), expected_path.to_str().unwrap());

    // 7. fetch_binary -> non-archive tool
    let out = Command::new(&scrim_bin)
        .args(["which", "fetch_binary"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    let expected_path = expected_cache_dir
        .join("sha256:3333333333333333333333333333333333333333333333333333333333333333")
        .join("fetch_binary");
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), expected_path.to_str().unwrap());

    // 8. templated_tool
    let out = Command::new(&scrim_bin)
        .args(["which", "templated_tool"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    let expected_path = expected_cache_dir
        .join("sha256:4444444444444444444444444444444444444444444444444444444444444444")
        .join("bin/sub_tool");
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), expected_path.to_str().unwrap());

    // 9. Missing tool argument
    let out = Command::new(&scrim_bin)
        .args(["which"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: tool name argument required for which command"));

    // 10. Unknown tool
    let out = Command::new(&scrim_bin)
        .args(["which", "nonexistent"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Tool 'nonexistent' not found in config"));

    // 11. Relative system_path rejected with actionable error
    let rel_config = temp_path.join("rel_config");
    fs::create_dir_all(&rel_config).unwrap();
    fs::write(
        rel_config.join("scrim.yaml"),
        "default_platform: linux-x86_64\ntools:\n  rel_tool:\n    system_path: ./rel/path\n",
    ).unwrap();
    let out = Command::new(&scrim_bin)
        .args(["which", "rel_tool"])
        .current_dir(&rel_config)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Tool 'rel_tool' specifies relative system_path './rel/path'. 'system_path' must be an absolute path."));

    // 12. Extra arguments rejected
    let out = Command::new(&scrim_bin)
        .args(["which", "sys_tool", "extra"])
        .current_dir(temp_path)
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: unexpected argument 'extra'"));
}

#[test]
fn test_platform_cli_config_and_which() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        r#"
telemetry: false
tools:
  node:
    platforms:
      linux-x86_64:
        system_path: /bin/linux_node
      macos-aarch64:
        system_path: /opt/homebrew/bin/mac_node
  only_mac:
    platforms:
      macos-aarch64:
        system_path: /opt/homebrew/bin/only_mac
  only_linux:
    platforms:
      linux-x86_64:
        system_path: /bin/only_linux
  universal:
    platforms:
      any:
        system_path: /bin/universal
"#,
    )
    .unwrap();

    // 1. scrim config --platform macos-aarch64
    let out = Command::new(&scrim_bin)
        .args(["config", "--platform", "macos-aarch64"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("node:"));
    assert!(stdout.contains("/opt/homebrew/bin/mac_node"));
    assert!(!stdout.contains("/bin/linux_node"));
    assert!(stdout.contains("only_mac:"));
    assert!(stdout.contains("universal:"));
    assert!(!stdout.contains("only_linux:"));

    // 2. scrim config -p macos-aarch64 only_mac
    let out = Command::new(&scrim_bin)
        .args(["config", "-p", "macos-aarch64", "only_mac"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("system_path: /opt/homebrew/bin/only_mac"));

    // 3. scrim config --platform macos-aarch64 only_linux (unconfigured tool for target platform)
    let out = Command::new(&scrim_bin)
        .args(["config", "--platform", "macos-aarch64", "only_linux"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Tool 'only_linux' is not configured for platform 'macos-aarch64'"));

    // 4. scrim config --platform linux_x86_64 (invalid delimiter error)
    let out = Command::new(&scrim_bin)
        .args(["config", "--platform", "linux_x86_64"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Invalid platform string 'linux_x86_64'"));

    // 5. scrim which --platform macos-aarch64 node
    let out = Command::new(&scrim_bin)
        .args(["which", "--platform", "macos-aarch64", "node"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.trim(), "/opt/homebrew/bin/mac_node");

    // 6. scrim which -p macos-aarch64 only_mac
    let out = Command::new(&scrim_bin)
        .args(["which", "-p", "macos-aarch64", "only_mac"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.trim(), "/opt/homebrew/bin/only_mac");

    // 7. scrim which --platform macos-aarch64 only_linux (unconfigured tool error)
    let out = Command::new(&scrim_bin)
        .args(["which", "--platform", "macos-aarch64", "only_linux"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Tool 'only_linux' is not configured for platform 'macos-aarch64'"));

    // 8. scrim which --platform darwin-arm64 node (synonym actionable error)
    let out = Command::new(&scrim_bin)
        .args(["which", "--platform", "darwin-arm64", "node"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("darwin"));
}

#[test]
fn test_platform_links_host_filtering_and_cleanup() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let bin_dir = temp_path.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let host_os = std::env::consts::OS;
    let non_host_os = if host_os == "linux" { "macos" } else { "linux" };
    let non_host_platform = format!("{}-aarch64", non_host_os);

    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        format!(
            r#"
telemetry: false
tools:
  host_tool:
    platforms:
      any:
        system_path: /bin/echo
  other_tool:
    platforms:
      {}:
        system_path: /bin/other
"#,
            non_host_platform
        ),
    )
    .unwrap();

    // Pre-create an obsolete link for other_tool pointing to scrim
    let obsolete_link = bin_dir.join("other_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &obsolete_link).unwrap();
    assert!(obsolete_link.exists());

    // 1. Dry run with --prune
    let out_dry = Command::new(&scrim_bin)
        .args(["links", "-n", "-p", "bin"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out_dry.status.code(), Some(2));
    let stderr_dry = String::from_utf8(out_dry.stderr).unwrap();
    assert!(stderr_dry.contains("[Scrim] Would link host_tool"));
    assert!(stderr_dry.contains("[Scrim] Would remove obsolete link other_tool"));
    assert!(!bin_dir.join("host_tool").exists());
    assert!(obsolete_link.exists());

    // 2. Normal run without --prune: links host_tool, does not prune other_tool
    let out_no_prune = Command::new(&scrim_bin)
        .args(["links", "bin"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert!(out_no_prune.status.success());
    let stderr_out_no_prune = String::from_utf8(out_no_prune.stderr).unwrap();
    assert!(stderr_out_no_prune.contains("[Scrim] Linked host_tool"));
    assert!(!stderr_out_no_prune.contains("[Scrim] Removed obsolete link"));
    assert!(bin_dir.join("host_tool").exists());
    assert!(obsolete_link.exists());

    // 3. Normal run with --prune: removes other_tool
    let out = Command::new(&scrim_bin)
        .args(["links", "-p", "bin"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr_out = String::from_utf8(out.stderr).unwrap();
    assert!(stderr_out.contains("[Scrim] Removed obsolete link other_tool"));
    assert!(bin_dir.join("host_tool").exists());
    assert!(!bin_dir.join("other_tool").exists());

    // 4. Second run with --prune should be idempotent
    let out_clean = Command::new(&scrim_bin)
        .args(["links", "-p", "bin"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert!(out_clean.status.success());
    let stderr_clean = String::from_utf8(out_clean.stderr).unwrap();
    assert!(stderr_clean.contains("[Scrim] All links are correct"));
}

#[test]
fn test_platform_run_and_proxy_unconfigured_tool_error() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let host_os = std::env::consts::OS;
    let non_host_os = if host_os == "linux" { "macos" } else { "linux" };
    let non_host_platform = format!("{}-aarch64", non_host_os);

    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        format!(
            r#"
telemetry: false
tools:
  non_host_tool:
    platforms:
      {}:
        system_path: /bin/echo
"#,
            non_host_platform
        ),
    )
    .unwrap();

    // 1. scrim run non_host_tool
    let out = Command::new(&scrim_bin)
        .args(["run", "non_host_tool"])
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Tool 'non_host_tool' is not configured for platform '"));

    // 2. Proxy execution via symlink non_host_tool -> scrim
    let proxy_shim = temp_path.join("non_host_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &proxy_shim).unwrap();

    let out_proxy = Command::new(&proxy_shim)
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out_proxy.status.code(), Some(1));
    let stderr_proxy = String::from_utf8(out_proxy.stderr).unwrap();
    assert!(stderr_proxy.contains("[Scrim] Error: Tool 'non_host_tool' is not configured for platform '"));
}

#[test]
fn test_proxy_path_fallback_success() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let shim_dir = temp_path.join("shims");
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&shim_dir).unwrap();
    fs::create_dir_all(&sys_dir).unwrap();

    // Real tool in sys_dir
    let sys_tool = sys_dir.join("fallback_tool");
    write_executable(
        &sys_tool,
        "#!/bin/sh\necho \"Fallback Executed: args=$*\"\n",
    );

    // Symlink in shim_dir pointing to scrim
    let shim_path = shim_dir.join("fallback_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // scrim.yaml in workspace has no definition for fallback_tool
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        r#"
default_platform: any
telemetry: false
tools:
  other_tool:
    system_path: /bin/echo
"#,
    )
    .unwrap();

    let path_env = format!("{}:{}", shim_dir.display(), sys_dir.display());
    let out = Command::new(&shim_path)
        .arg("hello")
        .arg("world")
        .env("PATH", &path_env)
        .current_dir(temp_path)
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.trim(), "Fallback Executed: args=hello world");
}

#[test]
fn test_proxy_path_fallback_hardlink() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let shim_dir = temp_path.join("shims");
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&shim_dir).unwrap();
    fs::create_dir_all(&sys_dir).unwrap();

    let sys_tool = sys_dir.join("hardlink_tool");
    write_executable(
        &sys_tool,
        "#!/bin/sh\necho \"Hardlink Fallback: args=$*\"\n",
    );

    // Hardlink in shim_dir pointing to scrim_bin. If cross-device linking (EXDEV)
    // fails (e.g. temp directory is on a different mount than scrim_bin), copy
    // scrim_bin into temp_path first in a child process before creating the hardlink.
    let shim_path = shim_dir.join("hardlink_tool");
    #[cfg(unix)]
    if fs::hard_link(&scrim_bin, &shim_path).is_err() {
        let scrim_copy = temp_path.join("scrim_copy");
        copy_executable(&scrim_bin, &scrim_copy);
        fs::hard_link(&scrim_copy, &shim_path).unwrap();
    }

    let path_env = format!("{}:{}", shim_dir.display(), sys_dir.display());
    let out = Command::new(&shim_path)
        .arg("arg1")
        .env("PATH", &path_env)
        .current_dir(temp_path)
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.trim(), "Hardlink Fallback: args=arg1");
}

#[test]
fn test_proxy_path_fallback_command_not_found_exit_127() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let shim_dir = temp_path.join("shims");
    fs::create_dir_all(&shim_dir).unwrap();

    let shim_path = shim_dir.join("missing_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // PATH only contains shim_dir, so missing_tool is not found anywhere else
    let out = Command::new(&shim_path)
        .env("PATH", &shim_dir)
        .current_dir(temp_path)
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(127));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: missing_tool: command not found"));
}

#[test]
fn test_proxy_path_fallback_permission_denied_exit_126() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let shim_dir = temp_path.join("shims");
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&shim_dir).unwrap();
    fs::create_dir_all(&sys_dir).unwrap();

    // Create an unexecutable interpreter file
    let noexec_interpreter = sys_dir.join("noexec_interpreter");
    fs::write(&noexec_interpreter, "binary").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&noexec_interpreter).unwrap().permissions();
        perms.set_mode(0o644); // Not executable
        fs::set_permissions(&noexec_interpreter, perms).unwrap();
    }

    // Script with shebang pointing to the unexecutable interpreter
    let sys_tool = sys_dir.join("perm_denied_tool");
    write_executable(
        &sys_tool,
        format!("#!{}\necho hi\n", noexec_interpreter.display()),
    );

    let shim_path = shim_dir.join("perm_denied_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    let path_env = format!("{}:{}", shim_dir.display(), sys_dir.display());
    let out = Command::new(&shim_path)
        .env("PATH", &path_env)
        .current_dir(temp_path)
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(126));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Failed to execute tool:"));
}

#[test]
fn test_proxy_path_fallback_telemetry() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let shim_dir = temp_path.join("shims");
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&shim_dir).unwrap();
    fs::create_dir_all(&sys_dir).unwrap();

    let sys_tool = sys_dir.join("telemetry_tool");
    write_executable(
        &sys_tool,
        "#!/bin/sh\necho \"Telemetry test\"\n",
    );

    let shim_path = shim_dir.join("telemetry_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // scrim.yaml with telemetry: true
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        r#"
default_platform: any
telemetry: true
tools:
  other_tool:
    system_path: /bin/echo
"#,
    )
    .unwrap();

    let state_home = temp_path.join("state");
    let config_home = temp_path.join("xdg_config");
    fs::create_dir_all(temp_path.join(".git")).unwrap();
    let telemetry_log = state_home.join("scrim/telemetry.log");

    let path_env = format!("{}:{}", shim_dir.display(), sys_dir.display());
    let out = Command::new(&shim_path)
        .env("PATH", &path_env)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .current_dir(temp_path)
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let content = poll_log_contains(&telemetry_log, "\"tool\": \"telemetry_tool\"");
    assert!(content.contains(&format!("\"target\": \"{}\"", sys_tool.display())));
}

#[test]
fn test_scrim_run_and_which_do_not_fallback_to_path() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&sys_dir).unwrap();

    let sys_tool = sys_dir.join("only_in_path");
    write_executable(&sys_tool, "#!/bin/sh\necho sys tool\n");

    // scrim run only_in_path must fail with status 1
    let out_run = Command::new(&scrim_bin)
        .args(["run", "only_in_path"])
        .env("PATH", &sys_dir)
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out_run.status.code(), Some(1));
    let stderr_run = String::from_utf8(out_run.stderr).unwrap();
    assert!(stderr_run.contains("[Scrim] Error: Tool 'only_in_path' not found in config"));

    // scrim which only_in_path must fail with status 1
    let out_which = Command::new(&scrim_bin)
        .args(["which", "only_in_path"])
        .env("PATH", &sys_dir)
        .current_dir(temp_path)
        .output()
        .unwrap();
    assert_eq!(out_which.status.code(), Some(1));
    let stderr_which = String::from_utf8(out_which.stderr).unwrap();
    assert!(stderr_which.contains("[Scrim] Error: Tool 'only_in_path' not found in config"));
}

#[test]
fn test_syntactic_parse_error_tolerance_in_proxy_invocation() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Create a broken scrim.yaml in the workspace (bad YAML syntax)
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(&scrim_yaml, ": invalid yaml : [unterminated").unwrap();

    // 2. Put a mock node binary in a "sys" directory on PATH
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&sys_dir).unwrap();
    let sys_node = sys_dir.join("node");
    write_executable(&sys_node, "#!/bin/sh\necho external node\n");

    // 3. Create proxy shim node -> scrim
    let shim_path = temp_path.join("node");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    // 4. Run the shim in the workspace: Scrim should warn on the syntax error,
    // ignore the broken file, and fall back to the external node in PATH!
    let output = Command::new(&shim_path)
        .current_dir(temp_path)
        .env("PATH", &sys_dir)
        .output()
        .expect("Failed to execute node shim");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), "external node");

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Warning: Failed to parse configuration file"));
}

#[test]
fn test_semantic_error_isolation_proxy_invocation() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Script for good_tool
    let good_script = temp_path.join("good.sh");
    write_executable(&good_script, "#!/bin/sh\necho good tool executed\n");

    // 2. Mock external binary for bad_tool on PATH to verify no accidental fallback
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&sys_dir).unwrap();
    let sys_bad = sys_dir.join("bad_tool");
    write_executable(&sys_bad, "#!/bin/sh\necho bad external\n");

    // 3. scrim.yaml with good_tool (valid) and bad_tool (semantic error: url without digest)
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        format!(
            r#"
tools:
  good_tool:
    platforms:
      linux-x86_64:
        system_path: {}
  bad_tool:
    platforms:
      linux-x86_64:
        url: https://example.com/bad.tar.gz
"#,
            good_script.display()
        ),
    )
    .unwrap();

    // 4. Create shims for both tools
    let good_shim = temp_path.join("good_tool");
    let bad_shim = temp_path.join("bad_tool");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&scrim_bin, &good_shim).unwrap();
        std::os::unix::fs::symlink(&scrim_bin, &bad_shim).unwrap();
    }

    // 5. Invoking good_tool shim MUST succeed despite bad_tool's defect
    let output_good = Command::new(&good_shim)
        .current_dir(temp_path)
        .env("PATH", &sys_dir)
        .output()
        .expect("Failed to execute good_tool shim");
    assert!(output_good.status.success());
    let stdout_good = String::from_utf8(output_good.stdout).unwrap();
    assert_eq!(stdout_good.trim(), "good tool executed");

    // 6. Invoking bad_tool shim MUST fail fast with status 1 and actionable error (no fallback to PATH!)
    let output_bad = Command::new(&bad_shim)
        .current_dir(temp_path)
        .env("PATH", &sys_dir)
        .output()
        .expect("Failed to execute bad_tool shim");
    assert_eq!(output_bad.status.code(), Some(1));
    let stderr_bad = String::from_utf8(output_bad.stderr).unwrap();
    assert!(stderr_bad.contains("[Scrim] Error: Tool 'bad_tool' specifies 'url' without a digest"));
}

#[test]
fn test_scrim_run_allow_fallback_to_lower_layer() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Lower layer (parent) defines node validly
    let parent_script = temp_path.join("parent_node.sh");
    write_executable(&parent_script, "#!/bin/sh\necho parent node\n");
    let parent_config = temp_path.join("scrim.yaml");
    fs::write(
        &parent_config,
        format!(
            r#"
tools:
  node:
    platforms:
      linux-x86_64:
        system_path: {}
"#,
            parent_script.display()
        ),
    )
    .unwrap();

    // 2. Child directory defines node with a semantic defect (url without digest)
    let child_dir = temp_path.join("child");
    fs::create_dir_all(&child_dir).unwrap();
    let child_config = child_dir.join("scrim.yaml");
    fs::write(
        &child_config,
        r#"
tools:
  node:
    platforms:
      linux-x86_64:
        url: https://example.com/broken_node.tar.gz
"#,
    )
    .unwrap();

    // 3. Normal 'scrim run node' fails fast without fallback
    let output_normal = Command::new(&scrim_bin)
        .current_dir(&child_dir)
        .args(["run", "node"])
        .output()
        .expect("Failed to run scrim run node");
    assert_eq!(output_normal.status.code(), Some(1));
    let stderr_normal = String::from_utf8(output_normal.stderr).unwrap();
    assert!(stderr_normal.contains("[Scrim] Error: Tool 'node' specifies 'url' without a digest"));

    // 4. 'scrim run --allow-fallback node' warns on stderr and executes parent layer's node!
    let output_fallback = Command::new(&scrim_bin)
        .current_dir(&child_dir)
        .args(["run", "--allow-fallback", "node"])
        .output()
        .expect("Failed to run scrim run --allow-fallback node");
    assert!(output_fallback.status.success());
    let stdout_fallback = String::from_utf8(output_fallback.stdout).unwrap();
    assert_eq!(stdout_fallback.trim(), "parent node");
    let stderr_fallback = String::from_utf8(output_fallback.stderr).unwrap();
    assert!(stderr_fallback.contains("[Scrim] Warning: Tool 'node' has an invalid configuration"));
}

#[test]
fn test_scrim_run_allow_fallback_to_path() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. External binary on PATH
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&sys_dir).unwrap();
    let sys_node = sys_dir.join("node");
    write_executable(&sys_node, "#!/bin/sh\necho path node\n");

    // 2. Workspace config defines node with semantic error
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        r#"
tools:
  node:
    platforms:
      linux-x86_64:
        url: https://example.com/broken_node.tar.gz
"#,
    )
    .unwrap();

    // 3. Normal 'scrim run node' fails fast without fallback to PATH
    let output_normal = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .env("PATH", &sys_dir)
        .args(["run", "node"])
        .output()
        .expect("Failed to run scrim run node");
    assert_eq!(output_normal.status.code(), Some(1));
    let stderr_normal = String::from_utf8(output_normal.stderr).unwrap();
    assert!(stderr_normal.contains("[Scrim] Error: Tool 'node' specifies 'url' without a digest"));

    // 4. 'scrim run --allow-fallback node' warns on stderr and executes PATH binary!
    let output_fallback = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .env("PATH", &sys_dir)
        .args(["run", "--allow-fallback", "node"])
        .output()
        .expect("Failed to run scrim run --allow-fallback node");
    assert!(output_fallback.status.success());
    let stdout_fallback = String::from_utf8(output_fallback.stdout).unwrap();
    assert_eq!(stdout_fallback.trim(), "path node");
    let stderr_fallback = String::from_utf8(output_fallback.stderr).unwrap();
    assert!(stderr_fallback.contains("[Scrim] Warning: Tool 'node' has an invalid configuration"));
}

#[test]
fn test_scrim_which_allow_fallback_to_lower_layer() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Parent config defines valid node
    let parent_config = temp_path.join("scrim.yaml");
    fs::write(
        &parent_config,
        r#"
tools:
  node:
    platforms:
      linux-x86_64:
        system_path: /bin/parent_node
"#,
    )
    .unwrap();

    // 2. Child config defines node with a semantic error
    let child_dir = temp_path.join("child");
    fs::create_dir_all(&child_dir).unwrap();
    let child_config = child_dir.join("scrim.yaml");
    fs::write(
        &child_config,
        r#"
tools:
  node:
    platforms:
      linux-x86_64:
        url: https://example.com/broken_node.tar.gz
"#,
    )
    .unwrap();

    // 3. Normal 'scrim which node' fails fast without fallback
    let output_normal = Command::new(&scrim_bin)
        .current_dir(&child_dir)
        .args(["which", "node"])
        .output()
        .expect("Failed to run scrim which node");
    assert_eq!(output_normal.status.code(), Some(1));
    let stderr_normal = String::from_utf8(output_normal.stderr).unwrap();
    assert!(stderr_normal.contains("[Scrim] Error: Tool 'node' specifies 'url' without a digest"));

    // 4. 'scrim which --allow-fallback node' warns on stderr and prints parent layer's node path
    let output_fallback = Command::new(&scrim_bin)
        .current_dir(&child_dir)
        .args(["which", "--allow-fallback", "node"])
        .output()
        .expect("Failed to run scrim which --allow-fallback node");
    assert!(output_fallback.status.success());
    let stdout_fallback = String::from_utf8(output_fallback.stdout).unwrap();
    assert_eq!(stdout_fallback.trim(), "/bin/parent_node");
    let stderr_fallback = String::from_utf8(output_fallback.stderr).unwrap();
    assert!(stderr_fallback.contains("[Scrim] Warning: Tool 'node' has an invalid configuration"));
}

#[test]
fn test_scrim_which_allow_fallback_to_path() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. External binary on PATH
    let sys_dir = temp_path.join("sys");
    fs::create_dir_all(&sys_dir).unwrap();
    let sys_node = sys_dir.join("node");
    write_executable(&sys_node, "#!/bin/sh\necho path node\n");

    // 2. Workspace config defines node with semantic error
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        r#"
tools:
  node:
    platforms:
      linux-x86_64:
        url: https://example.com/broken_node.tar.gz
"#,
    )
    .unwrap();

    // 3. Normal 'scrim which node' fails fast without fallback to PATH
    let output_normal = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .env("PATH", &sys_dir)
        .args(["which", "node"])
        .output()
        .expect("Failed to run scrim which node");
    assert_eq!(output_normal.status.code(), Some(1));
    let stderr_normal = String::from_utf8(output_normal.stderr).unwrap();
    assert!(stderr_normal.contains("[Scrim] Error: Tool 'node' specifies 'url' without a digest"));

    // 4. 'scrim which --allow-fallback node' warns on stderr and prints PATH binary!
    let output_fallback = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .env("PATH", &sys_dir)
        .args(["which", "--allow-fallback", "node"])
        .output()
        .expect("Failed to run scrim which --allow-fallback node");
    assert!(output_fallback.status.success());
    let stdout_fallback = String::from_utf8(output_fallback.stdout).unwrap();
    assert_eq!(stdout_fallback.trim(), sys_node.to_str().unwrap());
    let stderr_fallback = String::from_utf8(output_fallback.stderr).unwrap();
    assert!(stderr_fallback.contains("[Scrim] Warning: Tool 'node' has an invalid configuration"));
}

#[test]
fn test_scrim_config_allow_fallback() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // 1. Parent config defines valid node
    let parent_config = temp_path.join("scrim.yaml");
    fs::write(
        &parent_config,
        r#"
tools:
  node:
    platforms:
      linux-x86_64:
        system_path: /bin/parent_node
"#,
    )
    .unwrap();

    // 2. Child config defines valid good_tool and broken node
    let child_dir = temp_path.join("child");
    fs::create_dir_all(&child_dir).unwrap();
    let child_config = child_dir.join("scrim.yaml");
    fs::write(
        &child_config,
        r#"
tools:
  good_tool:
    platforms:
      linux-x86_64:
        system_path: /bin/good
  node:
    platforms:
      linux-x86_64:
        url: https://example.com/broken_node.tar.gz
"#,
    )
    .unwrap();

    // 3. 'scrim config --allow-fallback node' warns on stderr and outputs parent node's YAML, exiting 0
    let output_single = Command::new(&scrim_bin)
        .current_dir(&child_dir)
        .args(["config", "--allow-fallback", "node"])
        .output()
        .expect("Failed to run scrim config --allow-fallback node");
    assert!(output_single.status.success());
    let stdout_single = String::from_utf8(output_single.stdout).unwrap();
    assert!(stdout_single.contains("system_path: /bin/parent_node"));
    let stderr_single = String::from_utf8(output_single.stderr).unwrap();
    assert!(stderr_single.contains("[Scrim] Warning: Tool 'node' has an invalid configuration"));

    // 4. 'scrim config --allow-fallback' without arguments: prints good_tool AND fallback node, warns on stderr, exits 0
    let output_all = Command::new(&scrim_bin)
        .current_dir(&child_dir)
        .args(["config", "--allow-fallback"])
        .output()
        .expect("Failed to run scrim config --allow-fallback");
    assert!(output_all.status.success());
    let stdout_all = String::from_utf8(output_all.stdout).unwrap();
    assert!(stdout_all.contains("good_tool:"));
    assert!(stdout_all.contains("node:"));
    assert!(stdout_all.contains("/bin/parent_node"));
    let stderr_all = String::from_utf8(output_all.stderr).unwrap();
    assert!(stderr_all.contains("[Scrim] Warning: Tool 'node' has an invalid configuration"));
}

#[test]
fn test_scrim_config_with_semantic_errors() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    // scrim.yaml with good_tool (valid) and bad_tool (broken)
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        r#"
tools:
  good_tool:
    platforms:
      linux-x86_64:
        system_path: /bin/good
  bad_tool:
    platforms:
      linux-x86_64:
        url: https://example.com/bad.tar.gz
"#,
    )
    .unwrap();

    // 1. 'scrim config good_tool' succeeds and prints YAML
    let output_good = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .args(["config", "good_tool"])
        .output()
        .expect("Failed to run scrim config good_tool");
    assert!(output_good.status.success());
    let stdout_good = String::from_utf8(output_good.stdout).unwrap();
    assert!(stdout_good.contains("system_path: /bin/good"));

    // 2. 'scrim config bad_tool' fails with exit 1 and diagnostic on stderr
    let output_bad = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .args(["config", "bad_tool"])
        .output()
        .expect("Failed to run scrim config bad_tool");
    assert_eq!(output_bad.status.code(), Some(1));
    let stderr_bad = String::from_utf8(output_bad.stderr).unwrap();
    assert!(stderr_bad.contains("[Scrim] Error: Tool 'bad_tool' specifies 'url' without a digest"));

    // 3. 'scrim config' without arguments: prints good_tool on stdout, error for bad_tool on stderr, exits 1
    let output_all = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .args(["config"])
        .output()
        .expect("Failed to run scrim config");
    assert_eq!(output_all.status.code(), Some(1));
    let stdout_all = String::from_utf8(output_all.stdout).unwrap();
    assert!(stdout_all.contains("good_tool:"));
    assert!(!stdout_all.contains("bad_tool:"));
    let stderr_all = String::from_utf8(output_all.stderr).unwrap();
    assert!(stderr_all.contains("[Scrim] Error: Tool 'bad_tool' specifies 'url' without a digest"));
}

#[test]
fn test_scrim_links_aborts_on_semantic_error() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let target_links_dir = temp_path.join("bin");
    fs::create_dir_all(&target_links_dir).unwrap();

    // scrim.yaml with good_tool (valid) and bad_tool (broken)
    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        r#"
tools:
  good_tool:
    platforms:
      linux-x86_64:
        system_path: /bin/good
  bad_tool:
    platforms:
      linux-x86_64:
        url: https://example.com/bad.tar.gz
"#,
    )
    .unwrap();

    // 'scrim links' must fail immediately without creating any links
    let output = Command::new(&scrim_bin)
        .current_dir(temp_path)
        .args(["links", target_links_dir.to_str().unwrap()])
        .output()
        .expect("Failed to run scrim links");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("[Scrim] Error: Tool 'bad_tool' specifies 'url' without a digest"));

    // Verify target directory has NO links created
    assert!(!target_links_dir.join("good_tool").exists());
    assert!(!target_links_dir.join("bad_tool").exists());
}

fn poll_log_contains(log_path: &Path, needle: &str) -> String {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    while start.elapsed() < timeout {
        if let Ok(content) = fs::read_to_string(log_path) {
            if content.contains(needle) {
                return content;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!(
        "Timed out waiting for '{}' in telemetry log '{}'",
        needle,
        log_path.display()
    );
}

#[test]
fn test_e2e_telemetry_file_default_xdg_and_custom_path() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let local_tool = temp_path.join("fast_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"fast output\"\n");

    let state_home = temp_path.join("state_root");
    let config_home = temp_path.join("xdg_config");
    let custom_log = temp_path.join("custom/nested/dir/telemetry.log");

    // 1. Default XDG path via `file: {}`
    let proj_default = temp_path.join("proj_default");
    fs::create_dir_all(proj_default.join(".git")).unwrap();
    fs::write(
        proj_default.join("scrim.yaml"),
        format!(
            "telemetry:\n  file: {{}}\ndefault_platform: linux-x86_64\ntools:\n  fast_tool:\n    system_path: \"{}\"\n",
            local_tool.display()
        ),
    )
    .unwrap();
    let shim_default = proj_default.join("fast_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_default).unwrap();

    let out = Command::new(&shim_default)
        .current_dir(&proj_default)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "fast output\n");

    let default_log = state_home.join("scrim/telemetry.log");
    let default_content = poll_log_contains(&default_log, "\"tool\": \"fast_tool\"");
    assert!(default_content.contains(&format!("\"target\": \"{}\"", local_tool.display())));

    // 2. Explicit custom nested path via string shorthand `file: <abs_path>`
    let proj_custom = temp_path.join("proj_custom");
    fs::create_dir_all(proj_custom.join(".git")).unwrap();
    fs::write(
        proj_custom.join("scrim.yaml"),
        format!(
            "telemetry:\n  file: \"{}\"\ndefault_platform: linux-x86_64\ntools:\n  fast_tool:\n    system_path: \"{}\"\n",
            custom_log.display(),
            local_tool.display()
        ),
    )
    .unwrap();
    let shim_custom = proj_custom.join("fast_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_custom).unwrap();

    let out_custom = Command::new(&shim_custom)
        .current_dir(&proj_custom)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();
    assert!(out_custom.status.success());
    let custom_content = poll_log_contains(&custom_log, "\"tool\": \"fast_tool\"");
    assert!(custom_content.contains(&format!("\"target\": \"{}\"", local_tool.display())));

    // 3. `scrim config` serializes string shorthand in canonical sequence-of-objects form
    let out_cfg = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(&proj_custom)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .unwrap();
    assert!(out_cfg.status.success());
    let cfg_stdout = String::from_utf8(out_cfg.stdout).unwrap();
    assert!(cfg_stdout.contains("telemetry:"));
    assert!(cfg_stdout.contains("file:"));
    assert!(cfg_stdout.contains(&format!("- path: {}", custom_log.display())));
}

#[test]
fn test_e2e_telemetry_additive_layering_and_deduplication() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let local_tool = temp_path.join("my_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"layered ok\"\n");

    let state_home = temp_path.join("state");
    let config_home = temp_path.join("xdg_config");
    let default_log = state_home.join("scrim/telemetry.log");
    let log_a = temp_path.join("logs/a.log");
    let log_b = temp_path.join("logs/b.log");

    // Parent config sets log_a
    let repo_root = temp_path.join("repo");
    let child_dir = repo_root.join("sub");
    fs::create_dir_all(repo_root.join(".git")).unwrap();
    fs::create_dir_all(&child_dir).unwrap();

    fs::write(
        repo_root.join("scrim.yaml"),
        format!(
            "telemetry:\n  file: \"{}\"\ndefault_platform: linux-x86_64\ntools:\n  my_tool:\n    system_path: \"{}\"\n",
            log_a.display(),
            local_tool.display()
        ),
    )
    .unwrap();

    // Imported file adds default {}, duplicate log_a, duplicate resolved default_log, and log_b
    fs::write(
        child_dir.join("extra.yaml"),
        format!(
            "telemetry:\n  file:\n    - {{}}\n    - \"{}\"\n    - \"{}\"\n    - path: \"{}\"\n",
            log_a.display(),
            default_log.display(),
            log_b.display()
        ),
    )
    .unwrap();

    // Child sets deprecated `telemetry: false` (must NOT disable parent or imported sinks)
    fs::write(
        child_dir.join("scrim.yaml"),
        "telemetry: false\nimports:\n  - path: extra.yaml\n    precedence: after\n",
    )
    .unwrap();

    let shim = child_dir.join("my_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim).unwrap();

    let out = Command::new(&shim)
        .current_dir(&child_dir)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();
    assert!(out.status.success());

    let content_a = poll_log_contains(&log_a, "\"tool\": \"my_tool\"");
    let content_b = poll_log_contains(&log_b, "\"tool\": \"my_tool\"");
    let content_default = poll_log_contains(&default_log, "\"tool\": \"my_tool\"");

    // Small wait to ensure any hypothetical duplicate write would have settled
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(fs::read_to_string(&log_a).unwrap().lines().count(), 1, "log_a should have 1 entry: {:?}", content_a);
    assert_eq!(fs::read_to_string(&log_b).unwrap().lines().count(), 1, "log_b should have 1 entry: {:?}", content_b);
    assert_eq!(
        fs::read_to_string(&default_log).unwrap().lines().count(),
        1,
        "default_log should have 1 entry after runtime deduplication: {:?}",
        content_default
    );
}

#[test]
fn test_e2e_telemetry_relative_path_fault_tolerance() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();
    fs::create_dir_all(temp_path.join(".git")).unwrap();

    let local_tool = temp_path.join("real_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"fault tolerant ok\"\n");

    let valid_log = temp_path.join("valid/telemetry.log");
    let config_home = temp_path.join("xdg_config");
    let state_home = temp_path.join("xdg_state");

    fs::write(
        temp_path.join("scrim.yaml"),
        format!(
            "telemetry:\n  file:\n    - relative/bad.log\n    - \"{}\"\ndefault_platform: linux-x86_64\ntools:\n  my_tool:\n    system_path: \"{}\"\n",
            valid_log.display(),
            local_tool.display()
        ),
    )
    .unwrap();

    let shim = temp_path.join("my_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim).unwrap();

    let out = Command::new(&shim)
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "fault tolerant ok\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("[Scrim] Warning:"), "stderr: {}", stderr);
    assert!(stderr.contains("relative/bad.log"), "stderr: {}", stderr);

    assert!(!temp_path.join("relative/bad.log").exists());
    let valid_content = poll_log_contains(&valid_log, "\"tool\": \"my_tool\"");
    assert!(valid_content.contains(&format!("\"target\": \"{}\"", local_tool.display())));
}

#[test]
fn test_e2e_telemetry_child_detaches_stdio_no_pipe_hang() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();
    fs::create_dir_all(temp_path.join(".git")).unwrap();

    let local_tool = temp_path.join("real_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"fifo test ok\"\n");

    let fifo_path = temp_path.join("telemetry.fifo");
    let mkfifo_status = Command::new("mkfifo").arg(&fifo_path).status().unwrap();
    assert!(mkfifo_status.success(), "mkfifo failed");

    let config_home = temp_path.join("xdg_config");
    let state_home = temp_path.join("xdg_state");
    fs::write(
        temp_path.join("scrim.yaml"),
        format!(
            "telemetry:\n  file: \"{}\"\ndefault_platform: linux-x86_64\ntools:\n  my_tool:\n    system_path: \"{}\"\n",
            fifo_path.display(),
            local_tool.display()
        ),
    )
    .unwrap();

    let shim = temp_path.join("my_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim).unwrap();

    // Run shim with piped stdout/stderr while NO reader is attached to the FIFO.
    // The telemetry child blocks in open(fifo_path), so output() returns promptly
    // ONLY if detach_stdio() closed/redirected inherited stdout/stderr pipes.
    let (tx, rx) = std::sync::mpsc::channel();
    let shim_clone = shim.clone();
    let dir_clone = temp_path.to_path_buf();
    let cfg_clone = config_home.clone();
    let state_clone = state_home.clone();
    let cmd_thread = std::thread::spawn(move || {
        let out = Command::new(&shim_clone)
            .current_dir(&dir_clone)
            .env("HOME", &dir_clone)
            .env("XDG_CONFIG_HOME", &cfg_clone)
            .env("XDG_STATE_HOME", &state_clone)
            .output()
            .unwrap();
        let _ = tx.send(out);
    });

    let out = match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(out) => out,
        Err(_) => {
            // Unblock the child if it held stdout/stderr open while blocked on FIFO
            let _ = fs::OpenOptions::new().read(true).open(&fifo_path);
            panic!("Command::output() hung waiting on telemetry child holding stdio pipes");
        }
    };
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "fifo test ok\n");
    cmd_thread.join().unwrap();

    // Now open the FIFO read end to unblock the telemetry child and verify the entry arrives.
    let (fifo_tx, fifo_rx) = std::sync::mpsc::channel();
    let fifo_clone = fifo_path.clone();
    let reader_thread = std::thread::spawn(move || {
        let content = fs::read_to_string(&fifo_clone).unwrap();
        let _ = fifo_tx.send(content);
    });
    let fifo_content = fifo_rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("Timed out reading telemetry entry from FIFO");
    reader_thread.join().unwrap();
    assert!(fifo_content.contains("\"tool\": \"my_tool\""));
    assert!(fifo_content.contains(&format!("\"target\": \"{}\"", local_tool.display())));
}

#[test]
fn test_e2e_telemetry_scrim_run_and_xdg_state_fallback() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();
    fs::create_dir_all(temp_path.join(".git")).unwrap();

    let local_tool = temp_path.join("real_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"run args=$*\"\n");

    let home_dir = temp_path.join("home");
    let config_home = temp_path.join("xdg_config");
    fs::create_dir_all(&home_dir).unwrap();
    fs::write(
        temp_path.join("scrim.yaml"),
        format!(
            "telemetry:\n  file: {{}}\ndefault_platform: linux-x86_64\ntools:\n  my_tool:\n    system_path: \"{}\"\n",
            local_tool.display()
        ),
    )
    .unwrap();

    let default_log = home_dir.join(".local/state/scrim/telemetry.log");

    // 1. `scrim run` with XDG_STATE_HOME unset falls back to $HOME/.local/state/scrim/telemetry.log
    let out_unset = Command::new(&scrim_bin)
        .args(["run", "my_tool", "one"])
        .current_dir(temp_path)
        .env("HOME", &home_dir)
        .env("XDG_CONFIG_HOME", &config_home)
        .env_remove("XDG_STATE_HOME")
        .output()
        .unwrap();
    assert!(out_unset.status.success());
    assert_eq!(String::from_utf8_lossy(&out_unset.stdout), "run args=one\n");

    let content1 = poll_log_contains(&default_log, "\"tool\": \"my_tool\"");
    assert!(content1.contains(&format!("\"target\": \"{}\"", local_tool.display())));

    // 2. Relative XDG_STATE_HOME is ignored and also falls back to $HOME/.local/state/scrim/telemetry.log
    fs::remove_file(&default_log).unwrap();
    let out_rel = Command::new(&scrim_bin)
        .args(["run", "my_tool", "two"])
        .current_dir(temp_path)
        .env("HOME", &home_dir)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", "relative/state")
        .output()
        .unwrap();
    assert!(out_rel.status.success());
    assert_eq!(String::from_utf8_lossy(&out_rel.stdout), "run args=two\n");

    let content2 = poll_log_contains(&default_log, "\"tool\": \"my_tool\"");
    assert!(content2.contains(&format!("\"target\": \"{}\"", local_tool.display())));
}

#[test]
fn test_e2e_telemetry_invalid_yaml_discarded_retains_lower_layer() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();

    let local_tool = temp_path.join("real_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"lower layer ok\"\n");

    let repo_root = temp_path.join("repo");
    let child_dir = repo_root.join("sub");
    fs::create_dir_all(repo_root.join(".git")).unwrap();
    fs::create_dir_all(&child_dir).unwrap();

    let parent_log = temp_path.join("parent_telemetry.log");
    let config_home = temp_path.join("xdg_config");
    let state_home = temp_path.join("xdg_state");

    fs::write(
        repo_root.join("scrim.yaml"),
        format!(
            "telemetry:\n  file: \"{}\"\ndefault_platform: linux-x86_64\ntools:\n  my_tool:\n    system_path: \"{}\"\n",
            parent_log.display(),
            local_tool.display()
        ),
    )
    .unwrap();

    // Child contains invalid bare-null `file:`, so child file is discarded with warning
    fs::write(
        child_dir.join("scrim.yaml"),
        "telemetry:\n  file:\ntools:\n  my_tool:\n    system_path: /nonexistent/broken_tool\n",
    )
    .unwrap();

    let shim = child_dir.join("my_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim).unwrap();

    let out = Command::new(&shim)
        .current_dir(&child_dir)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "lower layer ok\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("[Scrim] Warning:"), "stderr: {}", stderr);

    let log_content = poll_log_contains(&parent_log, "\"tool\": \"my_tool\"");
    assert!(log_content.contains(&format!("\"target\": \"{}\"", local_tool.display())));
}

#[test]
fn test_e2e_telemetry_relative_path_in_config_and_links() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();
    fs::create_dir_all(temp_path.join(".git")).unwrap();

    let valid_log = temp_path.join("logs/valid.log");
    let config_home = temp_path.join("xdg_config");
    let links_dir = temp_path.join("bin");
    fs::create_dir_all(&links_dir).unwrap();

    fs::write(
        temp_path.join("scrim.yaml"),
        format!(
            "telemetry:\n  file:\n    - relative/bad.log\n    - \"{}\"\ndefault_platform: linux-x86_64\ntools:\n  my_tool:\n    system_path: /bin/echo\n",
            valid_log.display()
        ),
    )
    .unwrap();

    // 1. `scrim config` exits 0, warns on stderr, omits invalid sink, keeps valid sink
    let out_cfg = Command::new(&scrim_bin)
        .arg("config")
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .unwrap();
    assert!(out_cfg.status.success());
    let cfg_stdout = String::from_utf8(out_cfg.stdout).unwrap();
    let cfg_stderr = String::from_utf8(out_cfg.stderr).unwrap();
    assert!(cfg_stderr.contains("[Scrim] Warning:"), "stderr: {}", cfg_stderr);
    assert!(cfg_stderr.contains("relative/bad.log"), "stderr: {}", cfg_stderr);
    assert!(!cfg_stdout.contains("relative/bad.log"), "stdout: {}", cfg_stdout);
    assert!(cfg_stdout.contains(&format!("- path: {}", valid_log.display())), "stdout: {}", cfg_stdout);

    // 2. `scrim links` exits 0, warns on stderr, and still creates the tool symlink
    let out_links = Command::new(&scrim_bin)
        .args(["links", links_dir.to_str().unwrap()])
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .unwrap();
    assert!(out_links.status.success());
    let links_stderr = String::from_utf8(out_links.stderr).unwrap();
    assert!(links_stderr.contains("[Scrim] Warning:"), "stderr: {}", links_stderr);
    assert!(links_dir.join("my_tool").exists());
}

#[test]
fn test_e2e_telemetry_write_failure_isolation() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();
    fs::create_dir_all(temp_path.join(".git")).unwrap();

    let local_tool = temp_path.join("real_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"write isolation ok\"\n");

    // Create a regular file to make unwritable_sink's parent path impossible to create
    let blocker = temp_path.join("blocker");
    fs::write(&blocker, "not a directory").unwrap();
    let unwritable_sink = blocker.join("telemetry.log");
    let valid_log = temp_path.join("valid/telemetry.log");
    let config_home = temp_path.join("xdg_config");
    let state_home = temp_path.join("xdg_state");

    fs::write(
        temp_path.join("scrim.yaml"),
        format!(
            "telemetry:\n  file:\n    - \"{}\"\n    - \"{}\"\ndefault_platform: linux-x86_64\ntools:\n  my_tool:\n    system_path: \"{}\"\n",
            unwritable_sink.display(),
            valid_log.display(),
            local_tool.display()
        ),
    )
    .unwrap();

    let shim = temp_path.join("my_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim).unwrap();

    let out = Command::new(&shim)
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "write isolation ok\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");

    let log_content = poll_log_contains(&valid_log, "\"tool\": \"my_tool\"");
    assert!(log_content.contains(&format!("\"target\": \"{}\"", local_tool.display())));
}

#[test]
fn test_e2e_config_caching_proxy_background_persistence() {
    let scrim_bin = find_scrim_bin();
    let temp = tempdir().unwrap();
    let temp_path = temp.path();
    fs::create_dir_all(temp_path.join(".git")).unwrap();

    let local_tool = temp_path.join("real_tool");
    write_executable(&local_tool, "#!/bin/sh\necho \"cached proxy run ok\"\n");

    let config_home = temp_path.join("xdg_config");
    let cache_home = temp_path.join("xdg_cache");
    let state_home = temp_path.join("xdg_state");

    let scrim_yaml = temp_path.join("scrim.yaml");
    fs::write(
        &scrim_yaml,
        format!(
            "default_platform: any\ntools:\n  my_tool:\n    system_path: \"{}\"\n",
            local_tool.display()
        ),
    )
    .unwrap();

    let shim = temp_path.join("my_tool");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim).unwrap();

    // 1. Initial proxy invocation: execution succeeds immediately
    let out = Command::new(&shim)
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CACHE_HOME", &cache_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "cached proxy run ok\n");

    // The background child should have persisted the parsed config cache under <cache_home>/scrim/config/<hash>.bin
    let config_cache_dir = cache_home.join("scrim/config");
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    let mut cache_file_found = false;
    while start.elapsed() < timeout {
        if let Ok(entries) = fs::read_dir(&config_cache_dir) {
            let files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            if !files.is_empty() {
                cache_file_found = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(cache_file_found, "Expected config cache file to be created in {:?}", config_cache_dir);

    // 2. Second proxy invocation hits binary cache
    let out2 = Command::new(&shim)
        .current_dir(temp_path)
        .env("HOME", temp_path)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CACHE_HOME", &cache_home)
        .env("XDG_STATE_HOME", &state_home)
        .output()
        .unwrap();

    assert!(out2.status.success());
    assert_eq!(String::from_utf8_lossy(&out2.stdout), "cached proxy run ok\n");
}

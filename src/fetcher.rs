use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::io::Write;

pub fn fetch_tool(tool_name: &str, url: &str, sha256: &str, archive_bin: Option<&str>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let cache_dir = Path::new(&home).join(".cache/scrim/tools").join(tool_name).join(sha256);
    
    let is_tar_gz = url.ends_with(".tar.gz") || url.ends_with(".tgz");
    let is_zip = url.ends_with(".zip");
    let is_archive = is_tar_gz || is_zip;

    let target_path = if is_archive {
        if let Some(bin) = archive_bin {
            cache_dir.join(bin)
        } else {
            cache_dir.join(tool_name)
        }
    } else {
        cache_dir.join(tool_name)
    };

    if target_path.exists() {
        return Ok(target_path);
    }

    fs::create_dir_all(&cache_dir)?;

    let download_path = if is_archive {
        cache_dir.join(format!("{}.archive", tool_name))
    } else {
        cache_dir.join(tool_name)
    };

    // Download with curl
    let status = Command::new("curl")
        .arg("-L")
        .arg("-o")
        .arg(&download_path)
        .arg(url)
        .status()?;

    if !status.success() {
        return Err("Failed to download tool".into());
    }

    // Verify SHA256 using sha256sum
    let mut child = Command::new("sha256sum")
        .arg("-c")
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{}  {}", sha256, download_path.display())?;
    }

    let status = child.wait()?;
    if !status.success() {
        let _ = fs::remove_file(&download_path);
        return Err("SHA256 verification failed".into());
    }

    if is_archive {
        if is_tar_gz {
            let status = Command::new("tar")
                .arg("-xzf")
                .arg(&download_path)
                .arg("-C")
                .arg(&cache_dir)
                .status()?;
            if !status.success() {
                let _ = fs::remove_file(&download_path);
                return Err("Failed to extract tar.gz archive".into());
            }
        } else if is_zip {
            let status = Command::new("unzip")
                .arg("-q")
                .arg(&download_path)
                .arg("-d")
                .arg(&cache_dir)
                .status()?;
            if !status.success() {
                let _ = fs::remove_file(&download_path);
                return Err("Failed to extract zip archive".into());
            }
        }
        let _ = fs::remove_file(&download_path);
    }

    // Ensure the binary is executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if target_path.exists() {
            let mut perms = fs::metadata(&target_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&target_path, perms)?;
        } else {
            return Err(format!("Extracted executable not found at {}", target_path.display()).into());
        }
    }

    Ok(target_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_fetch_tool_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        // 1. Create a local mock source tool
        let mock_src = temp_path.join("mock_src");
        fs::write(&mock_src, "echo 'hello'").unwrap();

        // Compute sha256
        let sha256_output = Command::new("sha256sum")
            .arg(&mock_src)
            .output()
            .unwrap();
        let sha256_str = String::from_utf8(sha256_output.stdout).unwrap();
        let sha256 = sha256_str.split_whitespace().next().unwrap().to_string();

        // 2. Set temporary HOME environment variable to isolate the cache
        let fake_home = temp_path.join("fake_home");
        fs::create_dir_all(&fake_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        // 3. Fetch the tool using file:// URL (which curl processes identically)
        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, &sha256, None);

        // Restore original HOME immediately to prevent side-effects on other tests
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        // 4. Assertions
        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/my_test_tool"));

        // Verify it was marked executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "File is not executable!");
        }
    }

    #[test]
    fn test_fetch_tool_sha_mismatch() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mock_src = temp_path.join("mock_src");
        fs::write(&mock_src, "echo 'hello'").unwrap();

        let fake_home = temp_path.join("fake_home");
        fs::create_dir_all(&fake_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "incorrect_sha_hash", None);

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        // Assertions: should fail with SHA verification error
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("verification failed"));
    }

    #[test]
    fn test_fetch_tool_tar_gz_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        // Create a mock binary structure inside an archive
        let mock_src_dir = temp_path.join("mock_src_dir");
        fs::create_dir_all(mock_src_dir.join("bin")).unwrap();
        fs::write(mock_src_dir.join("bin").join("mytool"), "echo 'hello archive'").unwrap();
        
        let tar_gz_path = temp_path.join("mytool.tar.gz");
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&tar_gz_path)
            .arg("-C")
            .arg(&mock_src_dir)
            .arg(".")
            .status()
            .unwrap();
        assert!(status.success());

        // Compute sha256 of the archive
        let sha256_output = Command::new("sha256sum")
            .arg(&tar_gz_path)
            .output()
            .unwrap();
        let sha256_str = String::from_utf8(sha256_output.stdout).unwrap();
        let sha256 = sha256_str.split_whitespace().next().unwrap().to_string();

        let fake_home = temp_path.join("fake_home");
        fs::create_dir_all(&fake_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, &sha256, Some("bin/mytool"));

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/my_test_tool"));
        assert!(target_path.to_str().unwrap().ends_with("bin/mytool"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "File is not executable!");
        }
    }
}

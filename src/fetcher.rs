use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self};
use sha2::{Sha256, Digest};
use flate2::read::GzDecoder;
use tar::Archive;
use zip::ZipArchive;

pub fn fetch_tool(tool_name: &str, url: &str, sha256: &str, archive_path: Option<&str>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let cache_dir = Path::new(&home).join(".cache/scrim/tools").join(sha256);
    
    let is_tar_gz = url.ends_with(".tar.gz") || url.ends_with(".tgz");
    let is_zip = url.ends_with(".zip");
    let is_archive = is_tar_gz || is_zip;

    let target_path = if is_archive {
        if let Some(bin) = archive_path {
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
        cache_dir.join("download.archive")
    } else {
        cache_dir.join(tool_name)
    };

    if let Some(local_path) = url.strip_prefix("file://") {
        if !Path::new(local_path).exists() {
            return Err(format!("Failed to download tool: local file {} does not exist", local_path).into());
        }
        fs::copy(local_path, &download_path)?;
    } else {
        crate::scrim_progress!("Downloading {}...", tool_name);
        let response = match ureq::get(url).call() {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return Err(format!("Failed to download tool: HTTP status {}", resp.status()).into());
                }
                resp
            },
            Err(e) => return Err(format!("Failed to download tool: {}", e).into()),
        };
        let mut reader = response.into_body().into_reader();
        let mut file = File::create(&download_path)?;
        io::copy(&mut reader, &mut file)?;
    }

    let mut file = File::open(&download_path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let computed_sha256 = format!("{:x}", hasher.finalize());

    if computed_sha256 != sha256 {
        let err_msg = format!("SHA256 verification failed for {}.\nExpected: {}\nActual:   {}", tool_name, sha256, computed_sha256);
        let _ = fs::remove_file(&download_path);
        return Err(err_msg.into());
    }

    if is_archive {
        crate::scrim_progress!("Unpacking {} archive...", tool_name);
        if is_tar_gz {
            let tar_gz = File::open(&download_path)?;
            let tar = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(tar);
            if let Err(e) = archive.unpack(&cache_dir) {
                let _ = fs::remove_file(&download_path);
                return Err(format!("Failed to extract tar.gz archive: {}", e).into());
            }
        } else if is_zip {
            let zip_file = File::open(&download_path)?;
            let mut archive = match ZipArchive::new(zip_file) {
                Ok(a) => a,
                Err(e) => {
                    let _ = fs::remove_file(&download_path);
                    return Err(format!("Failed to open zip archive: {}", e).into());
                }
            };
            if let Err(e) = archive.extract(&cache_dir) {
                let _ = fs::remove_file(&download_path);
                return Err(format!("Failed to extract zip archive: {}", e).into());
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
        let mut file = fs::File::open(&mock_src).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha256 = format!("{:x}", hasher.finalize());

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
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/"));
        assert!(target_path.to_str().unwrap().ends_with("my_test_tool"));

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

        // Compute actual sha256 to verify error message contents
        let mut file = fs::File::open(&mock_src).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let actual_sha256 = format!("{:x}", hasher.finalize());

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
        assert!(err_msg.contains("SHA256 verification failed for my_test_tool."));
        assert!(err_msg.contains("Expected: incorrect_sha_hash"));
        assert!(err_msg.contains(&format!("Actual:   {}", actual_sha256)));
    }

    #[test]
    fn test_fetch_tool_download_failure() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        fs::create_dir_all(&fake_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        // A non-existent file path will cause a local file fetch to fail
        let file_url = "file:///tmp/this_file_does_not_exist_scrim_test_12345";
        let result = fetch_tool("my_test_tool", file_url, "any_sha256", None);

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool"));
        assert!(err_msg.contains("does not exist"));
    }

    #[test]
    fn test_fetch_tool_http_404_failure() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        fs::create_dir_all(&fake_home).unwrap();
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        // Start a mock HTTP server returning 404
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf);
                let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found";
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let url = format!("http://127.0.0.1:{}/not_found", port);
        let result = fetch_tool("my_test_tool", &url, "any_sha256", None);

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("404"), "Expected HTTP 404 error, got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_network_error() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        fs::create_dir_all(&fake_home).unwrap();
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        // Use a port that is definitely not listening to force a connection error
        let url = "http://127.0.0.1:1";
        let result = fetch_tool("my_test_tool", url, "any_sha256", None);

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool:"), "Got: {}", err_msg);
        assert!(!err_msg.ends_with("Failed to download tool"), "Underlying error was swallowed");
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
        let tar_gz = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        builder.append_dir_all(".", &mock_src_dir).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        // Compute sha256 of the archive
        let mut file = fs::File::open(&tar_gz_path).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha256 = format!("{:x}", hasher.finalize());

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
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/"));
        assert!(target_path.to_str().unwrap().ends_with("bin/mytool"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "File is not executable!");
        }
    }

    #[test]
    fn test_fetch_tool_zip_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let zip_path = temp_path.join("mytool.zip");
        let zip_file = fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_file);
        
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
            
        zip.start_file("bin/", options.clone()).unwrap();
        zip.start_file("bin/mytool", options).unwrap();
        use std::io::Write;
        zip.write_all(b"echo 'hello zip archive'").unwrap();
        zip.finish().unwrap();

        let mut file = fs::File::open(&zip_path).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha256 = format!("{:x}", hasher.finalize());

        let fake_home = temp_path.join("fake_home");
        fs::create_dir_all(&fake_home).unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        let file_url = format!("file://{}", zip_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, &sha256, Some("bin/mytool"));

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
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

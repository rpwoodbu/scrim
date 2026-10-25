use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self, Write};
use archive::{ArchiveFormat, extract_and_prepare_archive, resolve_relative_bin_path};
use hashing::HashingWriter;

fn cached_urls_contain(url_file: &Path, url: &str) -> bool {
    if let Ok(content) = fs::read_to_string(url_file) {
        content.lines().any(|line| line.trim() == url.trim())
    } else {
        false
    }
}

fn record_cached_url(url_file: &Path, url: &str) -> io::Result<()> {
    if cached_urls_contain(url_file, url) {
        return Ok(());
    }
    let needs_leading_newline = if let Ok(content) = fs::read_to_string(url_file) {
        !content.is_empty() && !content.ends_with('\n')
    } else {
        false
    };

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(url_file)?;
    if needs_leading_newline {
        writeln!(file)?;
    }
    writeln!(file, "{}", url.trim())?;
    file.flush()?;
    Ok(())
}

fn tool_cache_dir(base_cache_dir: &Path, digest_algorithm: &str, digest_value: &str) -> PathBuf {
    base_cache_dir
        .join("tools")
        .join(format!("{}:{}", digest_algorithm, digest_value))
}

pub fn cached_binary_path(
    tool_name: &str,
    url: &str,
    digest_algorithm: &str,
    digest_value: &str,
    archive_path: Option<&str>,
    base_cache_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cache_dir = tool_cache_dir(base_cache_dir, digest_algorithm, digest_value);
    Ok(cache_dir.join(resolve_relative_bin_path(tool_name, url, archive_path)?))
}

pub fn fetch_tool(
    tool_name: &str,
    url: &str,
    digest_algorithm: &str,
    digest_value: &str,
    archive_path: Option<&str>,
    base_cache_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let tools_dir = base_cache_dir.join("tools");
    let cache_key = format!("{}:{}", digest_algorithm, digest_value);
    let cache_dir = tools_dir.join(&cache_key);
    let url_file = tools_dir.join(format!("{}.url", cache_key));

    let rel_bin_path = resolve_relative_bin_path(tool_name, url, archive_path)?;
    let target_path = cache_dir.join(&rel_bin_path);

    if target_path.exists() && cached_urls_contain(&url_file, url) {
        return Ok(target_path);
    }

    let archive_format = ArchiveFormat::from_url(url);

    fs::create_dir_all(&tools_dir)?;

    // Download to a temporary file in the tools directory
    let mut temp_file = tempfile::NamedTempFile::new_in(&tools_dir)?;
    let mut hw = HashingWriter::new(&mut temp_file, digest_algorithm)?;

    if let Some(local_path) = url.strip_prefix("file://") {
        let local_path = local_path.split('?').next().unwrap_or(local_path);
        if !Path::new(local_path).exists() {
            return Err(format!("Failed to download tool: local file {} does not exist", local_path).into());
        }
        let mut file = File::open(local_path)?;
        io::copy(&mut file, &mut hw)?;
    } else {
        logger::scrim_progress!("Downloading {}...", tool_name);
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
        io::copy(&mut reader, &mut hw)?;
    }

    hw.verify_digest(tool_name, digest_algorithm, digest_value)?;

    if let Some(format) = archive_format {
        logger::scrim_progress!("Unpacking {} archive...", tool_name);

        // Extract to a TempDir first to ensure atomicity
        let unpack_dir = tempfile::TempDir::new_in(&tools_dir)?;
        let archive_file = temp_file.reopen()?;
        extract_and_prepare_archive(
            format,
            archive_file,
            &tools_dir,
            unpack_dir.path(),
            &rel_bin_path,
        )?;

        // Atomically move the extracted directory into place
        if let Err(e) = fs::rename(unpack_dir.path(), &cache_dir) {
            // If the rename failed because the directory isn't empty, another process already populated the cache.
            // On Unix, this is typically ENOTEMPTY or EEXIST.
            if target_path.exists() {
                // Someone else succeeded, ignore the error
            } else {
                return Err(format!("Failed to persist extracted archive to cache: {}", e).into());
            }
        } else {
            // Rename succeeded, prevent the TempDir destructor from trying to delete the old path
            let _ = unpack_dir.keep();
        }
        record_cached_url(&url_file, url)?;
    } else {
        fs::create_dir_all(&cache_dir)?;

        // Ensure the binary is executable BEFORE exposing it
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::symlink_metadata(temp_file.path())?;
            if metadata.file_type().is_symlink() {
                return Err(format!("Downloaded executable {} is a symlink, which is not allowed", temp_file.path().display()).into());
            }
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(temp_file.path(), perms)?;
        }

        if let Err(e) = temp_file.persist(&target_path) {
            if target_path.exists() {
                // Someone else succeeded
            } else {
                return Err(format!("Failed to persist single binary to cache: {}", e).into());
            }
        }
        record_cached_url(&url_file, url)?;
    }

    Ok(target_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256, Sha512};
    use tempfile::tempdir;

    #[test]
    fn test_tool_cache_dir() {
        let base = Path::new("/home/user/.cache/scrim");
        assert_eq!(
            tool_cache_dir(base, "sha256", "abc12345"),
            PathBuf::from("/home/user/.cache/scrim/tools/sha256:abc12345")
        );
        assert_eq!(
            tool_cache_dir(base, "sha512", "def67890"),
            PathBuf::from("/home/user/.cache/scrim/tools/sha512:def67890")
        );
    }

    #[test]
    fn test_cached_binary_path_archive_and_binary() {
        let dir = Path::new("/cache");
        // Non-archive
        let p1 = cached_binary_path("mytool", "https://example.com/bin", "sha256", "1234", None, dir).unwrap();
        assert_eq!(p1, PathBuf::from("/cache/tools/sha256:1234/mytool"));

        // Archive with custom path
        let p2 = cached_binary_path("mytool", "https://example.com/bin.tar.gz", "sha256", "1234", Some("sub/bin"), dir).unwrap();
        assert_eq!(p2, PathBuf::from("/cache/tools/sha256:1234/sub/bin"));

        // Archive with default path
        let p3 = cached_binary_path("mytool", "https://example.com/bin.tar.gz", "sha256", "1234", None, dir).unwrap();
        assert_eq!(p3, PathBuf::from("/cache/tools/sha256:1234/mytool"));

        // AppImage default path
        let p4 = cached_binary_path("mytool", "https://example.com/pkg.AppImage", "sha256", "1234", None, dir).unwrap();
        assert_eq!(p4, PathBuf::from("/cache/tools/sha256:1234/AppRun"));

        // Invalid archive paths
        assert!(cached_binary_path("mytool", "https://example.com/bin.tar.gz", "sha256", "1234", Some("/abs/path"), dir).is_err());
        assert!(cached_binary_path("mytool", "https://example.com/bin.tar.gz", "sha256", "1234", Some("sub/../bad"), dir).is_err());
    }

    #[test]
    fn test_fetch_tool_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mock_src = temp_path.join("mock_src");
        fs::write(&mock_src, "echo 'hello'").unwrap();

        let mut file = fs::File::open(&mock_src).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha256 = format!("{:x}", hasher.finalize());

        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", &sha256, None, &base_cache);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/"));
        assert!(target_path.to_str().unwrap().ends_with("my_test_tool"));

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}.url", sha256));
        assert!(url_file.exists());
        assert_eq!(fs::read_to_string(&url_file).unwrap().trim(), file_url.trim());

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

        let mut file = fs::File::open(&mock_src).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let actual_sha256 = format!("{:x}", hasher.finalize());

        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", "incorrect_sha_hash", None, &base_cache);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("SHA256 verification failed for my_test_tool."));
        assert!(err_msg.contains("Expected: incorrect_sha_hash"));
        assert!(err_msg.contains(&format!("Actual:   {}", actual_sha256)));
    }

    #[test]
    fn test_fetch_tool_sha512_success_and_mismatch() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mock_src = temp_path.join("mock_src_sha512");
        fs::write(&mock_src, "echo 'hello sha512'").unwrap();

        let mut file = fs::File::open(&mock_src).unwrap();
        let mut hasher = Sha512::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha512 = format!("{:x}", hasher.finalize());

        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_sha512_tool", &file_url, "sha512", &sha512, None, &base_cache);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/"));

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha512:{}.url", sha512));
        assert!(url_file.exists());
        assert_eq!(fs::read_to_string(&url_file).unwrap().trim(), file_url.trim());

        // Now test mismatch with sha512
        let result_bad = fetch_tool("my_sha512_tool", &file_url, "sha512", "bad_hash", None, &base_cache);
        assert!(result_bad.is_err());
        let err_msg = result_bad.err().unwrap().to_string();
        assert!(err_msg.contains("SHA512 verification failed for my_sha512_tool."));
        assert!(err_msg.contains("Expected: bad_hash"));
        assert!(err_msg.contains(&format!("Actual:   {}", sha512)));
    }

    #[test]
    fn test_fetch_tool_download_failure() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let file_url = "file:///tmp/this_file_does_not_exist_scrim_test_12345";
        let result = fetch_tool("my_test_tool", file_url, "sha256", "any_sha256", None, &base_cache);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool"));
        assert!(err_msg.contains("does not exist"));
    }

    #[test]
    fn test_fetch_tool_http_404_failure() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf);
                let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found";
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let url = format!("http://127.0.0.1:{}/not_found", port);
        let result = fetch_tool("my_test_tool", &url, "sha256", "any_sha256", None, &base_cache);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("404"), "Expected HTTP 404 error, got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_network_error() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let url = "http://127.0.0.1:1";
        let result = fetch_tool("my_test_tool", url, "sha256", "any_sha256", None, &base_cache);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool:"), "Got: {}", err_msg);
        assert!(!err_msg.ends_with("Failed to download tool"), "Underlying error was swallowed");
    }

    #[test]
    fn test_fetch_tool_tar_gz_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mock_src_dir = temp_path.join("mock_src_dir");
        fs::create_dir_all(mock_src_dir.join("bin")).unwrap();
        fs::write(mock_src_dir.join("bin").join("mytool"), "echo 'hello archive'").unwrap();
        
        let tar_gz_path = temp_path.join("mytool.tar.gz");
        let tar_gz = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        builder.append_dir_all(".", &mock_src_dir).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let mut file = fs::File::open(&tar_gz_path).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha256 = format!("{:x}", hasher.finalize());

        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", &sha256, Some("bin/mytool"), &base_cache);

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
    fn test_fetch_tool_archive_path_validation() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let url = "https://example.com/tool.tar.gz";

        // Test absolute path
        let res_absolute = fetch_tool("my_tool", url, "sha256", "any_sha", Some("/absolute/path"), &base_cache);
        assert!(res_absolute.is_err());
        assert!(res_absolute.err().unwrap().to_string().contains("must be a relative path"));

        // Test '..' traversal
        let res_traversal = fetch_tool("my_tool", url, "sha256", "any_sha", Some("bin/../secret"), &base_cache);
        assert!(res_traversal.is_err());
        assert!(res_traversal.err().unwrap().to_string().contains("without '..'"));
    }

    #[test]
    fn test_fetch_tool_http_streaming_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let content = b"echo 'hello from http server'";
        let mut hasher = Sha256::new();
        hasher.update(content);
        let sha256 = format!("{:x}", hasher.finalize());

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content.len(),
                    std::str::from_utf8(content).unwrap()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let url = format!("http://127.0.0.1:{}/binary", port);
        let result = fetch_tool("http_tool", &url, "sha256", &sha256, None, &base_cache);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert_eq!(fs::read(&target_path).unwrap(), content);
    }

    #[test]
    fn test_fetch_tool_http_ipv6_streaming_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let content = b"echo 'hello from ipv6 http server'";
        let mut hasher = Sha256::new();
        hasher.update(content);
        let sha256 = format!("{:x}", hasher.finalize());

        let listener = match std::net::TcpListener::bind("[::1]:0") {
            Ok(l) => l,
            Err(_) => return, // Gracefully skip if IPv6 loopback is unavailable in environment
        };
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content.len(),
                    std::str::from_utf8(content).unwrap()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let url = format!("http://[::1]:{}/binary", port);
        let result = fetch_tool("http_tool_ipv6", &url, "sha256", &sha256, None, &base_cache);

        assert!(result.is_ok(), "Failed IPv6 fetch: {:?}", result.err());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert_eq!(fs::read(&target_path).unwrap(), content);
    }

    #[test]
    fn test_fetch_tool_http_ipv6_network_error() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        // Port 1 on IPv6 loopback is not listening
        let url = "http://[::1]:1";
        let result = fetch_tool("ipv6_error_tool", url, "sha256", "any_sha256", None, &base_cache);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool:"), "Got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_existing_cache_fast_path() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = tool_cache_dir(&base_cache, "sha256", sha256);
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}.url", sha256));
        fs::write(&url_file, "http://invalid.invalid/never_called\n").unwrap();

        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", "sha256", sha256, None, &base_cache);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), cached_bin);
    }

    #[test]
    fn test_fetch_tool_existing_cache_missing_url_file_downloads() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = tool_cache_dir(&base_cache, "sha256", sha256);
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        // Without <sha256>.url, fetch_tool must not return the cached binary and must attempt download
        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", "sha256", sha256, None, &base_cache);

        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("Failed to download tool"));
    }

    #[test]
    fn test_fetch_tool_existing_cache_mismatched_url_downloads() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = tool_cache_dir(&base_cache, "sha256", sha256);
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}.url", sha256));
        fs::write(&url_file, "http://previously.downloaded/tool\n").unwrap();

        // With mismatched URL in <sha256>.url, fetch_tool must not use cache and must attempt download
        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", "sha256", sha256, None, &base_cache);

        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("Failed to download tool"));
    }

    #[test]
    fn test_fetch_tool_url_updated_hash_mismatch_facilitates_update() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        // 1. Initial cached version (v1)
        let old_src = temp_path.join("v1_tool");
        fs::write(&old_src, b"version 1 binary").unwrap();
        let old_sha256 = format!("{:x}", Sha256::digest(b"version 1 binary"));
        let old_url = format!("file://{}", old_src.to_str().unwrap());

        let res_v1 = fetch_tool("my_tool", &old_url, "sha256", &old_sha256, None, &base_cache);
        assert!(res_v1.is_ok());

        // 2. User updates URL to v2 but leaves sha256 as old_sha256
        let new_src = temp_path.join("v2_tool");
        fs::write(&new_src, b"version 2 binary with different content").unwrap();
        let new_sha256 = format!("{:x}", Sha256::digest(b"version 2 binary with different content"));
        let new_url = format!("file://{}", new_src.to_str().unwrap());

        let res_v2 = fetch_tool("my_tool", &new_url, "sha256", &old_sha256, None, &base_cache);
        assert!(res_v2.is_err());
        let err_msg = res_v2.err().unwrap().to_string();
        assert!(err_msg.contains("SHA256 verification failed for my_tool."));
        assert!(err_msg.contains(&format!("Expected: {}", old_sha256)));
        assert!(err_msg.contains(&format!("Actual:   {}", new_sha256)));
    }

    #[test]
    fn test_fetch_tool_multiple_urls_for_same_hash() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");
        let base_cache = fake_home.join(".cache/scrim");

        let content = b"identical tool payload";
        let sha256 = format!("{:x}", Sha256::digest(content));

        let src1 = temp_path.join("src1");
        fs::write(&src1, content).unwrap();
        let url1 = format!("file://{}", src1.to_str().unwrap());

        let src2 = temp_path.join("src2");
        fs::write(&src2, content).unwrap();
        let url2 = format!("file://{}", src2.to_str().unwrap());

        // Fetch from url1
        let res1 = fetch_tool("multi_tool", &url1, "sha256", &sha256, None, &base_cache);
        assert!(res1.is_ok());

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}.url", sha256));
        assert!(url_file.exists());
        let file_content = fs::read_to_string(&url_file).unwrap();
        assert!(file_content.contains(&url1));

        // Fetch from url2 (mirror)
        let res2 = fetch_tool("multi_tool", &url2, "sha256", &sha256, None, &base_cache);
        assert!(res2.is_ok());

        let updated_content = fs::read_to_string(&url_file).unwrap();
        assert!(updated_content.contains(&url1));
        assert!(updated_content.contains(&url2));

        // Now remove src1 and src2 from disk to guarantee both URLs hit the cache without downloading
        fs::remove_file(&src1).unwrap();
        fs::remove_file(&src2).unwrap();

        assert!(fetch_tool("multi_tool", &url1, "sha256", &sha256, None, &base_cache).is_ok());
        assert!(fetch_tool("multi_tool", &url2, "sha256", &sha256, None, &base_cache).is_ok());
    }
}





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

pub fn cached_binary_path(
    tool_name: &str,
    url: &str,
    archive_path: Option<&str>,
    cache_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(cache_dir.join(resolve_relative_bin_path(tool_name, url, archive_path)?))
}

pub fn fetch_tool(
    tool_name: &str,
    url: &str,
    digest_algorithm: &str,
    digest_value: &str,
    archive_path: Option<&str>,
    cache_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let tools_dir = match cache_dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let cache_key = format!("{}:{}", digest_algorithm, digest_value);
    let url_file = tools_dir.join(format!("{}.url", cache_key));

    let rel_bin_path = resolve_relative_bin_path(tool_name, url, archive_path)?;
    let target_path = cache_dir.join(&rel_bin_path);

    if target_path.exists() && cached_urls_contain(&url_file, url) {
        return Ok(target_path);
    }

    let archive_format = ArchiveFormat::from_url(url);

    fs::create_dir_all(tools_dir)?;

    // Download to a temporary file in the tools directory
    let mut temp_file = tempfile::NamedTempFile::new_in(tools_dir)?;
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
        let unpack_dir = tempfile::TempDir::new_in(tools_dir)?;
        let archive_file = temp_file.reopen()?;
        extract_and_prepare_archive(
            format,
            archive_file,
            tools_dir,
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
    fn test_cached_binary_path_archive_and_binary() {
        let dir = Path::new("/cache/tools/sha256:1234");
        // Non-archive
        let p1 = cached_binary_path("mytool", "https://example.com/bin", None, dir).unwrap();
        assert_eq!(p1, PathBuf::from("/cache/tools/sha256:1234/mytool"));

        // Archive with custom path
        let p2 = cached_binary_path("mytool", "https://example.com/bin.tar.gz", Some("sub/bin"), dir).unwrap();
        assert_eq!(p2, PathBuf::from("/cache/tools/sha256:1234/sub/bin"));

        // Archive with default path
        let p3 = cached_binary_path("mytool", "https://example.com/bin.tar.gz", None, dir).unwrap();
        assert_eq!(p3, PathBuf::from("/cache/tools/sha256:1234/mytool"));

        // AppImage default path
        let p4 = cached_binary_path("mytool", "https://example.com/pkg.AppImage", None, dir).unwrap();
        assert_eq!(p4, PathBuf::from("/cache/tools/sha256:1234/AppRun"));

        // Invalid archive paths
        assert!(cached_binary_path("mytool", "https://example.com/bin.tar.gz", Some("/abs/path"), dir).is_err());
        assert!(cached_binary_path("mytool", "https://example.com/bin.tar.gz", Some("sub/../bad"), dir).is_err());
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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join("sha256:incorrect_sha_hash");

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", "incorrect_sha_hash", None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha512:{}", sha512));

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_sha512_tool", &file_url, "sha512", &sha512, None, &cache_dir);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/"));

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha512:{}.url", sha512));
        assert!(url_file.exists());
        assert_eq!(fs::read_to_string(&url_file).unwrap().trim(), file_url.trim());

        // Now test mismatch with sha512
        let cache_dir_bad = fake_home.join(".cache/scrim/tools").join("sha512:bad_hash");
        let result_bad = fetch_tool("my_sha512_tool", &file_url, "sha512", "bad_hash", None, &cache_dir_bad);
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
        let cache_dir = fake_home.join(".cache/scrim/tools").join("sha256:any_sha256");

        let file_url = "file:///tmp/this_file_does_not_exist_scrim_test_12345";
        let result = fetch_tool("my_test_tool", file_url, "sha256", "any_sha256", None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool"));
        assert!(err_msg.contains("does not exist"));
    }

    #[test]
    fn test_fetch_tool_http_404_failure() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join("sha256:any_sha256");

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
        let result = fetch_tool("my_test_tool", &url, "sha256", "any_sha256", None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("404"), "Expected HTTP 404 error, got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_network_error() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join("sha256:any_sha256");

        let url = "http://127.0.0.1:1";
        let result = fetch_tool("my_test_tool", url, "sha256", "any_sha256", None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", &sha256, Some("bin/mytool"), &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", zip_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", &sha256, Some("bin/mytool"), &cache_dir);

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

    #[test]
    fn test_fetch_tool_archive_path_validation() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join("sha256:any_sha");

        let url = "https://example.com/tool.tar.gz";

        // Test absolute path
        let res_absolute = fetch_tool("my_tool", url, "sha256", "any_sha", Some("/absolute/path"), &cache_dir);
        assert!(res_absolute.is_err());
        assert!(res_absolute.err().unwrap().to_string().contains("must be a relative path"));

        // Test '..' traversal
        let res_traversal = fetch_tool("my_tool", url, "sha256", "any_sha", Some("bin/../secret"), &cache_dir);
        assert!(res_traversal.is_err());
        assert!(res_traversal.err().unwrap().to_string().contains("without '..'"));
    }

    #[test]
    fn test_fetch_tool_archive_default_path() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let zip_path = temp_path.join("mytool.zip");
        let zip_file = fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_file);
        
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
            
        zip.start_file("default_tool", options).unwrap();
        use std::io::Write;
        zip.write_all(b"echo 'default archive'").unwrap();
        zip.finish().unwrap();

        let mut file = fs::File::open(&zip_path).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha256 = format!("{:x}", hasher.finalize());

        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", zip_path.to_str().unwrap());
        let result = fetch_tool("default_tool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().ends_with("default_tool"));
    }

    #[test]
    #[cfg(unix)]
    fn test_fetch_tool_symlink_rejection() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("mytool.tar.gz");
        let tar_gz = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("/etc/passwd").unwrap();
        builder.append_data(&mut header, "mytool", &[][..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let mut file = fs::File::open(&tar_gz_path).unwrap();
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).unwrap();
        let sha256 = format!("{:x}", hasher.finalize());

        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("is a symlink outside of the extracted directory"));
    }

    #[test]
    #[cfg(unix)]
    fn test_fetch_tool_tar_gz_internal_symlink_allowed() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("mytool_symlink.tar.gz");
        let tar_gz = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let data = b"#!/bin/sh\necho 'internal symlink target'\n";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_size(data.len() as u64);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder.append_data(&mut file_header, "bin/real_tool", &data[..]).unwrap();

        let mut sym_header = tar::Header::new_gnu();
        sym_header.set_size(0);
        sym_header.set_entry_type(tar::EntryType::Symlink);
        sym_header.set_link_name("bin/real_tool").unwrap();
        sym_header.set_cksum();
        builder.append_data(&mut sym_header, "mytool", &[][..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let sha256 = format!("{:x}", Sha256::digest(fs::read(&tar_gz_path).unwrap()));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_ok(), "Internal symlink should be allowed: {:?}", result.err());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.is_symlink());

        use std::os::unix::fs::PermissionsExt;
        let real_meta = fs::metadata(cache_dir.join("bin/real_tool")).unwrap();
        assert_eq!(real_meta.permissions().mode() & 0o111, 0o111);
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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let url = format!("http://127.0.0.1:{}/binary", port);
        let result = fetch_tool("http_tool", &url, "sha256", &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let url = format!("http://[::1]:{}/binary", port);
        let result = fetch_tool("http_tool_ipv6", &url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_ok(), "Failed IPv6 fetch: {:?}", result.err());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert_eq!(fs::read(&target_path).unwrap(), content);
    }

    #[test]
    fn test_fetch_tool_http_ipv6_tar_gz_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mock_src_dir = temp_path.join("mock_src_dir_ipv6");
        fs::create_dir_all(mock_src_dir.join("bin")).unwrap();
        fs::write(mock_src_dir.join("bin").join("mytool_ipv6"), "echo 'hello ipv6 archive'").unwrap();

        let tar_gz_path = temp_path.join("mytool_ipv6.tar.gz");
        let tar_gz = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        builder.append_dir_all(".", &mock_src_dir).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let archive_bytes = fs::read(&tar_gz_path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&archive_bytes);
        let sha256 = format!("{:x}", hasher.finalize());

        let listener = match std::net::TcpListener::bind("[::1]:0") {
            Ok(l) => l,
            Err(_) => return, // Gracefully skip if IPv6 loopback is unavailable in environment
        };
        let port = listener.local_addr().unwrap().port();
        let archive_bytes_clone = archive_bytes.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf);
                let response_header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    archive_bytes_clone.len()
                );
                let _ = stream.write_all(response_header.as_bytes());
                let _ = stream.write_all(&archive_bytes_clone);
            }
        });

        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let url = format!("http://[::1]:{}/mytool_ipv6.tar.gz", port);
        let result = fetch_tool("mytool_ipv6", &url, "sha256", &sha256, Some("bin/mytool_ipv6"), &cache_dir);

        assert!(result.is_ok(), "Failed IPv6 archive fetch: {:?}", result.err());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().ends_with("bin/mytool_ipv6"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "File is not executable!");
        }
    }

    #[test]
    fn test_fetch_tool_http_ipv6_network_error() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join("sha256:any_sha256");

        // Port 1 on IPv6 loopback is not listening
        let url = "http://[::1]:1";
        let result = fetch_tool("ipv6_error_tool", url, "sha256", "any_sha256", None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool:"), "Got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_binary_not_found_in_archive() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("mytool.tar.gz");
        let tar_gz = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        
        let mut header = tar::Header::new_gnu();
        let data = b"echo wrong";
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, "wrong_binary", &data[..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let sha256 = format!("{:x}", Sha256::digest(fs::read(&tar_gz_path).unwrap()));

        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, Some("expected_binary"), &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Extracted executable not found at"));
    }

    #[test]
    fn test_fetch_tool_tgz_extension() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mock_src_dir = temp_path.join("mock_src_dir");
        fs::create_dir_all(&mock_src_dir).unwrap();
        fs::write(mock_src_dir.join("mytool"), "echo 'hello tgz'").unwrap();

        let tgz_path = temp_path.join("mytool.tgz");
        let tgz_file = fs::File::create(&tgz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tgz_file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        builder.append_dir_all(".", &mock_src_dir).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let sha256 = format!("{:x}", Sha256::digest(fs::read(&tgz_path).unwrap()));

        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tgz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().ends_with("mytool"));
    }

    #[test]
    fn test_fetch_tool_corrupt_tar_gz() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.tar.gz");
        fs::write(&corrupt_path, b"not a valid tar.gz file").unwrap();

        let sha256 = format!("{:x}", Sha256::digest(b"not a valid tar.gz file"));

        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", corrupt_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract tar.gz archive"), "Got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_corrupt_zip() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.zip");
        fs::write(&corrupt_path, b"not a valid zip file").unwrap();

        let sha256 = format!("{:x}", Sha256::digest(b"not a valid zip file"));

        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", corrupt_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to open zip archive"), "Got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_existing_cache_fast_path() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}.url", sha256));
        fs::write(&url_file, "http://invalid.invalid/never_called\n").unwrap();

        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", "sha256", sha256, None, &cache_dir);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), cached_bin);
    }

    #[test]
    fn test_fetch_tool_existing_cache_missing_url_file_downloads() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        // Without <sha256>.url, fetch_tool must not return the cached binary and must attempt download
        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", "sha256", sha256, None, &cache_dir);

        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("Failed to download tool"));
    }

    #[test]
    fn test_fetch_tool_existing_cache_mismatched_url_downloads() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}.url", sha256));
        fs::write(&url_file, "http://previously.downloaded/tool\n").unwrap();

        // With mismatched URL in <sha256>.url, fetch_tool must not use cache and must attempt download
        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", "sha256", sha256, None, &cache_dir);

        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("Failed to download tool"));
    }

    #[test]
    fn test_fetch_tool_url_updated_hash_mismatch_facilitates_update() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");

        // 1. Initial cached version (v1)
        let old_src = temp_path.join("v1_tool");
        fs::write(&old_src, b"version 1 binary").unwrap();
        let old_sha256 = format!("{:x}", Sha256::digest(b"version 1 binary"));
        let old_url = format!("file://{}", old_src.to_str().unwrap());

        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", old_sha256));
        let res_v1 = fetch_tool("my_tool", &old_url, "sha256", &old_sha256, None, &cache_dir);
        assert!(res_v1.is_ok());

        // 2. User updates URL to v2 but leaves sha256 as old_sha256
        let new_src = temp_path.join("v2_tool");
        fs::write(&new_src, b"version 2 binary with different content").unwrap();
        let new_sha256 = format!("{:x}", Sha256::digest(b"version 2 binary with different content"));
        let new_url = format!("file://{}", new_src.to_str().unwrap());

        let res_v2 = fetch_tool("my_tool", &new_url, "sha256", &old_sha256, None, &cache_dir);
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

        let content = b"identical tool payload";
        let sha256 = format!("{:x}", Sha256::digest(content));

        let src1 = temp_path.join("src1");
        fs::write(&src1, content).unwrap();
        let url1 = format!("file://{}", src1.to_str().unwrap());

        let src2 = temp_path.join("src2");
        fs::write(&src2, content).unwrap();
        let url2 = format!("file://{}", src2.to_str().unwrap());

        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        // Fetch from url1
        let res1 = fetch_tool("multi_tool", &url1, "sha256", &sha256, None, &cache_dir);
        assert!(res1.is_ok());

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}.url", sha256));
        assert!(url_file.exists());
        let file_content = fs::read_to_string(&url_file).unwrap();
        assert!(file_content.contains(&url1));

        // Fetch from url2 (mirror)
        let res2 = fetch_tool("multi_tool", &url2, "sha256", &sha256, None, &cache_dir);
        assert!(res2.is_ok());

        let updated_content = fs::read_to_string(&url_file).unwrap();
        assert!(updated_content.contains(&url1));
        assert!(updated_content.contains(&url2));

        // Now remove src1 and src2 from disk to guarantee both URLs hit the cache without downloading
        fs::remove_file(&src1).unwrap();
        fs::remove_file(&src2).unwrap();

        assert!(fetch_tool("multi_tool", &url1, "sha256", &sha256, None, &cache_dir).is_ok());
        assert!(fetch_tool("multi_tool", &url2, "sha256", &sha256, None, &cache_dir).is_ok());
    }

    #[test]
    fn test_fetch_tool_tar_xz_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            let content = b"echo 'hello tar xz'";
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, "bin/mytool", &content[..]).unwrap();
            builder.finish().unwrap();
        }
        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz_bytes).unwrap();

        let tar_xz_path = temp_path.join("mytool.tar.xz");
        fs::write(&tar_xz_path, &xz_bytes).unwrap();

        let sha256 = format!("{:x}", Sha256::digest(&xz_bytes));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tar_xz_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "sha256", &sha256, Some("bin/mytool"), &cache_dir);

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
    fn test_fetch_tool_txz_extension() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            let content = b"echo 'hello txz'";
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, "mytool", &content[..]).unwrap();
            builder.finish().unwrap();
        }
        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz_bytes).unwrap();

        let txz_path = temp_path.join("mytool.txz");
        fs::write(&txz_path, &xz_bytes).unwrap();

        let sha256 = format!("{:x}", Sha256::digest(&xz_bytes));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", txz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().ends_with("mytool"));
    }

    #[test]
    fn test_fetch_tool_xz_extension() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            let content = b"echo 'hello xz'";
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, "bin/mytool", &content[..]).unwrap();
            builder.finish().unwrap();
        }
        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz_bytes).unwrap();

        let xz_path = temp_path.join("mytool.xz");
        fs::write(&xz_path, &xz_bytes).unwrap();

        let sha256 = format!("{:x}", Sha256::digest(&xz_bytes));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", xz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, Some("bin/mytool"), &cache_dir);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().ends_with("bin/mytool"));
    }

    #[test]
    fn test_fetch_tool_corrupt_xz() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.tar.xz");
        fs::write(&corrupt_path, b"not a valid xz file").unwrap();

        let sha256 = format!("{:x}", Sha256::digest(b"not a valid xz file"));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", corrupt_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract xz archive"), "Got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_corrupt_tar_in_xz() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &b"not a tarball"[..], &mut xz_bytes).unwrap();

        let corrupt_tar_path = temp_path.join("corrupt_tar.tar.xz");
        fs::write(&corrupt_tar_path, &xz_bytes).unwrap();

        let sha256 = format!("{:x}", Sha256::digest(&xz_bytes));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", corrupt_tar_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract tar.xz archive"), "Got: {}", err_msg);
    }

    #[test]
    #[cfg(unix)]
    fn test_fetch_tool_tar_xz_symlink_rejection() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("/etc/passwd").unwrap();
            builder.append_data(&mut header, "mytool", &[][..]).unwrap();
            builder.finish().unwrap();
        }
        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz_bytes).unwrap();

        let tar_xz_path = temp_path.join("mytool.tar.xz");
        fs::write(&tar_xz_path, &xz_bytes).unwrap();

        let sha256 = format!("{:x}", Sha256::digest(&xz_bytes));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tar_xz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("is a symlink outside of the extracted directory"));
    }

    #[test]
    #[cfg(unix)]
    fn test_fetch_tool_unpack_filesystem_error() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("conflict.tar.gz");
        let tar_gz = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        // 1. Create a regular file entry named "conflict"
        let mut file_header1 = tar::Header::new_gnu();
        let data1 = b"regular file";
        file_header1.set_size(data1.len() as u64);
        file_header1.set_mode(0o644);
        file_header1.set_cksum();
        builder.append_data(&mut file_header1, "conflict", &data1[..]).unwrap();

        // 2. Create a child file inside "conflict" - OS fails with ENOTDIR (Not a directory)
        let mut file_header2 = tar::Header::new_gnu();
        let data2 = b"blocked content";
        file_header2.set_size(data2.len() as u64);
        file_header2.set_mode(0o644);
        file_header2.set_cksum();
        builder.append_data(&mut file_header2, "conflict/child.txt", &data2[..]).unwrap();

        builder.into_inner().unwrap().finish().unwrap();

        let sha256 = format!("{:x}", Sha256::digest(fs::read(&tar_gz_path).unwrap()));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("conflict_tool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract tar.gz archive"), "Got: {}", err_msg);
        assert!(err_msg.contains("failed to unpack"), "Got: {}", err_msg);
        assert!(
            err_msg.contains("Not a directory") || err_msg.contains("os error 20"),
            "Underlying filesystem error was not displayed! Got: {}",
            err_msg
        );
    }

    #[test]
    fn test_fetch_tool_appimage_default_apprun_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let appimage_path = temp_path.join("mytool.AppImage");
        test_helpers::create_fake_appimage(
            &appimage_path,
            &[
                ("AppRun", b"#!/bin/sh\necho 'hello from AppRun'\n", 0o755),
                ("usr/bin/mytool", b"#!/bin/sh\necho 'hello from usr/bin/mytool'\n", 0o755),
            ],
            &[],
        );

        let sha256 = format!("{:x}", Sha256::digest(fs::read(&appimage_path).unwrap()));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", appimage_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_ok(), "Failed to fetch AppImage: {:?}", result.err());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.ends_with("AppRun"), "Expected target_path to end with AppRun, got: {}", target_path.display());
        assert!(cache_dir.join("usr/bin/mytool").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "AppRun is not executable!");
        }
    }

    #[test]
    fn test_fetch_tool_appimage_lowercase_extension_and_custom_archive_path() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let appimage_path = temp_path.join("mytool.appimage");
        test_helpers::create_fake_appimage(
            &appimage_path,
            &[
                ("AppRun", b"#!/bin/sh\necho 'hello from AppRun'\n", 0o755),
                ("usr/bin/custom_bin", b"#!/bin/sh\necho 'hello from custom_bin'\n", 0o644),
            ],
            &[("usr/bin/link_to_bin", "custom_bin")],
        );

        let sha256 = format!("{:x}", Sha256::digest(fs::read(&appimage_path).unwrap()));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}?download=1", appimage_path.to_str().unwrap());
        let result = fetch_tool(
            "mytool",
            &file_url,
            "sha256",
            &sha256,
            Some("usr/bin/custom_bin"),
            &cache_dir,
        );

        assert!(result.is_ok(), "Failed to fetch .appimage: {:?}", result.err());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.ends_with("usr/bin/custom_bin"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "Target binary is not executable!");
            assert!(cache_dir.join("usr/bin/link_to_bin").is_symlink());
        }
    }

    #[test]
    fn test_fetch_tool_appimage_corrupt_error_propagated() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.AppImage");
        let data = b"not an ELF file at all";
        fs::write(&corrupt_path, data).unwrap();

        let sha256 = format!("{:x}", Sha256::digest(data));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", corrupt_path.to_str().unwrap());
        let result = fetch_tool("corrupt", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract AppImage"), "Got: {}", err_msg);
    }

    #[test]
    #[cfg(unix)]
    fn test_fetch_tool_appimage_relative_symlink_apprun_succeeds() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let appimage_path = temp_path.join("symlink_entry.AppImage");
        test_helpers::create_fake_appimage(
            &appimage_path,
            &[("usr/bin/real_bin", b"#!/bin/sh\necho 'real'\n", 0o644)],
            &[("AppRun", "usr/bin/real_bin")],
        );

        let sha256 = format!("{:x}", Sha256::digest(fs::read(&appimage_path).unwrap()));
        let fake_home = temp_path.join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join(format!("sha256:{}", sha256));

        let file_url = format!("file://{}", appimage_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, "sha256", &sha256, None, &cache_dir);

        assert!(result.is_ok(), "Expected relative AppRun symlink to succeed, got: {:?}", result.err());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.is_symlink());
        assert!(target_path.ends_with("AppRun"));

        use std::os::unix::fs::PermissionsExt;
        let real_bin_meta = fs::metadata(cache_dir.join("usr/bin/real_bin")).unwrap();
        assert_eq!(real_bin_meta.permissions().mode() & 0o111, 0o111, "Resolved target must be marked executable");
    }
}




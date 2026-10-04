use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self, Seek, Write};
use sha2::{Sha256, Digest};
use flate2::read::GzDecoder;
use tar::Archive;
use zip::ZipArchive;

struct HashingWriter<W, D> {
    writer: W,
    hasher: D,
}

impl<W: Write, D: Digest> Write for HashingWriter<W, D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.writer.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

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

pub fn fetch_tool(tool_name: &str, url: &str, sha256: &str, archive_path: Option<&str>, cache_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let tools_dir = match cache_dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let url_file = tools_dir.join(format!("{}.url", sha256));
    
    // Ignore query parameters when detecting archive types
    let url_path = url.split('?').next().unwrap_or(url);
    let is_tar_gz = url_path.ends_with(".tar.gz") || url_path.ends_with(".tgz");
    let is_tar_xz = url_path.ends_with(".tar.xz") || url_path.ends_with(".txz") || url_path.ends_with(".xz");
    let is_zip = url_path.ends_with(".zip");
    let is_archive = is_tar_gz || is_tar_xz || is_zip;

    let target_path = if is_archive {
        if let Some(bin) = archive_path {
            let bin_path = Path::new(bin);
            if bin_path.is_absolute() || bin.contains("..") {
                return Err(format!("Invalid archive_path: {} must be a relative path without '..'", bin).into());
            }
            cache_dir.join(bin)
        } else {
            cache_dir.join(tool_name)
        }
    } else {
        cache_dir.join(tool_name)
    };

    if target_path.exists() && cached_urls_contain(&url_file, url) {
        return Ok(target_path);
    }

    fs::create_dir_all(tools_dir)?;

    // Download to a temporary file in the tools directory
    let mut temp_file = tempfile::NamedTempFile::new_in(tools_dir)?;
    let mut hw = HashingWriter { writer: &mut temp_file, hasher: Sha256::new() };

    if let Some(local_path) = url.strip_prefix("file://") {
        let local_path = local_path.split('?').next().unwrap_or(local_path);
        if !Path::new(local_path).exists() {
            return Err(format!("Failed to download tool: local file {} does not exist", local_path).into());
        }
        let mut file = File::open(local_path)?;
        io::copy(&mut file, &mut hw)?;
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
        io::copy(&mut reader, &mut hw)?;
    }

    let computed_sha256 = format!("{:x}", hw.hasher.finalize());

    if computed_sha256 != sha256 {
        let err_msg = format!("SHA256 verification failed for {}.\nExpected: {}\nActual:   {}", tool_name, sha256, computed_sha256);
        return Err(err_msg.into());
    }

    if is_archive {
        crate::scrim_progress!("Unpacking {} archive...", tool_name);
        
        // Extract to a TempDir first to ensure atomicity
        let unpack_dir = tempfile::TempDir::new_in(tools_dir)?;
        
        if is_tar_gz {
            let tar_gz = temp_file.reopen()?;
            let tar = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(tar);
            if let Err(e) = archive.unpack(unpack_dir.path()) {
                return Err(format!("Failed to extract tar.gz archive: {}", e).into());
            }
        } else if is_tar_xz {
            let tar_xz = temp_file.reopen()?;
            let mut reader = std::io::BufReader::new(tar_xz);
            let mut decompressed_tar = tempfile::tempfile_in(tools_dir)?;
            if let Err(e) = lzma_rs::xz_decompress(&mut reader, &mut decompressed_tar) {
                return Err(format!("Failed to extract xz archive: {}", e).into());
            }
            decompressed_tar.rewind()?;
            let mut archive = Archive::new(decompressed_tar);
            if let Err(e) = archive.unpack(unpack_dir.path()) {
                return Err(format!("Failed to extract tar.xz archive: {}", e).into());
            }
        } else if is_zip {
            let zip_file = temp_file.reopen()?;
            let mut archive = match ZipArchive::new(zip_file) {
                Ok(a) => a,
                Err(e) => {
                    return Err(format!("Failed to open zip archive: {}", e).into());
                }
            };
            if let Err(e) = archive.extract(unpack_dir.path()) {
                return Err(format!("Failed to extract zip archive: {}", e).into());
            }
        }
        
        let temp_target_path = if let Some(bin) = archive_path {
            let bin_path = Path::new(bin);
            if bin_path.is_absolute() || bin.contains("..") {
                return Err(format!("Invalid archive_path: {} must be a relative path without '..'", bin).into());
            }
            unpack_dir.path().join(bin)
        } else {
            unpack_dir.path().join(tool_name)
        };

        if !temp_target_path.exists() {
            return Err(format!("Extracted executable not found at {}", temp_target_path.display()).into());
        }

        // Ensure the binary is executable BEFORE exposing it
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::symlink_metadata(&temp_target_path)?;
            if metadata.file_type().is_symlink() {
                return Err(format!("Extracted executable {} is a symlink, which is not allowed", temp_target_path.display()).into());
            }
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&temp_target_path, perms)?;
        }
        
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
    use tempfile::tempdir;

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, &sha256, None, &cache_dir);

        assert!(result.is_ok());
        let target_path = result.unwrap();
        assert!(target_path.exists());
        assert!(target_path.to_str().unwrap().contains("fake_home/.cache/scrim/tools/"));
        assert!(target_path.to_str().unwrap().ends_with("my_test_tool"));

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("{}.url", sha256));
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
        let cache_dir = fake_home.join(".cache/scrim/tools").join("incorrect_sha_hash");

        let file_url = format!("file://{}", mock_src.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, "incorrect_sha_hash", None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join("any_sha256");

        let file_url = "file:///tmp/this_file_does_not_exist_scrim_test_12345";
        let result = fetch_tool("my_test_tool", file_url, "any_sha256", None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to download tool"));
        assert!(err_msg.contains("does not exist"));
    }

    #[test]
    fn test_fetch_tool_http_404_failure() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join("any_sha256");

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
        let result = fetch_tool("my_test_tool", &url, "any_sha256", None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("404"), "Expected HTTP 404 error, got: {}", err_msg);
    }

    #[test]
    fn test_fetch_tool_network_error() {
        let temp = tempdir().unwrap();
        let fake_home = temp.path().join("fake_home");
        let cache_dir = fake_home.join(".cache/scrim/tools").join("any_sha256");

        let url = "http://127.0.0.1:1";
        let result = fetch_tool("my_test_tool", url, "any_sha256", None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, &sha256, Some("bin/mytool"), &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", zip_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, &sha256, Some("bin/mytool"), &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join("any_sha");

        let url = "https://example.com/tool.tar.gz";

        // Test absolute path
        let res_absolute = fetch_tool("my_tool", url, "any_sha", Some("/absolute/path"), &cache_dir);
        assert!(res_absolute.is_err());
        assert!(res_absolute.err().unwrap().to_string().contains("must be a relative path"));

        // Test '..' traversal
        let res_traversal = fetch_tool("my_tool", url, "any_sha", Some("bin/../secret"), &cache_dir);
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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", zip_path.to_str().unwrap());
        let result = fetch_tool("default_tool", &file_url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("is a symlink, which is not allowed"));
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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let url = format!("http://127.0.0.1:{}/binary", port);
        let result = fetch_tool("http_tool", &url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let url = format!("http://[::1]:{}/binary", port);
        let result = fetch_tool("http_tool_ipv6", &url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let url = format!("http://[::1]:{}/mytool_ipv6.tar.gz", port);
        let result = fetch_tool("mytool_ipv6", &url, &sha256, Some("bin/mytool_ipv6"), &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join("any_sha256");

        // Port 1 on IPv6 loopback is not listening
        let url = "http://[::1]:1";
        let result = fetch_tool("ipv6_error_tool", url, "any_sha256", None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", tar_gz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, &sha256, Some("expected_binary"), &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", tgz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", corrupt_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", corrupt_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(sha256);
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("{}.url", sha256));
        fs::write(&url_file, "http://invalid.invalid/never_called\n").unwrap();

        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", sha256, None, &cache_dir);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), cached_bin);
    }

    #[test]
    fn test_fetch_tool_existing_cache_missing_url_file_downloads() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = fake_home.join(".cache/scrim/tools").join(sha256);
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        // Without <sha256>.url, fetch_tool must not return the cached binary and must attempt download
        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", sha256, None, &cache_dir);

        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("Failed to download tool"));
    }

    #[test]
    fn test_fetch_tool_existing_cache_mismatched_url_downloads() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();
        let fake_home = temp_path.join("fake_home");

        let sha256 = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let cache_dir = fake_home.join(".cache/scrim/tools").join(sha256);
        fs::create_dir_all(&cache_dir).unwrap();
        let cached_bin = cache_dir.join("cached_tool");
        fs::write(&cached_bin, b"already cached").unwrap();

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("{}.url", sha256));
        fs::write(&url_file, "http://previously.downloaded/tool\n").unwrap();

        // With mismatched URL in <sha256>.url, fetch_tool must not use cache and must attempt download
        let result = fetch_tool("cached_tool", "http://invalid.invalid/never_called", sha256, None, &cache_dir);

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

        let cache_dir = fake_home.join(".cache/scrim/tools").join(&old_sha256);
        let res_v1 = fetch_tool("my_tool", &old_url, &old_sha256, None, &cache_dir);
        assert!(res_v1.is_ok());

        // 2. User updates URL to v2 but leaves sha256 as old_sha256
        let new_src = temp_path.join("v2_tool");
        fs::write(&new_src, b"version 2 binary with different content").unwrap();
        let new_sha256 = format!("{:x}", Sha256::digest(b"version 2 binary with different content"));
        let new_url = format!("file://{}", new_src.to_str().unwrap());

        let res_v2 = fetch_tool("my_tool", &new_url, &old_sha256, None, &cache_dir);
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

        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        // Fetch from url1
        let res1 = fetch_tool("multi_tool", &url1, &sha256, None, &cache_dir);
        assert!(res1.is_ok());

        let url_file = fake_home.join(".cache/scrim/tools").join(format!("{}.url", sha256));
        assert!(url_file.exists());
        let file_content = fs::read_to_string(&url_file).unwrap();
        assert!(file_content.contains(&url1));

        // Fetch from url2 (mirror)
        let res2 = fetch_tool("multi_tool", &url2, &sha256, None, &cache_dir);
        assert!(res2.is_ok());

        let updated_content = fs::read_to_string(&url_file).unwrap();
        assert!(updated_content.contains(&url1));
        assert!(updated_content.contains(&url2));

        // Now remove src1 and src2 from disk to guarantee both URLs hit the cache without downloading
        fs::remove_file(&src1).unwrap();
        fs::remove_file(&src2).unwrap();

        assert!(fetch_tool("multi_tool", &url1, &sha256, None, &cache_dir).is_ok());
        assert!(fetch_tool("multi_tool", &url2, &sha256, None, &cache_dir).is_ok());
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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", tar_xz_path.to_str().unwrap());
        let result = fetch_tool("my_test_tool", &file_url, &sha256, Some("bin/mytool"), &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", txz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", xz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, &sha256, Some("bin/mytool"), &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", corrupt_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", corrupt_tar_path.to_str().unwrap());
        let result = fetch_tool("corrupt_tool", &file_url, &sha256, None, &cache_dir);

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
        let cache_dir = fake_home.join(".cache/scrim/tools").join(&sha256);

        let file_url = format!("file://{}", tar_xz_path.to_str().unwrap());
        let result = fetch_tool("mytool", &file_url, &sha256, None, &cache_dir);

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("is a symlink, which is not allowed"));
    }
}


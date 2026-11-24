use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self, Write};
use archive::{ArchiveFormat, extract_and_prepare_archive, resolve_relative_bin_path};
use hashing::HashingWriter;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

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

// Public so the standalone `fetcher_test` crate can test cache layout directly;
// package visibility in BUILD.bazel still restricts external callers.
pub fn tool_cache_dir(base_cache_dir: &Path, digest_algorithm: &str, digest_value: &str) -> PathBuf {
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
        let agent: Agent = Agent::config_builder()
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        let response = match agent.get(url).call() {
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


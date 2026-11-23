use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use bincode::Options;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CACHE_MAGIC: &[u8; 4] = b"SCMC";
pub const CACHE_SCHEMA_VERSION: u32 = 1;

pub fn clean_commit_hash(commit: Option<&str>) -> Option<String> {
    commit
        .map(str::trim)
        .filter(|h| !h.is_empty() && !h.starts_with('{'))
        .map(String::from)
}

pub fn cache_file_name_for_path(canonical_path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_path.as_os_str().as_bytes());
    let hash = hasher.finalize();
    format!("{:x}.bin", hash)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CachedImportPrecedence {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedImportEntry {
    pub path: String,
    pub optional: bool,
    pub precedence: CachedImportPrecedence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedFileTelemetryConfig {
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CachedTelemetryConfig {
    pub file: Vec<CachedFileTelemetryConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CachedPlatformToolConfig {
    pub system_path: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>,
    pub archive_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CachedToolConfig {
    pub template: Option<String>,
    pub archive_path: Option<String>,
    pub platforms: BTreeMap<String, CachedPlatformToolConfig>,
    pub system_path: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>,
    pub desugar_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CachedFileConfig {
    pub telemetry: CachedTelemetryConfig,
    pub imports: Vec<CachedImportEntry>,
    pub tools: BTreeMap<String, CachedToolConfig>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheHeader {
    pub magic: [u8; 4],
    pub schema_version: u32,
    pub scrim_version: String,
    pub commit_hash: Option<String>,
    pub source_mtime_secs: u64,
    pub source_mtime_nanos: u32,
    pub source_size: u64,
}

#[derive(Debug, Clone)]
pub struct PendingCacheWrite {
    pub cache_file_path: PathBuf,
    pub source_mtime_secs: u64,
    pub source_mtime_nanos: u32,
    pub source_size: u64,
    pub cached_config: CachedFileConfig,
    pub commit_hash: Option<String>,
}

impl PendingCacheWrite {
    pub fn persist(&self) {
        let _ = self.persist_internal();
    }

    fn persist_internal(&self) -> Result<(), Box<dyn std::error::Error>> {
        let header = CacheHeader {
            magic: *CACHE_MAGIC,
            schema_version: CACHE_SCHEMA_VERSION,
            scrim_version: version::VERSION.to_string(),
            commit_hash: self.commit_hash.clone(),
            source_mtime_secs: self.source_mtime_secs,
            source_mtime_nanos: self.source_mtime_nanos,
            source_size: self.source_size,
        };
        let mut buffer = Vec::new();
        let bincode_opts = bincode::DefaultOptions::new().with_fixint_encoding();
        bincode_opts.serialize_into(&mut buffer, &header)?;
        bincode_opts.serialize_into(&mut buffer, &self.cached_config)?;
        atomic_write_cache_file(&self.cache_file_path, &buffer)?;
        Ok(())
    }
}

pub fn atomic_write_cache_file(target_path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = match target_path.parent() {
        Some(p) => p,
        None => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No parent directory")),
    };
    fs::create_dir_all(parent)?;

    let pid = std::process::id();
    let thread_id = format!("{:?}", std::thread::current().id());
    let thread_id_clean: String = thread_id
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let temp_name = format!(
        ".{}.tmp.{}.{}",
        target_path.file_name().and_then(|n| n.to_str()).unwrap_or("cache"),
        pid,
        thread_id_clean
    );
    let temp_path = parent.join(temp_name);

    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.flush()?;
    }

    if let Err(e) = fs::rename(&temp_path, target_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
    }
    Ok(())
}

pub fn try_read_cache(
    cache_file_path: &Path,
    source_mtime_secs: u64,
    source_mtime_nanos: u32,
    source_size: u64,
    current_commit: Option<&str>,
) -> Option<CachedFileConfig> {
    let bytes = fs::read(cache_file_path).ok()?;
    let limit = bytes.len() as u64;
    let bincode_opts = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(limit);

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut cursor = Cursor::new(&bytes);
        let header: CacheHeader = bincode_opts.deserialize_from(&mut cursor).ok()?;
        if header.magic != *CACHE_MAGIC {
            return None;
        }
        if header.schema_version != CACHE_SCHEMA_VERSION {
            return None;
        }
        if header.scrim_version != version::VERSION {
            return None;
        }
        let clean_current = clean_commit_hash(current_commit);
        if header.commit_hash != clean_current {
            return None;
        }
        if header.source_mtime_secs != source_mtime_secs
            || header.source_mtime_nanos != source_mtime_nanos
            || header.source_size != source_size
        {
            return None;
        }

        let cached_config: CachedFileConfig = bincode_opts.deserialize_from(&mut cursor).ok()?;
        Some(cached_config)
    }))
    .ok()?
}

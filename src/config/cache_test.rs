use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use bincode::Options;
use tempfile::tempdir;

use cache::{
    atomic_write_cache_file, cache_file_name_for_path, clean_commit_hash, try_read_cache,
    CacheHeader, CachedFileConfig, CachedImportEntry, CachedImportPrecedence,
    CachedPlatformToolConfig, CachedTelemetryConfig, CachedToolConfig, PendingCacheWrite,
    CACHE_MAGIC, CACHE_SCHEMA_VERSION,
};
use version::VERSION;

fn make_sample_cached_config() -> CachedFileConfig {
    let mut platforms = BTreeMap::new();
    platforms.insert(
        "linux-x86_64".to_string(),
        CachedPlatformToolConfig {
            system_path: Some("/usr/bin/node".to_string()),
            url: None,
            sha256: None,
            sha512: None,
            archive_path: None,
        },
    );

    let mut tools = BTreeMap::new();
    tools.insert(
        "node".to_string(),
        CachedToolConfig {
            template: None,
            archive_path: None,
            platforms,
            system_path: None,
            url: None,
            sha256: None,
            sha512: None,
            desugar_error: None,
        },
    );

    CachedFileConfig {
        telemetry: CachedTelemetryConfig::default(),
        imports: vec![CachedImportEntry {
            path: "sub.yaml".to_string(),
            optional: false,
            precedence: CachedImportPrecedence::Before,
        }],
        tools,
        warnings: vec!["sample warning".to_string()],
    }
}

#[test]
fn test_clean_commit_hash() {
    assert_eq!(clean_commit_hash(None), None);
    assert_eq!(clean_commit_hash(Some("")), None);
    assert_eq!(clean_commit_hash(Some("   ")), None);
    assert_eq!(clean_commit_hash(Some("{STABLE_GIT_COMMIT}")), None);
    assert_eq!(clean_commit_hash(Some("{FOO}")), None);
    assert_eq!(
        clean_commit_hash(Some("  abcdef123456  ")),
        Some("abcdef123456".to_string())
    );
}

#[test]
fn test_cache_file_name_for_path() {
    let p1 = Path::new("/etc/scrim/scrim.yaml");
    let p2 = Path::new("/home/user/.config/scrim/scrim.yaml");

    let name1 = cache_file_name_for_path(p1);
    let name2 = cache_file_name_for_path(p2);

    assert!(name1.ends_with(".bin"));
    assert_eq!(name1.len(), 64 + 4);
    assert_eq!(name1, cache_file_name_for_path(p1)); // deterministic
    assert_ne!(name1, name2);
}

#[test]
fn test_header_validation_and_roundtrip() {
    let temp = tempdir().unwrap();
    let cache_file = temp.path().join("test_entry.bin");

    let sample = make_sample_cached_config();
    let pending = PendingCacheWrite {
        cache_file_path: cache_file.clone(),
        source_mtime_secs: 1234567890,
        source_mtime_nanos: 456789,
        source_size: 1024,
        cached_config: sample.clone(),
        commit_hash: Some("commit_123".to_string()),
    };
    pending.persist();

    assert!(cache_file.exists());

    // Valid read matches exact parameters
    let read = try_read_cache(&cache_file, 1234567890, 456789, 1024, Some("commit_123"));
    assert_eq!(read, Some(sample));
}

#[test]
fn test_try_read_cache_magic_mismatch() {
    let temp = tempdir().unwrap();
    let cache_file = temp.path().join("bad_magic.bin");

    let header = CacheHeader {
        magic: *b"BADM",
        schema_version: CACHE_SCHEMA_VERSION,
        scrim_version: VERSION.to_string(),
        commit_hash: Some("c1".to_string()),
        source_mtime_secs: 100,
        source_mtime_nanos: 200,
        source_size: 300,
    };
    let mut bytes = Vec::new();
    let bincode_opts = bincode::DefaultOptions::new().with_fixint_encoding();
    bincode_opts.serialize_into(&mut bytes, &header).unwrap();
    bincode_opts
        .serialize_into(&mut bytes, &make_sample_cached_config())
        .unwrap();
    fs::write(&cache_file, &bytes).unwrap();

    let res = try_read_cache(&cache_file, 100, 200, 300, Some("c1"));
    assert_eq!(res, None);
}

#[test]
fn test_try_read_cache_schema_version_mismatch() {
    let temp = tempdir().unwrap();
    let cache_file = temp.path().join("bad_schema.bin");

    let header = CacheHeader {
        magic: *CACHE_MAGIC,
        schema_version: CACHE_SCHEMA_VERSION + 99,
        scrim_version: VERSION.to_string(),
        commit_hash: Some("c1".to_string()),
        source_mtime_secs: 100,
        source_mtime_nanos: 200,
        source_size: 300,
    };
    let mut bytes = Vec::new();
    let bincode_opts = bincode::DefaultOptions::new().with_fixint_encoding();
    bincode_opts.serialize_into(&mut bytes, &header).unwrap();
    bincode_opts
        .serialize_into(&mut bytes, &make_sample_cached_config())
        .unwrap();
    fs::write(&cache_file, &bytes).unwrap();

    let res = try_read_cache(&cache_file, 100, 200, 300, Some("c1"));
    assert_eq!(res, None);
}

#[test]
fn test_try_read_cache_scrim_version_mismatch() {
    let temp = tempdir().unwrap();
    let cache_file = temp.path().join("bad_version.bin");

    let header = CacheHeader {
        magic: *CACHE_MAGIC,
        schema_version: CACHE_SCHEMA_VERSION,
        scrim_version: "99.99.99".to_string(),
        commit_hash: Some("c1".to_string()),
        source_mtime_secs: 100,
        source_mtime_nanos: 200,
        source_size: 300,
    };
    let mut bytes = Vec::new();
    let bincode_opts = bincode::DefaultOptions::new().with_fixint_encoding();
    bincode_opts.serialize_into(&mut bytes, &header).unwrap();
    bincode_opts
        .serialize_into(&mut bytes, &make_sample_cached_config())
        .unwrap();
    fs::write(&cache_file, &bytes).unwrap();

    let res = try_read_cache(&cache_file, 100, 200, 300, Some("c1"));
    assert_eq!(res, None);
}

#[test]
fn test_try_read_cache_commit_hash_mismatch() {
    let temp = tempdir().unwrap();
    let cache_file = temp.path().join("test_commit.bin");

    let pending = PendingCacheWrite {
        cache_file_path: cache_file.clone(),
        source_mtime_secs: 100,
        source_mtime_nanos: 200,
        source_size: 300,
        cached_config: make_sample_cached_config(),
        commit_hash: Some("commit_original".to_string()),
    };
    pending.persist();

    // Matching commit
    assert!(try_read_cache(&cache_file, 100, 200, 300, Some("commit_original")).is_some());

    // Mismatched commit
    assert_eq!(
        try_read_cache(&cache_file, 100, 200, 300, Some("commit_modified")),
        None
    );

    // Unstamped / None
    assert_eq!(try_read_cache(&cache_file, 100, 200, 300, None), None);
}

#[test]
fn test_try_read_cache_stat_mismatch() {
    let temp = tempdir().unwrap();
    let cache_file = temp.path().join("test_stat.bin");

    let pending = PendingCacheWrite {
        cache_file_path: cache_file.clone(),
        source_mtime_secs: 100,
        source_mtime_nanos: 200,
        source_size: 300,
        cached_config: make_sample_cached_config(),
        commit_hash: None,
    };
    pending.persist();

    // mtime secs mismatch
    assert_eq!(try_read_cache(&cache_file, 101, 200, 300, None), None);

    // mtime nanos mismatch
    assert_eq!(try_read_cache(&cache_file, 100, 201, 300, None), None);

    // size mismatch
    assert_eq!(try_read_cache(&cache_file, 100, 200, 301, None), None);

    // all match
    assert!(try_read_cache(&cache_file, 100, 200, 300, None).is_some());
}

#[test]
fn test_try_read_cache_corrupted_data_and_memory_limit() {
    let temp = tempdir().unwrap();
    let cache_file = temp.path().join("corrupt.bin");

    // Random non-bincode bytes
    fs::write(&cache_file, b"arbitrary corrupted non-bincode payload").unwrap();
    assert_eq!(try_read_cache(&cache_file, 100, 200, 300, None), None);

    // Truncated file (e.g. 2 bytes)
    fs::write(&cache_file, b"SC").unwrap();
    assert_eq!(try_read_cache(&cache_file, 100, 200, 300, None), None);

    // Non-existent file
    assert_eq!(
        try_read_cache(&temp.path().join("non_existent.bin"), 100, 200, 300, None),
        None
    );

    // Valid header followed by an exorbitant container length prefix (u64::MAX)
    // Ensures bincode with_limit fails open without attempting a huge allocation
    let header = CacheHeader {
        magic: *CACHE_MAGIC,
        schema_version: CACHE_SCHEMA_VERSION,
        scrim_version: VERSION.to_string(),
        commit_hash: None,
        source_mtime_secs: 100,
        source_mtime_nanos: 200,
        source_size: 300,
    };
    let mut bytes = Vec::new();
    let bincode_opts = bincode::DefaultOptions::new().with_fixint_encoding();
    bincode_opts.serialize_into(&mut bytes, &header).unwrap();
    bytes.extend_from_slice(&u64::MAX.to_le_bytes());
    fs::write(&cache_file, &bytes).unwrap();
    assert_eq!(try_read_cache(&cache_file, 100, 200, 300, None), None);
}

#[test]
fn test_atomic_write_cache_file_success_and_cleanup() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("config/final_entry.bin");

    assert!(!target.exists());
    let data = b"binary cache payload content";
    atomic_write_cache_file(&target, data).unwrap();

    assert!(target.exists());
    assert_eq!(fs::read(&target).unwrap(), data);

    // Verify parent directory contains only the target file (no leaked temporary files)
    let parent = target.parent().unwrap();
    let files: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    assert_eq!(files, vec![target.file_name().unwrap()]);
}

#[test]
fn test_atomic_write_cache_file_unwritable_location_fails_open() {
    let temp = tempdir().unwrap();
    let blocker = temp.path().join("blocker_file");
    fs::write(&blocker, b"not a dir").unwrap();

    let target = blocker.join("cache.bin");
    let err = atomic_write_cache_file(&target, b"test");
    assert!(err.is_err());
}

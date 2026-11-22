use config::Config;
use linker::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use config::ToolConfig;
use std::collections::BTreeMap;
use tempfile::tempdir;

fn mock_host_tool() -> ToolConfig {
    let mut platforms = BTreeMap::new();
    platforms.insert(
        "any".to_string(),
        config::PlatformToolConfig {
            system_path: Some("/bin/echo".to_string()),
            ..Default::default()
        },
    );
    ToolConfig {
        platforms,
        ..Default::default()
    }
}

#[test]
fn test_path_relative_from_same_dir() {
    let target = Path::new("/usr/local/bin/scrim");
    let base = Path::new("/usr/local/bin");
    assert_eq!(path_relative_from(target, base), Path::new("scrim"));
}

#[test]
fn test_path_relative_from_nested() {
    let target = Path::new("/opt/scrim/bin/scrim");
    let base = Path::new("/usr/local/bin");
    assert_eq!(
        path_relative_from(target, base),
        Path::new("../../../opt/scrim/bin/scrim")
    );
}

#[test]
fn test_create_links_target_not_dir() {
    let dir = tempdir().unwrap();
    let nonexistent = dir.path().join("nonexistent");
    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy").unwrap();

    let config = Config::default();
    let res = create_links(&nonexistent, &scrim_exe, &config, false, false);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("does not exist"));
}

#[test]
fn test_create_links_creates_and_cleans_up() {
    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_exe").unwrap();

    // Create an existing stale scrim link
    let stale_link = bin_dir.join("oldtool");
    symlink(Path::new("../scrim"), &stale_link).unwrap();

    // Create an unrelated regular file
    let other_file = bin_dir.join("unrelated");
    fs::write(&other_file, "content").unwrap();

    let mut tools = BTreeMap::new();
    tools.insert("node".to_string(), mock_host_tool());
    tools.insert("go".to_string(), mock_host_tool());
    let config = Config {
        tools,
        ..Default::default()
    };

    // Without prune: links node and go, but does NOT remove stale_link
    let modified = create_links(&bin_dir, &scrim_exe, &config, false, false).unwrap();
    assert!(modified);

    // Check node link
    let node_link = bin_dir.join("node");
    assert!(node_link.exists());
    assert_eq!(fs::read_link(&node_link).unwrap(), Path::new("../scrim"));

    // Check go link
    let go_link = bin_dir.join("go");
    assert!(go_link.exists());
    assert_eq!(fs::read_link(&go_link).unwrap(), Path::new("../scrim"));

    // Check oldtool (stale scrim link) was NOT removed without prune
    assert!(stale_link.exists());

    // Check unrelated file remains untouched
    assert!(other_file.exists());

    // Calling create_links again without prune should succeed and return modified = false
    let modified_second = create_links(&bin_dir, &scrim_exe, &config, false, false).unwrap();
    assert!(!modified_second);
    assert!(stale_link.exists());

    // Calling create_links with prune should remove stale_link and return modified = true
    let modified_prune = create_links(&bin_dir, &scrim_exe, &config, false, true).unwrap();
    assert!(modified_prune);
    assert!(!stale_link.exists());
    assert!(other_file.exists());

    // Calling create_links again with prune should succeed idempotently and return modified = false
    let modified_prune_second = create_links(&bin_dir, &scrim_exe, &config, false, true).unwrap();
    assert!(!modified_prune_second);
}

#[test]
fn test_create_links_dry_run() {
    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_exe").unwrap();

    // Create a stale scrim link
    let stale_link = bin_dir.join("oldtool");
    symlink(Path::new("../scrim"), &stale_link).unwrap();

    let mut tools = BTreeMap::new();
    tools.insert("node".to_string(), mock_host_tool());
    let config = Config {
        tools,
        ..Default::default()
    };

    // In dry run without prune, modified should be true (needs to link node), but oldtool is not touched
    let modified = create_links(&bin_dir, &scrim_exe, &config, true, false).unwrap();
    assert!(modified);

    // Neither node was created nor oldtool was removed
    assert!(!bin_dir.join("node").exists());
    assert!(stale_link.exists());

    // Now perform actual link creation without prune
    let actual_modified = create_links(&bin_dir, &scrim_exe, &config, false, false).unwrap();
    assert!(actual_modified);
    assert!(bin_dir.join("node").exists());
    assert!(stale_link.exists());

    // Now dry-run without prune: should report modified = false because node exists and prune is off
    let dry_modified_clean = create_links(&bin_dir, &scrim_exe, &config, true, false).unwrap();
    assert!(!dry_modified_clean);

    // Dry-run with prune: should report modified = true because oldtool would be removed
    let dry_prune_modified = create_links(&bin_dir, &scrim_exe, &config, true, true).unwrap();
    assert!(dry_prune_modified);
    assert!(stale_link.exists());

    // Actual run with prune: removes oldtool
    let actual_prune_modified = create_links(&bin_dir, &scrim_exe, &config, false, true).unwrap();
    assert!(actual_prune_modified);
    assert!(!stale_link.exists());

    // Dry-run with prune again: reports clean
    let dry_prune_clean = create_links(&bin_dir, &scrim_exe, &config, true, true).unwrap();
    assert!(!dry_prune_clean);
}

#[test]
fn test_create_links_does_not_overwrite_existing_file_or_conflicting_symlink() {
    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_scrim").unwrap();

    // 1. Tool 'node' already exists as a regular file
    let existing_node_file = bin_dir.join("node");
    fs::write(&existing_node_file, "original node binary content").unwrap();

    // 2. Tool 'go' already exists as a symlink pointing elsewhere
    let existing_go_link = bin_dir.join("go");
    symlink(Path::new("/bin/echo"), &existing_go_link).unwrap();

    // 3. Tool 'rust' does not exist yet
    let mut tools = BTreeMap::new();
    tools.insert("node".to_string(), mock_host_tool());
    tools.insert("go".to_string(), mock_host_tool());
    tools.insert("rust".to_string(), mock_host_tool());
    let config = Config {
        tools,
        ..Default::default()
    };

    // Execution should return error because node and go could not be linked
    let res = create_links(&bin_dir, &scrim_exe, &config, false, false);
    assert!(res.is_err());

    // Verify 'node' was not overwritten
    assert!(existing_node_file.is_file());
    assert_eq!(fs::read_to_string(&existing_node_file).unwrap(), "original node binary content");

    // Verify 'go' symlink was not overwritten and still points to /bin/echo
    assert!(fs::symlink_metadata(&existing_go_link).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(&existing_go_link).unwrap(), Path::new("/bin/echo"));

    // Verify 'rust' symlink was created (since it continued processing)
    let rust_link = bin_dir.join("rust");
    assert!(rust_link.exists());
    assert_eq!(fs::read_link(&rust_link).unwrap(), Path::new("../scrim"));
}

#[test]
fn test_create_links_host_platform_filtering() {
    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_scrim").unwrap();

    let host = config::Platform::host().unwrap();
    let non_host_os = if host.os == config::Os::Linux { "macos" } else { "linux" };
    let non_host_platform = format!("{}-aarch64", non_host_os);

    let mut tools = BTreeMap::new();
    // Tool 1: Configured for host
    tools.insert("host_tool".to_string(), mock_host_tool());

    // Tool 2: Configured only for a different OS/platform
    let mut non_host_plat_map = BTreeMap::new();
    non_host_plat_map.insert(
        non_host_platform,
        config::PlatformToolConfig {
            system_path: Some("/bin/non_host_bin".to_string()),
            ..Default::default()
        },
    );
    tools.insert(
        "unconfigured_tool".to_string(),
        ToolConfig {
            platforms: non_host_plat_map,
            ..Default::default()
        },
    );

    let config = Config {
        tools,
        ..Default::default()
    };

    // Pre-create an obsolete link for unconfigured_tool pointing to scrim
    let obsolete_link = bin_dir.join("unconfigured_tool");
    symlink(Path::new("../scrim"), &obsolete_link).unwrap();
    assert!(obsolete_link.exists());

    // Run dry-run without prune
    let dry_modified = create_links(&bin_dir, &scrim_exe, &config, true, false).unwrap();
    assert!(dry_modified);
    assert!(!bin_dir.join("host_tool").exists());
    assert!(obsolete_link.exists());

    // Normal run without prune
    let modified = create_links(&bin_dir, &scrim_exe, &config, false, false).unwrap();
    assert!(modified);
    assert!(bin_dir.join("host_tool").exists());
    assert!(obsolete_link.exists());

    // Run with prune
    let modified_prune = create_links(&bin_dir, &scrim_exe, &config, false, true).unwrap();
    assert!(modified_prune);
    assert!(!bin_dir.join("unconfigured_tool").exists());
}

#[test]
fn test_create_links_obsolete_link_removal_failure() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_scrim").unwrap();

    let stale_link = bin_dir.join("oldtool");
    symlink(Path::new("../scrim"), &stale_link).unwrap();

    if let Ok(mut perms) = fs::metadata(&bin_dir).map(|m| m.permissions()) {
        perms.set_mode(0o555);
        let _ = fs::set_permissions(&bin_dir, perms.clone());

        if fs::remove_file(&stale_link).is_err() {
            let config = Config::default();
            let res = create_links(&bin_dir, &scrim_exe, &config, false, true);
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&bin_dir, perms);
            assert!(res.is_err());
            assert_eq!(
                res.unwrap_err().to_string(),
                "One or more links could not be created"
            );
        } else {
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&bin_dir, perms);
        }
    }
}

#[test]
fn test_create_links_read_dir_failure() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_scrim").unwrap();

    if let Ok(mut perms) = fs::metadata(&bin_dir).map(|m| m.permissions()) {
        perms.set_mode(0o333);
        let _ = fs::set_permissions(&bin_dir, perms.clone());

        if fs::read_dir(&bin_dir).is_err() {
            let config = Config::default();
            let res = create_links(&bin_dir, &scrim_exe, &config, false, true);
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&bin_dir, perms);
            assert!(res.is_err());
            assert_eq!(
                res.unwrap_err().to_string(),
                "One or more links could not be created"
            );
        } else {
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&bin_dir, perms);
        }
    }
}

#[test]
fn test_create_links_validates_all_tools_including_non_host() {
    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_scrim").unwrap();

    let mut tools = std::collections::BTreeMap::new();
    // Valid tool for host platform
    tools.insert(
        "tool_a".to_string(),
        ToolConfig {
            system_path: Some("/bin/tool_a".to_string()),
            ..Default::default()
        },
    );
    // Invalid tool for non-host platform (macos-aarch64 has url without digest)
    let mut non_host_platforms = std::collections::BTreeMap::new();
    non_host_platforms.insert(
        "macos-aarch64".to_string(),
        config::PlatformToolConfig {
            url: Some("https://example.com/mac.tar.gz".to_string()),
            ..Default::default()
        },
    );
    tools.insert(
        "tool_b".to_string(),
        ToolConfig {
            platforms: non_host_platforms,
            ..Default::default()
        },
    );

    let config = Config {
        tools,
        ..Default::default()
    };

    let res = create_links(&bin_dir, &scrim_exe, &config, false, false);
    assert!(
        res.is_err(),
        "Expected create_links to fail on non-host tool defect, but it succeeded"
    );
    // Verify NO links were created in bin_dir
    assert!(!bin_dir.join("tool_a").exists());
    assert!(!bin_dir.join("tool_b").exists());
}

#[test]
fn test_create_links_makes_no_filesystem_changes_on_error() {
    let dir = tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let scrim_exe = dir.path().join("scrim");
    fs::write(&scrim_exe, "dummy_scrim").unwrap();

    let mut tools = std::collections::BTreeMap::new();
    // 1. Valid tool on host platform
    tools.insert(
        "tool_good".to_string(),
        ToolConfig {
            system_path: Some("/bin/tool_good".to_string()),
            ..Default::default()
        },
    );
    // 2. Invalid tool (missing digest)
    tools.insert(
        "tool_bad".to_string(),
        ToolConfig {
            url: Some("https://example.com/bad.tar.gz".to_string()),
            ..Default::default()
        },
    );

    let config = Config {
        tools,
        ..Default::default()
    };

    let res = create_links(&bin_dir, &scrim_exe, &config, false, false);
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err().to_string(),
        "One or more tool configurations are invalid"
    );

    // Under DESIGN.md, Scrim MUST make NO filesystem changes when validation fails!
    assert!(
        !bin_dir.join("tool_good").exists(),
        "tool_good symlink was created despite configuration validation error"
    );
    assert!(!bin_dir.join("tool_bad").exists());
}

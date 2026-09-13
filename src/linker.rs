use crate::config::Config;
use crate::{scrim_error, scrim_progress};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};

pub fn path_relative_from(target: &Path, base: &Path) -> PathBuf {
    let target_components: Vec<_> = target.components().collect();
    let base_components: Vec<_> = base.components().collect();
    let mut i = 0;
    while i < target_components.len()
        && i < base_components.len()
        && target_components[i] == base_components[i]
    {
        i += 1;
    }
    let mut rel = PathBuf::new();
    for comp in &base_components[i..] {
        if matches!(comp, Component::Normal(_)) {
            rel.push("..");
        }
    }
    for comp in &target_components[i..] {
        rel.push(comp.as_os_str());
    }
    if rel.as_os_str().is_empty() {
        rel.push(".");
    }
    rel
}

pub fn create_links(
    target_dir: &Path,
    scrim_exe: &Path,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    if !target_dir.is_dir() {
        return Err(format!(
            "Target directory '{}' does not exist or is not a directory",
            target_dir.display()
        )
        .into());
    }

    let canonical_target_dir = fs::canonicalize(target_dir).unwrap_or_else(|_| target_dir.to_path_buf());
    let canonical_scrim_exe = fs::canonicalize(scrim_exe).unwrap_or_else(|_| scrim_exe.to_path_buf());

    let rel_scrim_exe = path_relative_from(&canonical_scrim_exe, &canonical_target_dir);

    // 1. Create or update symlinks for configured tools
    let mut tool_names: Vec<_> = config.tools.keys().cloned().collect();
    tool_names.sort();

    let mut modified = false;
    let mut had_errors = false;

    for tool in &tool_names {
        let link_path = target_dir.join(tool);

        if let Ok(meta) = fs::symlink_metadata(&link_path) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = fs::read_link(&link_path) {
                    if target == rel_scrim_exe {
                        continue;
                    }
                }
                scrim_error!("'{}' already exists and points elsewhere", link_path.display());
                had_errors = true;
                continue;
            } else {
                scrim_error!("'{}' already exists", link_path.display());
                had_errors = true;
                continue;
            }
        }

        symlink(&rel_scrim_exe, &link_path)?;
        scrim_progress!("Linked {} -> {}", tool, rel_scrim_exe.display());
        modified = true;
    }

    // 2. Remove obsolete symlinks pointing to scrim
    match fs::read_dir(target_dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        scrim_error!(
                            "Failed to read directory entry in '{}': {}",
                            target_dir.display(),
                            e
                        );
                        had_errors = true;
                        continue;
                    }
                };
                let entry_path = entry.path();
                let file_name = match entry_path.file_name().and_then(|s| s.to_str()) {
                    Some(name) => name.to_string(),
                    None => continue,
                };

                // If this file is a tool in the config, it was just processed above
                if config.tools.contains_key(&file_name) {
                    continue;
                }

                // Check if entry is a symlink pointing to the scrim binary
                if let Ok(meta) = fs::symlink_metadata(&entry_path) {
                    if meta.file_type().is_symlink() {
                        let is_scrim_link = if let Ok(target) = fs::read_link(&entry_path) {
                            let resolved_target = if target.is_relative() {
                                canonical_target_dir.join(&target)
                            } else {
                                target.clone()
                            };

                            let canonical_resolved = fs::canonicalize(&resolved_target)
                                .unwrap_or(resolved_target);

                            canonical_resolved == canonical_scrim_exe
                        } else {
                            false
                        };

                        if is_scrim_link {
                            if let Err(e) = fs::remove_file(&entry_path) {
                                scrim_error!(
                                    "Failed to remove obsolete link '{}': {}",
                                    file_name,
                                    e
                                );
                                had_errors = true;
                            } else {
                                scrim_progress!("Removed obsolete link {}", file_name);
                                modified = true;
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            scrim_error!("Failed to read directory '{}': {}", target_dir.display(), e);
            had_errors = true;
        }
    }

    if !had_errors && !modified {
        scrim_progress!("All links are correct");
    }

    if had_errors {
        return Err("One or more links could not be created".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolConfig;
    use std::collections::HashMap;
    use tempfile::tempdir;

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
        let res = create_links(&nonexistent, &scrim_exe, &config);
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

        let mut tools = HashMap::new();
        tools.insert("node".to_string(), ToolConfig::default());
        tools.insert("go".to_string(), ToolConfig::default());
        let config = Config {
            telemetry: None,
            tools,
        };

        create_links(&bin_dir, &scrim_exe, &config).unwrap();

        // Check node link
        let node_link = bin_dir.join("node");
        assert!(node_link.exists());
        assert_eq!(fs::read_link(&node_link).unwrap(), Path::new("../scrim"));

        // Check go link
        let go_link = bin_dir.join("go");
        assert!(go_link.exists());
        assert_eq!(fs::read_link(&go_link).unwrap(), Path::new("../scrim"));

        // Check oldtool (stale scrim link) was removed
        assert!(!stale_link.exists());

        // Check unrelated file remains untouched
        assert!(other_file.exists());

        // Calling create_links again should succeed idempotently
        create_links(&bin_dir, &scrim_exe, &config).unwrap();
        assert!(node_link.exists());
        assert!(go_link.exists());
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
        let mut tools = HashMap::new();
        tools.insert("node".to_string(), ToolConfig::default());
        tools.insert("go".to_string(), ToolConfig::default());
        tools.insert("rust".to_string(), ToolConfig::default());
        let config = Config {
            telemetry: None,
            tools,
        };

        // Execution should return error because node and go could not be linked
        let res = create_links(&bin_dir, &scrim_exe, &config);
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
                let res = create_links(&bin_dir, &scrim_exe, &config);
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
                let res = create_links(&bin_dir, &scrim_exe, &config);
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
}


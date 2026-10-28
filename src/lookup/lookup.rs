use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Searches for an executable matching `program_name` across directories in `path_var`.
///
/// To prevent infinite loops when Scrim itself is present in PATH (via symlink, hardlink,
/// or binary), candidate files whose filesystem identity (device and inode) or canonical
/// path matches `current_exe` are skipped.
pub fn find_in_path(
    program_name: &str,
    path_var: Option<&OsStr>,
    current_exe: Option<&Path>,
) -> Option<PathBuf> {
    let path_var = path_var?;
    let (exe_dev_ino, exe_canon) = match current_exe {
        Some(exe) => {
            let meta = std::fs::metadata(exe).ok().map(|m| (m.dev(), m.ino()));
            let canon = std::fs::canonicalize(exe).ok();
            (meta, canon)
        }
        None => (None, None),
    };

    for dir in std::env::split_paths(path_var) {
        // In POSIX, an empty PATH entry denotes the current working directory.
        // Normalizing to "." ensures the path has a directory component (e.g. "./tool"),
        // preventing Command::new from treating it as an unqualified name and re-invoking execvp.
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &dir
        };
        let candidate = dir.join(program_name);

        // Check if candidate exists (following symlinks)
        let metadata = match std::fs::metadata(&candidate) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if !metadata.is_file() {
            continue;
        }

        // Must have at least one executable bit set (owner, group, or other)
        if metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }

        // Check if candidate matches current_exe by device and inode (covers hardlinks and symlinks)
        if let Some((exe_dev, exe_ino)) = exe_dev_ino {
            if metadata.dev() == exe_dev && metadata.ino() == exe_ino {
                continue;
            }
        }

        // Also check if candidate matches current_exe by canonical path
        if let Some(ref canon_exe) = exe_canon {
            if let Ok(cand_canon) = std::fs::canonicalize(&candidate) {
                if &cand_canon == canon_exe {
                    continue;
                }
            }
        }

        return Some(candidate);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_in_path_none_or_empty() {
        assert_eq!(find_in_path("node", None, None), None);
        assert_eq!(find_in_path("node", Some(OsStr::new("")), None), None);
    }

    #[test]
    fn test_find_in_path_empty_entry_in_path_normalizes_to_dot() {
        // An empty entry in PATH (e.g. ":/usr/bin") corresponds to "."
        let path_with_empty_entry = OsStr::new(":/nonexistent_dir");
        let result = find_in_path("nonexistent_binary_xyz_123", Some(path_with_empty_entry), None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_in_path_basic() {
        let temp = tempfile::tempdir().unwrap();
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let tool_path = bin_dir.join("mytool");
        std::fs::write(&tool_path, "#!/bin/sh\n").unwrap();
        let mut perms = std::fs::metadata(&tool_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tool_path, perms).unwrap();

        let path_env = std::env::join_paths([&bin_dir]).unwrap();
        assert_eq!(
            find_in_path("mytool", Some(&path_env), None),
            Some(tool_path)
        );
        assert_eq!(find_in_path("nonexistent", Some(&path_env), None), None);
    }

    #[test]
    fn test_find_in_path_skips_non_executable_and_directories() {
        let temp = tempfile::tempdir().unwrap();
        let dir1 = temp.path().join("dir1");
        let dir2 = temp.path().join("dir2");
        let dir3 = temp.path().join("dir3");
        std::fs::create_dir_all(&dir1).unwrap();
        std::fs::create_dir_all(&dir2).unwrap();
        std::fs::create_dir_all(&dir3).unwrap();

        // dir1 has a directory named "mytool"
        std::fs::create_dir_all(dir1.join("mytool")).unwrap();

        // dir2 has a non-executable file named "mytool"
        let non_exec = dir2.join("mytool");
        std::fs::write(&non_exec, "not executable").unwrap();
        let mut perms = std::fs::metadata(&non_exec).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&non_exec, perms).unwrap();

        // dir3 has the valid executable "mytool"
        let valid_exec = dir3.join("mytool");
        std::fs::write(&valid_exec, "#!/bin/sh\n").unwrap();
        let mut perms = std::fs::metadata(&valid_exec).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&valid_exec, perms).unwrap();

        let path_env = std::env::join_paths([&dir1, &dir2, &dir3]).unwrap();
        assert_eq!(
            find_in_path("mytool", Some(&path_env), None),
            Some(valid_exec)
        );
    }

    #[test]
    fn test_find_in_path_skips_symlinks_and_hardlinks_to_current_exe() {
        let temp = tempfile::tempdir().unwrap();
        let scrim_dir = temp.path().join("scrim_bin");
        let link_dir1 = temp.path().join("links_sym");
        let link_dir2 = temp.path().join("links_hard");
        let sys_dir = temp.path().join("sys_bin");

        std::fs::create_dir_all(&scrim_dir).unwrap();
        std::fs::create_dir_all(&link_dir1).unwrap();
        std::fs::create_dir_all(&link_dir2).unwrap();
        std::fs::create_dir_all(&sys_dir).unwrap();

        // Mock current_exe
        let fake_scrim = scrim_dir.join("scrim");
        std::fs::write(&fake_scrim, "binary content").unwrap();
        let mut perms = std::fs::metadata(&fake_scrim).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_scrim, perms).unwrap();

        // Symlink in link_dir1
        let symlink_path = link_dir1.join("node");
        std::os::unix::fs::symlink(&fake_scrim, &symlink_path).unwrap();

        // Hardlink in link_dir2
        let hardlink_path = link_dir2.join("node");
        std::fs::hard_link(&fake_scrim, &hardlink_path).unwrap();

        // Real node executable in sys_dir
        let real_node = sys_dir.join("node");
        std::fs::write(&real_node, "#!/bin/sh\necho real node\n").unwrap();
        let mut perms = std::fs::metadata(&real_node).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&real_node, perms).unwrap();

        let path_env = std::env::join_paths([&link_dir1, &link_dir2, &sys_dir]).unwrap();

        // Must skip link_dir1/node (symlink) and link_dir2/node (hardlink), returning sys_dir/node
        assert_eq!(
            find_in_path("node", Some(&path_env), Some(&fake_scrim)),
            Some(real_node)
        );

        // If sys_dir is removed from PATH, find_in_path should return None
        let path_links_only = std::env::join_paths([&link_dir1, &link_dir2]).unwrap();
        assert_eq!(
            find_in_path("node", Some(&path_links_only), Some(&fake_scrim)),
            None
        );
    }
}

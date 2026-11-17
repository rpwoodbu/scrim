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

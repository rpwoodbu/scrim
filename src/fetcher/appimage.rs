use backhand::{FilesystemReader, InnerNode};
use elf::parse_elf_end_offset;
use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub fn format_extract_error(err: &dyn std::error::Error) -> String {
    let mut cur = err;
    let mut msg = err.to_string();
    while let Some(source) = cur.source() {
        let s = source.to_string();
        if !msg.contains(&s) {
            msg = format!("{}: {}", msg, s);
        }
        cur = source;
    }
    msg
}

fn find_appimage_squashfs_offset(file: &mut File) -> Result<u64, String> {
    let file_len = file
        .metadata()
        .map_err(|e| format!("Failed to read AppImage metadata: {}", e))?
        .len();

    let (e_ident, max_offset) = parse_elf_end_offset(file)
        .map_err(|e| format!("Invalid AppImage: {}", e))?;

    if &e_ident[8..11] == b"AI\x01" {
        return Err("Type 1 AppImages (ISO 9660 based) are deprecated and unsupported. Please use a Type 2 AppImage.".to_string());
    }

    if max_offset >= file_len {
        return Err(format!(
            "Invalid AppImage: computed SquashFS offset ({}) is beyond end of file ({})",
            max_offset, file_len
        ));
    }

    Ok(max_offset)
}

fn resolve_symlink_within_root(
    root: &Path,
    link_location: &Path,
    target: &Path,
) -> Result<(), String> {
    if target.is_absolute() {
        return Err(format!(
            "Symlink {} points to absolute path {}, which is disallowed for security reasons",
            link_location.display(),
            target.display()
        ));
    }
    let parent = link_location.parent().unwrap_or(root);
    let rel_parent = parent.strip_prefix(root).unwrap_or(Path::new(""));
    let mut depth: isize = 0;
    for comp in rel_parent.components() {
        if matches!(comp, std::path::Component::Normal(_)) {
            depth += 1;
        }
    }
    for comp in target.components() {
        match comp {
            std::path::Component::Normal(_) => depth += 1,
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!(
                        "Symlink {} -> {} points outside of the extracted directory",
                        link_location.display(),
                        target.display()
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "Symlink {} -> {} contains invalid path component",
                    link_location.display(),
                    target.display()
                ));
            }
        }
    }
    Ok(())
}

pub fn extract_appimage(appimage_file: File, dest_dir: &Path) -> Result<(), String> {
    let mut file = appimage_file;
    let offset = find_appimage_squashfs_offset(&mut file)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("Failed to rewind AppImage file: {}", e))?;

    let reader = io::BufReader::new(file);
    let filesystem = FilesystemReader::from_reader_with_offset(reader, offset).map_err(|e| {
        format!(
            "Failed to read SquashFS filesystem at offset {}: {}",
            offset,
            format_extract_error(&e)
        )
    })?;

    let mut symlinks_to_create: Vec<(PathBuf, PathBuf)> = Vec::new();

    for node in filesystem.files() {
        let rel_path = node.fullpath.strip_prefix("/").unwrap_or(&node.fullpath);
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        if !rel_path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
        {
            return Err(format!(
                "Invalid path in AppImage SquashFS: {}",
                rel_path.display()
            ));
        }

        let out_path = dest_dir.join(rel_path);
        match &node.inner {
            InnerNode::Dir(_) => {
                fs::create_dir_all(&out_path).map_err(|e| {
                    format!("Failed to create directory {}: {}", out_path.display(), e)
                })?;
            }
            InnerNode::File(f) => {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| {
                        format!(
                            "Failed to create parent directory {}: {}",
                            parent.display(),
                            e
                        )
                    })?;
                }
                let mut out_file = File::create(&out_path)
                    .map_err(|e| format!("Failed to create file {}: {}", out_path.display(), e))?;
                let mut file_reader = filesystem.file(f).reader();
                io::copy(&mut file_reader, &mut out_file)
                    .map_err(|e| format!("Failed to extract file {}: {}", out_path.display(), e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = (node.header.permissions as u32) & 0o777;
                    if mode != 0 {
                        let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode));
                    }
                }
            }
            InnerNode::Symlink(s) => {
                resolve_symlink_within_root(dest_dir, &out_path, &s.link)?;
                symlinks_to_create.push((out_path, s.link.clone()));
            }
            _ => {}
        }
    }

    for (out_path, link_target) in symlinks_to_create {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create parent directory {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&link_target, &out_path)
                .map_err(|e| format!("Failed to create symlink {}: {}", out_path.display(), e))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use test_helpers::{create_fake_appimage, make_elf64_stub};

    #[test]
    fn test_extract_appimage_success() {
        let temp = tempdir().unwrap();
        let appimage_path = temp.path().join("test.AppImage");
        create_fake_appimage(
            &appimage_path,
            &[("usr/bin/real_bin", b"#!/bin/sh\necho 'real'\n", 0o755)],
            &[("AppRun", "usr/bin/real_bin")],
        );

        let dest = temp.path().join("out");
        fs::create_dir_all(&dest).unwrap();
        let file = File::open(&appimage_path).unwrap();
        assert!(extract_appimage(file, &dest).is_ok());
        assert!(dest.join("usr/bin/real_bin").exists());
        assert!(dest.join("AppRun").is_symlink());
    }

    #[test]
    fn test_extract_appimage_type1_rejected() {
        let temp = tempdir().unwrap();
        let mut type1_bytes = make_elf64_stub();
        type1_bytes[8..11].copy_from_slice(b"AI\x01");
        type1_bytes.extend_from_slice(b"trailing_payload");

        let appimage_path = temp.path().join("type1.AppImage");
        fs::write(&appimage_path, &type1_bytes).unwrap();

        let dest = temp.path().join("out");
        fs::create_dir_all(&dest).unwrap();
        let file = File::open(&appimage_path).unwrap();
        let err = extract_appimage(file, &dest).unwrap_err();
        assert!(err.contains("Type 1 AppImages (ISO 9660 based) are deprecated and unsupported"));
    }

    #[test]
    fn test_extract_appimage_offset_beyond_eof() {
        let temp = tempdir().unwrap();
        let stub_only = make_elf64_stub();
        let p = temp.path().join("stub_only.AppImage");
        fs::write(&p, &stub_only).unwrap();

        let dest = temp.path().join("out");
        fs::create_dir_all(&dest).unwrap();
        let file = File::open(&p).unwrap();
        let err = extract_appimage(file, &dest).unwrap_err();
        assert!(err.contains("is beyond end of file"));
    }

    #[test]
    fn test_extract_appimage_corrupt_not_elf() {
        let temp = tempdir().unwrap();
        let corrupt_path = temp.path().join("corrupt.AppImage");
        fs::write(&corrupt_path, b"not an ELF file at all").unwrap();

        let dest = temp.path().join("out");
        fs::create_dir_all(&dest).unwrap();
        let file = File::open(&corrupt_path).unwrap();
        let err = extract_appimage(file, &dest).unwrap_err();
        assert!(err.contains("not a valid ELF binary"), "Got: {}", err);
    }

    #[test]
    fn test_extract_appimage_corrupt_squashfs() {
        let temp = tempdir().unwrap();
        let mut bad_sqfs = make_elf64_stub();
        bad_sqfs.extend_from_slice(b"not a valid squashfs payload");

        let corrupt_path = temp.path().join("corrupt_sqfs.AppImage");
        fs::write(&corrupt_path, &bad_sqfs).unwrap();

        let dest = temp.path().join("out");
        fs::create_dir_all(&dest).unwrap();
        let file = File::open(&corrupt_path).unwrap();
        let err = extract_appimage(file, &dest).unwrap_err();
        assert!(err.contains("Failed to read SquashFS filesystem"), "Got: {}", err);
    }

    #[test]
    #[cfg(unix)]
    fn test_extract_appimage_escaping_symlinks_rejected() {
        let temp = tempdir().unwrap();

        // 1. Absolute symlink target
        let appimage_abs = temp.path().join("abs.AppImage");
        create_fake_appimage(
            &appimage_abs,
            &[("usr/bin/real_bin", b"echo 'real'", 0o755)],
            &[("AppRun", "/etc/passwd")],
        );
        let dest_abs = temp.path().join("out_abs");
        fs::create_dir_all(&dest_abs).unwrap();
        let err_abs = extract_appimage(File::open(&appimage_abs).unwrap(), &dest_abs).unwrap_err();
        assert!(err_abs.contains("disallowed for security reasons"), "Got: {}", err_abs);

        // 2. Relative escaping symlink target
        let appimage_rel = temp.path().join("rel.AppImage");
        create_fake_appimage(
            &appimage_rel,
            &[("usr/bin/real_bin", b"echo 'real'", 0o755)],
            &[("usr/bin/bad_link", "../../../etc/passwd")],
        );
        let dest_rel = temp.path().join("out_rel");
        fs::create_dir_all(&dest_rel).unwrap();
        let err_rel = extract_appimage(File::open(&appimage_rel).unwrap(), &dest_rel).unwrap_err();
        assert!(err_rel.contains("points outside of the extracted directory"), "Got: {}", err_rel);
    }
}

use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader, compression::Compressor};
use std::fs;
use std::io::Cursor;
use std::path::Path;

/// Constructs a minimal 128-byte 64-bit little-endian ELF stub containing the
/// Type 2 AppImage magic bytes (`AI\x02`) and a single `SHT_NULL` section header
/// ending at byte offset 128.
pub fn make_elf64_stub() -> Vec<u8> {
    let mut stub = vec![0u8; 128];
    // ELF magic
    stub[0..4].copy_from_slice(b"\x7fELF");
    stub[4] = 2; // 64-bit
    stub[5] = 1; // Little Endian
    stub[6] = 1; // ELF version 1
    // Type 2 AppImage magic at offset 8
    stub[8..11].copy_from_slice(b"AI\x02");
    // e_shoff at offset 40 = 64
    stub[40..48].copy_from_slice(&64u64.to_le_bytes());
    // e_shentsize at offset 58 = 64
    stub[58..60].copy_from_slice(&64u16.to_le_bytes());
    // e_shnum at offset 60 = 1
    stub[60..62].copy_from_slice(&1u16.to_le_bytes());
    // Section header 0 at offset 64..128 (SHT_NULL = 0)
    stub
}

/// Writes a synthetic Type 2 AppImage file at `path` consisting of [`make_elf64_stub`]
/// followed by a gzip-compressed SquashFS image containing `files` (`(path, content, mode)`)
/// and `symlinks` (`(link_path, target)`).
pub fn create_fake_appimage(path: &Path, files: &[(&str, &[u8], u16)], symlinks: &[(&str, &str)]) {
    let mut stub = make_elf64_stub();

    let mut sqfs_writer = FilesystemWriter::default();
    let compressor = FilesystemCompressor::new(Compressor::Gzip, None).unwrap();
    sqfs_writer.set_compressor(compressor);
    sqfs_writer.set_no_padding();

    for &(file_path, content, mode) in files {
        let p = Path::new(file_path);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() && parent != Path::new("/") {
                let dir_header = NodeHeader::new(0o755, 0, 0, 0);
                let _ = sqfs_writer.push_dir_all(parent, dir_header);
            }
        }
        let header = NodeHeader::new(mode, 0, 0, 0);
        sqfs_writer
            .push_file(Cursor::new(content.to_vec()), file_path, header)
            .unwrap();
    }

    for &(link_path, target) in symlinks {
        let p = Path::new(link_path);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() && parent != Path::new("/") {
                let dir_header = NodeHeader::new(0o755, 0, 0, 0);
                let _ = sqfs_writer.push_dir_all(parent, dir_header);
            }
        }
        let header = NodeHeader::new(0o777, 0, 0, 0);
        sqfs_writer.push_symlink(target, link_path, header).unwrap();
    }

    let mut sqfs_bytes = Cursor::new(Vec::new());
    sqfs_writer.write(&mut sqfs_bytes).unwrap();
    stub.extend_from_slice(&sqfs_bytes.into_inner());

    fs::write(path, &stub).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use backhand::{FilesystemReader, InnerNode};
    use std::fs::File;
    use std::io::{BufReader, Read};
    use tempfile::tempdir;

    #[test]
    fn test_make_elf64_stub_structure() {
        let stub = make_elf64_stub();
        assert_eq!(stub.len(), 128);
        assert_eq!(&stub[0..4], b"\x7fELF");
        assert_eq!(stub[4], 2); // ELFCLASS64
        assert_eq!(stub[5], 1); // ELFDATA2LSB
        assert_eq!(stub[6], 1); // EV_CURRENT
        assert_eq!(&stub[8..11], b"AI\x02"); // AppImage Type 2 magic

        let shoff = u64::from_le_bytes(stub[40..48].try_into().unwrap());
        let shentsize = u16::from_le_bytes(stub[58..60].try_into().unwrap());
        let shnum = u16::from_le_bytes(stub[60..62].try_into().unwrap());
        assert_eq!(shoff, 64);
        assert_eq!(shentsize, 64);
        assert_eq!(shnum, 1);
        assert_eq!(shoff + (shentsize as u64) * (shnum as u64), 128);
    }

    #[test]
    fn test_create_fake_appimage_writes_valid_elf_and_squashfs() {
        let temp = tempdir().unwrap();
        let appimage_path = temp.path().join("fake.AppImage");
        create_fake_appimage(
            &appimage_path,
            &[("usr/bin/tool", b"#!/bin/sh\necho hi\n", 0o755)],
            &[("AppRun", "usr/bin/tool")],
        );

        let bytes = fs::read(&appimage_path).unwrap();
        assert!(bytes.len() > 128);
        assert_eq!(&bytes[0..128], &make_elf64_stub()[..]);

        let file = File::open(&appimage_path).unwrap();
        let reader = BufReader::new(file);
        let sqfs = FilesystemReader::from_reader_with_offset(reader, 128).unwrap();

        let mut found_file = false;
        let mut found_symlink = false;
        for node in sqfs.files() {
            let rel = node.fullpath.strip_prefix("/").unwrap_or(&node.fullpath);
            if rel == Path::new("usr/bin/tool") {
                if let InnerNode::File(f) = &node.inner {
                    let mut content = Vec::new();
                    sqfs.file(f).reader().read_to_end(&mut content).unwrap();
                    assert_eq!(content, b"#!/bin/sh\necho hi\n");
                    assert_eq!(node.header.permissions, 0o755);
                    found_file = true;
                }
            } else if rel == Path::new("AppRun") {
                if let InnerNode::Symlink(s) = &node.inner {
                    assert_eq!(s.link, Path::new("usr/bin/tool"));
                    found_symlink = true;
                }
            }
        }
        assert!(found_file, "Expected usr/bin/tool in SquashFS");
        assert!(found_symlink, "Expected AppRun symlink in SquashFS");
    }
}

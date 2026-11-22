use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader, compression::Compressor};
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Locates a binary target in Bazel runfiles, relative to the current test/benchmark
/// executable, or in `bazel-bin/<pkg_rel_path>/<bin_name>`.
pub fn find_runfile_bin(pkg_rel_path: &str, bin_name: &str) -> PathBuf {
    if let Ok(runfiles_dir) = env::var("RUNFILES_DIR") {
        let path = Path::new(&runfiles_dir)
            .join("_main")
            .join(pkg_rel_path)
            .join(bin_name);
        if path.exists() {
            return path;
        }
    }
    if let Ok(current_exe) = env::current_exe() {
        if let Some(exe_name) = current_exe.file_name().and_then(|s| s.to_str()) {
            let sibling_runfiles = current_exe
                .with_file_name(format!("{}.runfiles", exe_name))
                .join("_main")
                .join(pkg_rel_path)
                .join(bin_name);
            if sibling_runfiles.exists() {
                return sibling_runfiles;
            }
        }
        if let Some(dir) = current_exe.parent() {
            let path = dir.join(bin_name);
            if path.exists() {
                return path;
            }
        }
    }
    let path = PathBuf::from("bazel-bin").join(pkg_rel_path).join(bin_name);
    if path.exists() {
        return path;
    }
    panic!("Could not locate '{}' binary", bin_name);
}

/// Locates the `scrim` binary (`//src/bin:scrim`).
pub fn find_scrim_bin() -> PathBuf {
    find_runfile_bin("src/bin", "scrim")
}

/// Locates the `scrim_bench` binary (`//benches:scrim_bench`).
pub fn find_scrim_bench_bin() -> PathBuf {
    find_runfile_bin("benches", "scrim_bench")
}

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

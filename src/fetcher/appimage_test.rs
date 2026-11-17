use appimage::extract_appimage;
use std::fs::{self, File};
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

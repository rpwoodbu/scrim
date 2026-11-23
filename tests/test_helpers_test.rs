use backhand::{FilesystemReader, InnerNode};
use std::env;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use tempfile::tempdir;
use test_helpers::{
    copy_executable, create_fake_appimage, find_runfile_bin, make_elf64_stub, write_executable,
};

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

#[test]
fn test_find_runfile_bin_locates_current_exe_sibling() {
    let current_exe = env::current_exe().unwrap();
    let exe_name = current_exe.file_name().unwrap().to_str().unwrap();
    let found = find_runfile_bin("nonexistent/pkg", exe_name);
    assert!(found.exists());
    assert_eq!(found.file_name().unwrap().to_str().unwrap(), exe_name);
}

#[test]
#[should_panic(expected = "Could not locate 'nonexistent_scrim_test_bin' binary")]
fn test_find_runfile_bin_panics_when_missing() {
    find_runfile_bin("nonexistent/pkg", "nonexistent_scrim_test_bin");
}

#[test]
fn test_write_executable_in_child_creates_runnable_file() {
    let temp = tempdir().unwrap();
    let script_path = temp.path().join("child_written.sh");
    write_executable(&script_path, "#!/bin/sh\necho 'child writer ok'\n");

    let out = std::process::Command::new(&script_path)
        .output()
        .expect("Failed to execute child written script");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "child writer ok");
}

#[test]
fn test_copy_executable_in_child_creates_runnable_copy() {
    let temp = tempdir().unwrap();
    let src_path = temp.path().join("src.sh");
    let dst_path = temp.path().join("dst.sh");
    write_executable(&src_path, "#!/bin/sh\necho 'copied ok'\n");
    copy_executable(&src_path, &dst_path);

    let out = std::process::Command::new(&dst_path)
        .output()
        .expect("Failed to execute copied script");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "copied ok");
}

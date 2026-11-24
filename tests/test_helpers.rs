use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader, compression::Compressor};
use std::env;
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

    write_executable(path, &stub);
}

/// Writes an executable file (or script) in a child process so that the parent test runner
/// process never holds an open writable file descriptor to it.
///
/// In multi-threaded test runners, writing an executable directly in-process can leak an open
/// write descriptor if another thread concurrently calls `fork()`. This triggers transient
/// `ETXTBSY` ("Text file busy") when the newly created file is subsequently executed. By
/// delegating the write to a child process that exits before this function returns, the
/// parent process never has an open write handle to leak.
pub fn write_executable(path: &Path, content: impl AsRef<[u8]>) {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg("cat > \"$1\" && chmod 755 -- \"$1\"")
        .arg("--")
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn writer child process");

    use std::io::Write;
    child
        .stdin
        .take()
        .expect("Failed to open stdin for writer")
        .write_all(content.as_ref())
        .expect("Failed to write content to writer child");

    let status = child.wait().expect("Failed to wait on writer child");
    assert!(status.success(), "Failed to write executable to {:?}", path);
}

/// Copies an executable binary in a child process (`cp`) so the parent test process
/// never opens the destination file for writing.
pub fn copy_executable(src: &Path, dst: &Path) {
    let status = std::process::Command::new("cp")
        .args(["--"])
        .args([src, dst])
        .status()
        .expect("Failed to spawn cp child process");
    assert!(status.success(), "Failed to copy {:?} to {:?}", src, dst);
}

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;
use std::net::TcpListener;
use std::io::{Read, Write};
use std::thread;

pub const TEST_CA_CERT_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDEzCCAfugAwIBAgIUTTaqWHgccHJlp43sdNsOITkIXj0wDQYJKoZIhvcNAQEL
BQAwGDEWMBQGA1UEAwwNU2NyaW0gVGVzdCBDQTAgFw0yNjExMjQwNDAyNTFaGA8y
MTI2MTAzMTA0MDI1MVowGDEWMBQGA1UEAwwNU2NyaW0gVGVzdCBDQTCCASIwDQYJ
KoZIhvcNAQEBBQADggEPADCCAQoCggEBANRmrmL13rqKtGiA/HYkcBdJV5njXXXa
UH8iV4cU+E6hWokeEF3JdYXsRTveIXcs+Icpip2NeyXTmYF6Cbczt2ifZfFSlHKn
3cT2UiYQy8pZt7q0xK6TgEpBj/ncqeTXEtBJ/Em3xd1QG4gFmBXd1vViwQx/2Vox
7sjaciXrdsU09HH+HaD15F7KY9dxjmiU2wVc52PLs3Xc0xzyG48eddEco3CNAoVl
WrWVfOYiVgMRMsvnfg6O6ok8kAy0EfTARFC1P8ZO5yvQvUb3ACWm3eUD4Zd2ylsE
c6OJzeQUFg10uaOs72Wz34IVymJLASbudllnYydCSOSD9RAwnWnAV8MCAwEAAaNT
MFEwHQYDVR0OBBYEFOsvonP2R1HQut5bkqCVovAX81ksMB8GA1UdIwQYMBaAFOsv
onP2R1HQut5bkqCVovAX81ksMA8GA1UdEwEB/wQFMAMBAf8wDQYJKoZIhvcNAQEL
BQADggEBAKmW+iQlwKIGLtROqQlzVuRtmSwTcYfqMJCCvxbyR0zN/ilLPnT0zOWA
Bbjy6NMbiMoCIHC2w0OUzHJQAaxFNx3skpbR8j7zTdavxz92UWfwF9gWEPzGqaQm
i6wPbK+aOCcyNrtgLrPPgrM6PIwHbDz4uIsXBxiWWpefX4p0us3sBlaWd6mhDkUe
tEcGg8FYLna2U1pdf0oA+U1od07VARjDfHjT2ZKeHFPhAHhatrBvNdNFvzVEU4D4
hPHTZKHUH0D3AXlDvVrit5wZMM16QrR2x4lB0nwxqJs09yqVpKyXq/i/g7FVDVlv
I3LFl4ZWpgeAnUfcgGgARU5cd+g1c5A=
-----END CERTIFICATE-----
"#;

pub const TEST_SERVER_CERT_DER: &[u8] = &[48, 130, 3, 73, 48, 130, 2, 49, 160, 3, 2, 1, 2, 2, 20, 45, 75, 126, 165, 227, 189, 252, 128, 85, 197, 240, 92, 26, 57, 112, 17, 43, 92, 204, 98, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 48, 24, 49, 22, 48, 20, 6, 3, 85, 4, 3, 12, 13, 83, 99, 114, 105, 109, 32, 84, 101, 115, 116, 32, 67, 65, 48, 32, 23, 13, 50, 54, 49, 49, 50, 52, 48, 52, 48, 50, 53, 49, 90, 24, 15, 50, 49, 50, 54, 49, 48, 51, 49, 48, 52, 48, 50, 53, 49, 90, 48, 20, 49, 18, 48, 16, 6, 3, 85, 4, 3, 12, 9, 108, 111, 99, 97, 108, 104, 111, 115, 116, 48, 130, 1, 34, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 3, 130, 1, 15, 0, 48, 130, 1, 10, 2, 130, 1, 1, 0, 209, 49, 18, 244, 141, 52, 131, 148, 147, 30, 250, 232, 243, 52, 17, 18, 224, 29, 124, 242, 82, 204, 253, 135, 46, 112, 210, 245, 184, 88, 57, 226, 38, 110, 157, 64, 80, 221, 188, 18, 165, 241, 105, 197, 245, 5, 159, 155, 59, 235, 7, 95, 244, 149, 138, 50, 212, 234, 229, 209, 249, 36, 114, 125, 43, 96, 166, 128, 187, 117, 173, 182, 34, 33, 200, 152, 55, 19, 158, 168, 79, 238, 19, 194, 39, 137, 205, 210, 103, 170, 151, 187, 137, 247, 43, 145, 27, 254, 32, 215, 235, 104, 67, 29, 88, 210, 85, 237, 249, 183, 245, 149, 81, 119, 53, 141, 72, 91, 173, 87, 146, 178, 6, 31, 145, 1, 148, 184, 66, 58, 157, 78, 141, 228, 47, 221, 189, 205, 194, 122, 238, 227, 60, 252, 187, 176, 88, 118, 80, 220, 102, 131, 115, 250, 31, 243, 164, 216, 170, 40, 113, 229, 174, 183, 131, 46, 41, 81, 212, 235, 219, 183, 10, 219, 189, 174, 232, 85, 233, 245, 56, 5, 244, 206, 127, 48, 223, 218, 77, 42, 134, 117, 166, 195, 217, 62, 177, 36, 82, 105, 27, 165, 42, 12, 83, 121, 2, 31, 142, 227, 14, 180, 59, 107, 184, 237, 213, 116, 168, 68, 18, 29, 203, 76, 72, 239, 141, 248, 89, 239, 107, 190, 105, 46, 33, 10, 92, 227, 117, 152, 243, 227, 186, 117, 116, 187, 64, 206, 225, 253, 48, 213, 176, 54, 175, 165, 2, 3, 1, 0, 1, 163, 129, 140, 48, 129, 137, 48, 26, 6, 3, 85, 29, 17, 4, 19, 48, 17, 130, 9, 108, 111, 99, 97, 108, 104, 111, 115, 116, 135, 4, 127, 0, 0, 1, 48, 9, 6, 3, 85, 29, 19, 4, 2, 48, 0, 48, 11, 6, 3, 85, 29, 15, 4, 4, 3, 2, 5, 160, 48, 19, 6, 3, 85, 29, 37, 4, 12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 1, 48, 29, 6, 3, 85, 29, 14, 4, 22, 4, 20, 110, 206, 229, 87, 210, 183, 70, 75, 101, 66, 183, 204, 148, 31, 239, 212, 96, 126, 172, 252, 48, 31, 6, 3, 85, 29, 35, 4, 24, 48, 22, 128, 20, 235, 47, 162, 115, 246, 71, 81, 208, 186, 222, 91, 146, 160, 149, 162, 240, 23, 243, 89, 44, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 3, 130, 1, 1, 0, 48, 132, 238, 1, 156, 224, 136, 117, 96, 138, 128, 221, 104, 212, 4, 174, 226, 156, 79, 80, 225, 243, 104, 251, 13, 29, 55, 116, 253, 39, 140, 151, 241, 171, 220, 34, 192, 11, 0, 237, 241, 245, 253, 100, 61, 22, 179, 245, 26, 149, 78, 144, 71, 59, 39, 190, 111, 45, 216, 252, 62, 105, 67, 119, 64, 105, 58, 70, 208, 200, 249, 38, 217, 227, 75, 253, 17, 74, 131, 233, 143, 39, 163, 250, 243, 224, 187, 73, 195, 144, 14, 14, 91, 83, 172, 121, 61, 26, 55, 163, 203, 66, 141, 19, 59, 131, 64, 113, 165, 0, 20, 140, 116, 27, 156, 86, 50, 116, 93, 100, 92, 29, 189, 114, 184, 69, 31, 178, 15, 96, 12, 78, 53, 200, 112, 56, 58, 187, 202, 0, 255, 178, 181, 17, 164, 58, 72, 182, 18, 129, 128, 20, 213, 249, 2, 218, 240, 63, 174, 15, 139, 190, 2, 237, 77, 243, 43, 11, 4, 29, 246, 149, 45, 38, 32, 212, 246, 128, 69, 44, 55, 245, 99, 120, 70, 100, 85, 177, 87, 8, 39, 174, 164, 92, 12, 155, 137, 89, 165, 123, 34, 18, 96, 119, 107, 95, 88, 237, 175, 172, 71, 144, 33, 71, 216, 245, 137, 106, 116, 161, 86, 205, 166, 85, 205, 194, 140, 55, 142, 162, 97, 230, 107, 1, 174, 156, 191, 232, 200, 7, 82, 132, 26, 82, 112, 204, 156, 143, 226, 46, 35, 16, 202, 90, 45, 102];

pub const TEST_SERVER_KEY_DER: &[u8] = &[48, 130, 4, 190, 2, 1, 0, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 4, 130, 4, 168, 48, 130, 4, 164, 2, 1, 0, 2, 130, 1, 1, 0, 209, 49, 18, 244, 141, 52, 131, 148, 147, 30, 250, 232, 243, 52, 17, 18, 224, 29, 124, 242, 82, 204, 253, 135, 46, 112, 210, 245, 184, 88, 57, 226, 38, 110, 157, 64, 80, 221, 188, 18, 165, 241, 105, 197, 245, 5, 159, 155, 59, 235, 7, 95, 244, 149, 138, 50, 212, 234, 229, 209, 249, 36, 114, 125, 43, 96, 166, 128, 187, 117, 173, 182, 34, 33, 200, 152, 55, 19, 158, 168, 79, 238, 19, 194, 39, 137, 205, 210, 103, 170, 151, 187, 137, 247, 43, 145, 27, 254, 32, 215, 235, 104, 67, 29, 88, 210, 85, 237, 249, 183, 245, 149, 81, 119, 53, 141, 72, 91, 173, 87, 146, 178, 6, 31, 145, 1, 148, 184, 66, 58, 157, 78, 141, 228, 47, 221, 189, 205, 194, 122, 238, 227, 60, 252, 187, 176, 88, 118, 80, 220, 102, 131, 115, 250, 31, 243, 164, 216, 170, 40, 113, 229, 174, 183, 131, 46, 41, 81, 212, 235, 219, 183, 10, 219, 189, 174, 232, 85, 233, 245, 56, 5, 244, 206, 127, 48, 223, 218, 77, 42, 134, 117, 166, 195, 217, 62, 177, 36, 82, 105, 27, 165, 42, 12, 83, 121, 2, 31, 142, 227, 14, 180, 59, 107, 184, 237, 213, 116, 168, 68, 18, 29, 203, 76, 72, 239, 141, 248, 89, 239, 107, 190, 105, 46, 33, 10, 92, 227, 117, 152, 243, 227, 186, 117, 116, 187, 64, 206, 225, 253, 48, 213, 176, 54, 175, 165, 2, 3, 1, 0, 1, 2, 130, 1, 0, 16, 65, 128, 192, 85, 191, 188, 176, 221, 60, 229, 81, 24, 99, 21, 167, 146, 152, 96, 7, 1, 165, 177, 214, 225, 38, 61, 210, 127, 236, 133, 52, 252, 121, 16, 137, 184, 174, 254, 104, 49, 146, 128, 190, 116, 19, 167, 139, 59, 220, 255, 241, 246, 50, 239, 174, 19, 11, 28, 232, 83, 244, 191, 219, 215, 180, 215, 67, 126, 45, 191, 29, 189, 233, 195, 169, 134, 112, 37, 30, 93, 181, 185, 189, 214, 238, 55, 187, 15, 184, 187, 57, 68, 215, 155, 0, 87, 253, 71, 198, 151, 161, 190, 195, 103, 77, 83, 5, 158, 231, 235, 114, 218, 197, 137, 121, 12, 228, 40, 228, 122, 255, 16, 8, 64, 111, 175, 195, 124, 179, 164, 12, 140, 4, 133, 0, 169, 227, 103, 172, 136, 80, 148, 15, 175, 56, 77, 146, 112, 248, 46, 215, 230, 100, 144, 11, 38, 141, 188, 216, 65, 43, 185, 163, 240, 175, 68, 248, 139, 75, 147, 148, 216, 138, 86, 95, 124, 32, 228, 150, 25, 140, 127, 184, 148, 96, 192, 231, 74, 209, 112, 7, 62, 121, 206, 93, 234, 202, 84, 51, 55, 96, 122, 57, 211, 127, 163, 4, 23, 21, 99, 213, 70, 6, 195, 74, 192, 152, 25, 40, 179, 185, 214, 117, 247, 59, 58, 17, 124, 4, 149, 87, 171, 29, 73, 241, 59, 32, 97, 125, 33, 143, 181, 118, 111, 136, 191, 196, 225, 164, 246, 60, 69, 58, 103, 9, 2, 129, 129, 0, 251, 223, 73, 61, 82, 246, 130, 37, 252, 147, 206, 2, 77, 147, 215, 141, 200, 103, 150, 207, 183, 189, 230, 115, 75, 171, 91, 220, 9, 107, 68, 65, 25, 57, 227, 60, 189, 160, 240, 236, 124, 186, 114, 222, 217, 199, 141, 91, 159, 24, 128, 33, 209, 81, 100, 50, 250, 162, 143, 81, 62, 20, 225, 99, 12, 154, 254, 253, 239, 237, 73, 223, 174, 167, 65, 171, 130, 55, 157, 36, 120, 158, 0, 203, 102, 130, 96, 217, 61, 17, 188, 17, 208, 197, 172, 228, 106, 58, 62, 167, 155, 233, 253, 105, 159, 83, 234, 87, 12, 69, 176, 24, 211, 254, 143, 180, 36, 117, 151, 22, 35, 229, 29, 84, 45, 125, 211, 233, 2, 129, 129, 0, 212, 158, 185, 124, 53, 110, 240, 199, 46, 10, 34, 132, 248, 171, 103, 148, 233, 207, 53, 26, 95, 238, 254, 136, 53, 167, 212, 168, 63, 65, 62, 62, 53, 56, 141, 174, 13, 169, 150, 168, 41, 108, 241, 240, 199, 107, 123, 146, 124, 33, 29, 81, 167, 101, 10, 102, 219, 138, 42, 123, 5, 236, 159, 36, 255, 249, 85, 30, 231, 123, 56, 196, 40, 212, 245, 171, 215, 205, 253, 245, 106, 17, 20, 246, 192, 18, 227, 1, 176, 21, 30, 243, 144, 153, 255, 96, 27, 69, 92, 140, 222, 215, 174, 149, 233, 137, 33, 24, 211, 165, 68, 163, 153, 47, 172, 195, 205, 77, 167, 32, 85, 242, 149, 40, 202, 78, 148, 93, 2, 129, 128, 127, 102, 0, 136, 169, 55, 16, 187, 160, 23, 46, 227, 145, 83, 207, 122, 226, 188, 42, 215, 36, 44, 68, 173, 90, 191, 241, 53, 201, 249, 220, 203, 68, 180, 56, 17, 88, 37, 88, 215, 117, 123, 101, 112, 253, 28, 231, 142, 73, 89, 190, 45, 220, 207, 91, 108, 65, 196, 9, 211, 225, 31, 244, 165, 2, 18, 64, 26, 55, 202, 129, 76, 71, 238, 71, 212, 53, 136, 101, 206, 181, 199, 205, 197, 20, 51, 172, 67, 21, 149, 171, 216, 35, 14, 105, 150, 27, 121, 28, 137, 240, 222, 25, 20, 220, 199, 56, 72, 17, 104, 10, 91, 116, 151, 85, 30, 156, 54, 114, 86, 123, 143, 127, 23, 61, 239, 96, 73, 2, 129, 129, 0, 183, 25, 177, 118, 139, 181, 108, 88, 187, 7, 51, 78, 251, 174, 240, 86, 199, 59, 15, 234, 20, 175, 134, 54, 218, 174, 27, 110, 128, 122, 185, 21, 109, 83, 180, 127, 45, 120, 220, 43, 24, 241, 159, 48, 209, 43, 120, 145, 95, 160, 76, 213, 98, 168, 154, 218, 149, 57, 88, 247, 244, 212, 31, 1, 8, 7, 58, 12, 145, 32, 6, 116, 183, 13, 30, 59, 204, 110, 178, 195, 67, 130, 78, 97, 104, 107, 91, 94, 186, 186, 149, 154, 254, 237, 255, 132, 78, 55, 223, 73, 217, 38, 56, 77, 21, 126, 104, 40, 47, 43, 101, 254, 213, 14, 191, 99, 39, 76, 54, 30, 156, 165, 121, 90, 85, 78, 114, 105, 2, 129, 129, 0, 200, 239, 34, 10, 56, 121, 162, 40, 241, 159, 81, 105, 122, 125, 84, 100, 96, 208, 182, 191, 70, 46, 225, 193, 132, 167, 83, 153, 92, 205, 198, 168, 8, 79, 240, 159, 124, 206, 56, 173, 6, 231, 219, 182, 253, 191, 205, 8, 197, 185, 111, 243, 2, 163, 29, 18, 151, 116, 248, 163, 76, 121, 169, 254, 240, 3, 55, 139, 11, 8, 104, 212, 254, 5, 236, 134, 127, 129, 241, 76, 50, 106, 172, 8, 0, 44, 249, 7, 133, 175, 251, 164, 104, 139, 65, 237, 243, 41, 51, 190, 191, 204, 246, 109, 184, 51, 46, 30, 207, 208, 86, 171, 41, 238, 147, 14, 2, 213, 158, 107, 210, 148, 73, 215, 159, 83, 19, 84];



/// Starts a mock HTTPS server on 127.0.0.1 on an ephemeral port presenting a certificate
/// signed by `TEST_CA_CERT_PEM` for `localhost` and `127.0.0.1`.
pub fn start_mock_https_server(payload: Vec<u8>, max_connections: usize) -> (u16, thread::JoinHandle<()>) {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert = CertificateDer::from(TEST_SERVER_CERT_DER.to_vec());
    let key = PrivateKeyDer::Pkcs8(TEST_SERVER_KEY_DER.to_vec().into());

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("Failed to build ServerConfig");
    let server_config = Arc::new(server_config);

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind TcpListener");
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        for _ in 0..max_connections {
            if let Ok((mut tcp_stream, _)) = listener.accept() {
                let mut server_conn = match rustls::ServerConnection::new(server_config.clone()) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let mut tls_stream = rustls::Stream::new(&mut server_conn, &mut tcp_stream);
                let mut buf = [0; 1024];
                if tls_stream.read(&mut buf).is_ok() {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = tls_stream.write_all(response.as_bytes());
                    let _ = tls_stream.write_all(&payload);
                    let _ = tls_stream.flush();
                }
            }
        }
    });

    (port, handle)
}

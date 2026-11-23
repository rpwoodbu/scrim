use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Redirects standard I/O in the forked auxiliary child to `/dev/null` (closing as a
/// fallback) so callers capturing tool output via pipes never block waiting for EOF.
pub fn detach_stdio() {
    unsafe {
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_RDWR);
        if devnull >= 0 {
            let _ = libc::dup2(devnull, libc::STDIN_FILENO);
            let _ = libc::dup2(devnull, libc::STDOUT_FILENO);
            let _ = libc::dup2(devnull, libc::STDERR_FILENO);
            if devnull > libc::STDERR_FILENO {
                libc::close(devnull);
            }
        } else {
            libc::close(libc::STDIN_FILENO);
            libc::close(libc::STDOUT_FILENO);
            libc::close(libc::STDERR_FILENO);
        }
    }
}

pub fn report_usage(program_name: &str, target_path: &Path, log_paths: &[PathBuf]) {
    if log_paths.is_empty() {
        return;
    }
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // Child process: detach stdio, write telemetry in the background, and exit immediately.
        // Guard against panics so the child can never unwind into the parent's exec path.
        detach_stdio();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            write_usage_to_files(program_name, target_path, log_paths);
        }));
        unsafe { libc::_exit(0) };
    }
    // Parent process (or fork failure): return immediately without blocking
}

fn format_log_entry(program_name: &str, target_path: &Path) -> String {
    format!(
        "{{ \"tool\": \"{}\", \"target\": \"{}\", \"timestamp\": \"{}\" }}\n",
        program_name,
        target_path.display(),
        chrono::Utc::now().to_rfc3339()
    )
}

fn append_log_entry(log_path: &Path, log_entry: &str) {
    if let Some(parent) = log_path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = file.write_all(log_entry.as_bytes());
    }
}

/// Writes a single formatted telemetry entry to all configured destination files.
pub fn write_usage_to_files(program_name: &str, target_path: &Path, log_paths: &[PathBuf]) {
    let log_entry = format_log_entry(program_name, target_path);
    for log_path in log_paths {
        append_log_entry(log_path, &log_entry);
    }
}

// Public so the standalone `telemetry_test` crate can test logging without
// forking; package visibility in BUILD.bazel still restricts external callers.
pub fn write_usage_to_file(program_name: &str, target_path: &Path, log_path: &Path) {
    let log_entry = format_log_entry(program_name, target_path);
    append_log_entry(log_path, &log_entry);
}

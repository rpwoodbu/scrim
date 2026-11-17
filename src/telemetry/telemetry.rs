use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

const DEFAULT_TELEMETRY_LOG: &str = "/tmp/scrim_telemetry.log";

pub fn report_usage(program_name: &str, target_path: &Path) {
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // Child process: handle telemetry in the background and exit immediately
        write_usage_to_file(program_name, target_path, Path::new(DEFAULT_TELEMETRY_LOG));
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

// Public so the standalone `telemetry_test` crate can test logging without
// forking or writing to the global default path; package visibility in
// BUILD.bazel still restricts external callers.
pub fn write_usage_to_file(program_name: &str, target_path: &Path, log_path: &Path) {
    let log_entry = format_log_entry(program_name, target_path);

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = file.write_all(log_entry.as_bytes());
    }
}

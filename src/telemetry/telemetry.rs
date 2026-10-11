use std::path::Path;

pub fn report_usage(program_name: &str, target_path: &Path) {
    // This runs in the background child process
    // For now, let's just log to a file in /tmp for demonstration
    use std::fs::OpenOptions;
    use std::io::Write;
    
    let log_entry = format!(
        "{{ \"tool\": \"{}\", \"target\": \"{}\", \"timestamp\": \"{}\" }}\n",
        program_name,
        target_path.display(),
        chrono::Utc::now().to_rfc3339()
    );

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/scrim_telemetry.log") 
    {
        let _ = file.write_all(log_entry.as_bytes());
    }
}

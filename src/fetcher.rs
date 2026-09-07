use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::io::Write;

pub fn fetch_tool(tool_name: &str, url: &str, sha256: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let cache_dir = Path::new(&home).join(".cache/scrim/tools").join(tool_name).join(sha256);
    let target_path = cache_dir.join(tool_name);

    if target_path.exists() {
        return Ok(target_path);
    }

    fs::create_dir_all(&cache_dir)?;

    // Download with curl
    let status = Command::new("curl")
        .arg("-L")
        .arg("-o")
        .arg(&target_path)
        .arg(url)
        .status()?;

    if !status.success() {
        return Err("Failed to download tool".into());
    }

    // Verify SHA256 using sha256sum
    let mut child = Command::new("sha256sum")
        .arg("-c")
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{}  {}", sha256, target_path.display())?;
    }

    let status = child.wait()?;
    if !status.success() {
        let _ = fs::remove_file(&target_path);
        return Err("SHA256 verification failed".into());
    }

    // Ensure the binary is executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target_path, perms)?;
    }

    Ok(target_path)
}

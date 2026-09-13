use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_workspace_status_script() {
    let script_path = if let Ok(runfiles_dir) = env::var("RUNFILES_DIR") {
        Path::new(&runfiles_dir)
            .join("_main")
            .join("tools")
            .join("workspace_status.sh")
    } else {
        PathBuf::from("tools/workspace_status.sh")
    };
    assert!(
        script_path.exists(),
        "tools/workspace_status.sh not found at {:?}",
        script_path
    );

    // 1. Outside git directory
    let temp_dir_no_git = tempdir().unwrap();
    let output = Command::new(&script_path)
        .current_dir(temp_dir_no_git.path())
        .output()
        .expect("Failed to execute workspace_status.sh in empty directory");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), "STABLE_GIT_COMMIT");

    // 2. Inside a git repository (if git is available)
    let temp_dir_git = tempdir().unwrap();
    let git_path = temp_dir_git.path();
    let git_check = Command::new("git").arg("--version").output();
    if let Ok(check) = git_check {
        if check.status.success() {
            let init = Command::new("git")
                .args(["init"])
                .current_dir(git_path)
                .output()
                .unwrap();
            assert!(init.status.success());
            let _ = Command::new("git")
                .args(["config", "user.name", "Test User"])
                .current_dir(git_path)
                .output();
            let _ = Command::new("git")
                .args(["config", "user.email", "test@example.com"])
                .current_dir(git_path)
                .output();
            let commit_res = Command::new("git")
                .args(["commit", "--allow-empty", "-m", "initial commit"])
                .current_dir(git_path)
                .output()
                .unwrap();
            assert!(commit_res.status.success());

            let rev_parse = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(git_path)
                .output()
                .unwrap();
            let expected_hash = String::from_utf8(rev_parse.stdout).unwrap().trim().to_string();

            let output_git = Command::new(&script_path)
                .current_dir(git_path)
                .output()
                .expect("Failed to execute workspace_status.sh in git repo");
            assert!(output_git.status.success());
            let stdout_git = String::from_utf8(output_git.stdout).unwrap();
            assert_eq!(stdout_git.trim(), format!("STABLE_GIT_COMMIT {}", expected_hash));
        }
    }
}

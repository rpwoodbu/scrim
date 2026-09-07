use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn find_scrim_bench_bin() -> PathBuf {
    if let Ok(runfiles_dir) = env::var("RUNFILES_DIR") {
        let path = Path::new(&runfiles_dir).join("scrim").join("scrim_bench");
        if path.exists() { return path; }
        let path = Path::new(&runfiles_dir).join("__main__").join("scrim_bench");
        if path.exists() { return path; }
    }
    if let Ok(current_exe) = env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let path = dir.join("scrim_bench");
            if path.exists() { return path; }
        }
    }
    let path = PathBuf::from("bazel-bin/scrim_bench");
    if path.exists() { return path; }
    panic!("Could not locate 'scrim_bench' binary for validation");
}

#[test]
fn test_performance_threshold() {
    let bench_bin = find_scrim_bench_bin();
    let temp_dir = tempdir().unwrap();
    let criterion_home = temp_dir.path().join("criterion_home");
    fs::create_dir_all(&criterion_home).unwrap();

    println!("Running benchmark into: {}", criterion_home.display());

    let status = Command::new(&bench_bin)
        .arg("--bench")
        .env("CRITERION_HOME", &criterion_home)
        .status()
        .expect("Failed to execute scrim_bench");
    
    assert!(status.success(), "Benchmark execution failed");

    let estimates_file = criterion_home.join("e2e_shim_overhead/new/estimates.json");
    let content = fs::read_to_string(&estimates_file)
        .expect("Failed to read estimates.json. Did criterion run correctly?");

    // Quick and dirty manual JSON parsing to avoid pulling in serde_json just for this test
    let mean_marker = "\"mean\":";
    if let Some(mean_start) = content.find(mean_marker) {
        let remainder = &content[mean_start..];
        let pe_marker = "\"point_estimate\":";
        if let Some(pe_start) = remainder.find(pe_marker) {
            let value_start = pe_start + pe_marker.len();
            let value_end = remainder[value_start..].find(',').unwrap_or(remainder.len());
            let value_str = remainder[value_start..value_start + value_end].trim();
            
            let mean_ns: f64 = value_str.parse().expect("Failed to parse point_estimate");
            let mean_ms = mean_ns / 1_000_000.0;
            
            println!("E2E Shim Overhead: {:.3} ms", mean_ms);
            
            if mean_ms > 2.0 {
                panic!("FAIL: Overhead {:.3} ms exceeds the 2.0 ms threshold.", mean_ms);
            } else {
                println!("PASS: Overhead {:.3} ms is within the 2.0 ms threshold.", mean_ms);
            }
        } else {
            panic!("Could not find point_estimate in estimates.json");
        }
    } else {
        panic!("Could not find mean in estimates.json");
    }
}

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;
use test_helpers::find_scrim_bench_bin;

const THRESHOLD_MS: f64 = 2.0;

fn extract_point_estimate_ns(json: &str, metric: &str) -> f64 {
    let metric_marker = format!("\"{}\":", metric);
    let metric_start = json
        .find(&metric_marker)
        .unwrap_or_else(|| panic!("Could not find '{}' in estimates.json", metric));
    let after_key = &json[metric_start + metric_marker.len()..];
    let obj_rel_start = after_key
        .find('{')
        .unwrap_or_else(|| panic!("Could not find object for '{}'", metric));
    let obj_slice = &after_key[obj_rel_start..];
    let mut depth = 0usize;
    let mut obj_end = None;
    for (idx, ch) in obj_slice.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    obj_end = Some(idx + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let scoped = &obj_slice[..obj_end.unwrap_or_else(|| panic!("Unterminated object for '{}'", metric))];
    let pe_marker = "\"point_estimate\":";
    let pe_start = scoped
        .find(pe_marker)
        .unwrap_or_else(|| panic!("Could not find point_estimate for '{}'", metric));
    let value_start = pe_start + pe_marker.len();
    let value_end = scoped[value_start..]
        .find([',', '}'])
        .unwrap_or(scoped.len() - value_start);
    scoped[value_start..value_start + value_end]
        .trim()
        .parse::<f64>()
        .unwrap_or_else(|e| panic!("Failed to parse {} point_estimate: {}", metric, e))
}

fn read_estimates(criterion_home: &Path, bench_name: &str) -> (f64, f64) {
    let estimates_file = criterion_home
        .join(bench_name)
        .join("new/estimates.json");
    let content = fs::read_to_string(&estimates_file).unwrap_or_else(|e| {
        panic!(
            "Failed to read {}: {}. Did criterion run '{}'?",
            estimates_file.display(),
            e,
            bench_name
        )
    });
    let median_ns = extract_point_estimate_ns(&content, "median");
    let mean_ns = extract_point_estimate_ns(&content, "mean");
    (median_ns, mean_ns)
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

    let benchmarks = [
        "config_resolution",
        "config_parsing_minimal",
        "config_parsing_realistic",
        "resolve_tool",
        "e2e_shim_minimal",
        "e2e_shim_realistic",
    ];

    println!("\n=== Benchmark Summary (opt + musl) ===");
    println!("{:<28} {:>12} {:>12}", "Benchmark", "Median", "Mean");
    for name in benchmarks {
        let (median_ns, mean_ns) = read_estimates(&criterion_home, name);
        println!(
            "{:<28} {:>9.3} ms {:>9.3} ms",
            name,
            median_ns / 1_000_000.0,
            mean_ns / 1_000_000.0
        );
    }

    let (realistic_median_ns, realistic_mean_ns) =
        read_estimates(&criterion_home, "e2e_shim_realistic");
    let realistic_median_ms = realistic_median_ns / 1_000_000.0;
    let realistic_mean_ms = realistic_mean_ns / 1_000_000.0;

    if realistic_median_ms > THRESHOLD_MS {
        panic!(
            "FAIL: e2e_shim_realistic median net overhead {:.3} ms (mean {:.3} ms) exceeds the {:.1} ms threshold.",
            realistic_median_ms, realistic_mean_ms, THRESHOLD_MS
        );
    } else {
        println!(
            "\nPASS: e2e_shim_realistic median net overhead {:.3} ms (mean {:.3} ms) is within the {:.1} ms threshold.",
            realistic_median_ms, realistic_mean_ms, THRESHOLD_MS
        );
    }
}

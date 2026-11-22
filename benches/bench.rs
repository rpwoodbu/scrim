use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::tempdir;
use test_helpers::find_scrim_bin;

const TRUE_PLATFORMS: [(&str, &str); 3] = [
    (
        "0000000000000000000000000000000000000000000000000000000000000000",
        "https://example.com/downloads/true-1.0.0-linux-x86_64.tar.gz",
    ),
    (
        "1111111111111111111111111111111111111111111111111111111111111111",
        "https://example.com/downloads/true-1.0.0-linux-aarch64.tar.gz",
    ),
    (
        "2222222222222222222222222222222222222222222222222222222222222222",
        "https://example.com/downloads/true-1.0.0-macos-aarch64.tar.gz",
    ),
];

fn build_realistic_tools_yaml(extra_tool_count: usize) -> String {
    let mut yaml = format!(
        "telemetry:\n  file: {{}}\ntools:\n  true:\n    template: true_base\n  true_base:\n    archive_path: bin/true\n    platforms:\n      linux-x86_64:\n        url: {x86_url}\n        sha256: {x86_sha}\n      linux-aarch64:\n        url: {arm_url}\n        sha256: {arm_sha}\n      macos-aarch64:\n        url: {mac_url}\n        sha256: {mac_sha}\n",
        x86_sha = TRUE_PLATFORMS[0].0,
        x86_url = TRUE_PLATFORMS[0].1,
        arm_sha = TRUE_PLATFORMS[1].0,
        arm_url = TRUE_PLATFORMS[1].1,
        mac_sha = TRUE_PLATFORMS[2].0,
        mac_url = TRUE_PLATFORMS[2].1,
    );

    for i in 0..extra_tool_count {
        match i % 3 {
            0 => {
                yaml.push_str(&format!(
                    "  tool_{i:02}:\n    archive_path: pkg_{i}/bin/tool_{i}\n    platforms:\n      linux-x86_64:\n        url: https://example.com/releases/tool_{i}-v1.2.3-linux-x86_64.tar.gz\n        sha256: {i:064x}\n      linux-aarch64:\n        url: https://example.com/releases/tool_{i}-v1.2.3-linux-aarch64.tar.gz\n        sha256: {i:064x}\n      macos-aarch64:\n        url: https://example.com/releases/tool_{i}-v1.2.3-macos-aarch64.tar.gz\n        sha256: {i:064x}\n",
                ));
            }
            1 => {
                let target_idx = i - 1;
                yaml.push_str(&format!(
                    "  tool_{i:02}:\n    template: tool_{target_idx:02}\n    archive_path: pkg_{target_idx}/bin/tool_{i}\n",
                ));
            }
            _ => {
                yaml.push_str(&format!(
                    "  tool_{i:02}:\n    platforms:\n      linux-x86_64:\n        system_path: /usr/bin/tool_{i}\n        url: https://example.com/releases/tool_{i}.tar.gz\n        sha512: {i:0128x}\n      any:\n        system_path: /usr/local/bin/tool_{i}\n",
                ));
            }
        }
    }

    yaml
}

fn setup_realistic_user_config(xdg_config: &Path) {
    let user_scrim = xdg_config.join("scrim");
    fs::create_dir_all(&user_scrim).unwrap();
    fs::write(
        user_scrim.join("imported.yaml"),
        build_realistic_tools_yaml(48),
    )
    .unwrap();
    fs::write(
        user_scrim.join("scrim.yaml"),
        "imports:\n  - path: imported.yaml\n    optional: true\n    precedence: after\ntools:\n  git:\n    platforms:\n      any:\n        system_path: /usr/bin/git\n",
    )
    .unwrap();
}

fn isolated_command(
    program: &Path,
    cwd: &Path,
    home: &Path,
    xdg_config: &Path,
    xdg_cache: &Path,
    xdg_state: &Path,
) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg_config)
        .env("XDG_CACHE_HOME", xdg_cache)
        .env("XDG_STATE_HOME", xdg_state);
    cmd
}

fn bench_config_resolution(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let home_dir = dir.path().join("home");
    let xdg_config = dir.path().join("xdg_config");
    fs::create_dir_all(&home_dir).unwrap();
    setup_realistic_user_config(&xdg_config);

    let repo_root = dir.path().join("repo");
    let child = repo_root.join("a/b/c");
    fs::create_dir_all(repo_root.join(".git")).unwrap();
    fs::create_dir_all(&child).unwrap();
    fs::write(repo_root.join("scrim.yaml"), "{}").unwrap();

    assert!(config::load_config(&child, Some(&home_dir), Some(&xdg_config)).is_ok());

    c.bench_function("config_resolution", |b| {
        b.iter(|| config::load_config(black_box(&child), Some(&home_dir), Some(&xdg_config)))
    });
}

fn bench_config_parsing(c: &mut Criterion) {
    let minimal_yaml = r#"
telemetry:
  file: {}
default_platform: linux-x86_64
tools:
  node:
    system_path: /usr/local/bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
"#;
    let realistic_yaml = build_realistic_tools_yaml(48);

    let dir = tempdir().unwrap();
    let minimal_path = dir.path().join("minimal.yaml");
    let realistic_path = dir.path().join("realistic.yaml");
    fs::write(&minimal_path, minimal_yaml).unwrap();
    fs::write(&realistic_path, realistic_yaml).unwrap();

    assert!(config::read_config(&minimal_path).is_ok());
    assert!(config::read_config(&realistic_path).is_ok());

    c.bench_function("config_parsing_minimal", |b| {
        b.iter(|| config::read_config(black_box(&minimal_path)))
    });
    c.bench_function("config_parsing_realistic", |b| {
        b.iter(|| config::read_config(black_box(&realistic_path)))
    });
}

fn bench_resolver(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("scrim.yaml");
    fs::write(&config_path, build_realistic_tools_yaml(48)).unwrap();
    let config = config::read_config(&config_path).unwrap();

    assert!(matches!(
        resolver::resolve_tool("true", &config),
        Ok(Some(_))
    ));

    c.bench_function("resolve_tool", |b| {
        b.iter(|| resolver::resolve_tool(black_box("true"), black_box(&config)))
    });
}

fn run_paired_e2e_iters(
    iters: u64,
    shim_path: &Path,
    cwd: &Path,
    home: &Path,
    xdg_config: &Path,
    xdg_cache: &Path,
    xdg_state: &Path,
) -> Duration {
    let mut direct_cmd = isolated_command(
        Path::new("/bin/true"),
        cwd,
        home,
        xdg_config,
        xdg_cache,
        xdg_state,
    );
    let mut shim_cmd = isolated_command(shim_path, cwd, home, xdg_config, xdg_cache, xdg_state);

    let mut direct_ns: u128 = 0;
    let mut shim_ns: u128 = 0;
    for i in 0..iters {
        if i % 2 == 0 {
            let t0 = Instant::now();
            let st_d = direct_cmd.status().unwrap();
            direct_ns += t0.elapsed().as_nanos();
            assert!(st_d.success());

            let t1 = Instant::now();
            let st_s = shim_cmd.status().unwrap();
            shim_ns += t1.elapsed().as_nanos();
            assert!(st_s.success());
        } else {
            let t1 = Instant::now();
            let st_s = shim_cmd.status().unwrap();
            shim_ns += t1.elapsed().as_nanos();
            assert!(st_s.success());

            let t0 = Instant::now();
            let st_d = direct_cmd.status().unwrap();
            direct_ns += t0.elapsed().as_nanos();
            assert!(st_d.success());
        }
    }

    let net_ns = (shim_ns as i128 - direct_ns as i128).max(1) as u64;
    Duration::from_nanos(net_ns)
}

fn bench_e2e_shim(c: &mut Criterion) {
    let scrim_bin = find_scrim_bin();

    // 1. Minimal 1-tool system_path baseline (telemetry disabled)
    {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let xdg_config = dir.path().join("xdg_config");
        let xdg_cache = dir.path().join("xdg_cache");
        let xdg_state = dir.path().join("xdg_state");
        let repo = dir.path().join("repo");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg_config).unwrap();
        fs::create_dir_all(&xdg_cache).unwrap();
        fs::create_dir_all(&xdg_state).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();

        fs::write(
            repo.join("scrim.yaml"),
            "telemetry: false\ntools:\n  true:\n    platforms:\n      any:\n        system_path: /bin/true\n",
        )
        .unwrap();

        let shim_path = dir.path().join("true");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

        let check = isolated_command(&shim_path, &repo, &home, &xdg_config, &xdg_cache, &xdg_state)
            .status()
            .unwrap();
        assert!(check.success());

        c.bench_function("e2e_shim_minimal", |b| {
            b.iter_custom(|iters| {
                run_paired_e2e_iters(
                    iters,
                    &shim_path,
                    &repo,
                    &home,
                    &xdg_config,
                    &xdg_cache,
                    &xdg_state,
                )
            })
        });
    }

    // 2. Realistic 50-tool layered + imported + templated cached-fetch + file telemetry enabled
    {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let xdg_config = dir.path().join("xdg_config");
        let xdg_cache = dir.path().join("xdg_cache");
        let xdg_state = dir.path().join("xdg_state");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg_state).unwrap();
        setup_realistic_user_config(&xdg_config);

        // Pre-populate cached binary + .url file for all supported platforms
        let tools_cache = xdg_cache.join("scrim/tools");
        for &(digest, url) in &TRUE_PLATFORMS {
            let cache_key = format!("sha256:{}", digest);
            let cached_bin_dir = tools_cache.join(&cache_key).join("bin");
            fs::create_dir_all(&cached_bin_dir).unwrap();
            fs::copy("/bin/true", cached_bin_dir.join("true")).unwrap();
            fs::write(
                tools_cache.join(format!("{}.url", cache_key)),
                format!("{}\n", url),
            )
            .unwrap();
        }

        // Nested workspace inside .git repo (2 levels deep)
        let repo = dir.path().join("repo");
        let sub = repo.join("packages/app");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(&sub).unwrap();
        fs::write(repo.join("scrim.yaml"), "{}\n").unwrap();
        fs::write(sub.join("scrim.yaml"), "{}\n").unwrap();

        let shim_path = dir.path().join("true");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

        // Verify execution succeeds and telemetry log is populated
        let check = isolated_command(&shim_path, &sub, &home, &xdg_config, &xdg_cache, &xdg_state)
            .status()
            .unwrap();
        assert!(check.success());

        let tel_log = xdg_state.join("scrim/telemetry.log");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if let Ok(content) = fs::read_to_string(&tel_log) {
                if content.contains("\"tool\": \"true\"") {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            fs::read_to_string(&tel_log)
                .map(|c| c.contains("\"tool\": \"true\""))
                .unwrap_or(false),
            "Realistic benchmark fixture did not write telemetry log at {}",
            tel_log.display()
        );

        c.bench_function("e2e_shim_realistic", |b| {
            b.iter_custom(|iters| {
                run_paired_e2e_iters(
                    iters,
                    &shim_path,
                    &sub,
                    &home,
                    &xdg_config,
                    &xdg_cache,
                    &xdg_state,
                )
            })
        });
    }
}

criterion_group!(
    benches,
    bench_config_resolution,
    bench_config_parsing,
    bench_resolver,
    bench_e2e_shim
);
criterion_main!(benches);

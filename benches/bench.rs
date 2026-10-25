use criterion::{criterion_group, criterion_main, Criterion, black_box};
use config;
use resolver;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
use test_helpers::find_scrim_bin;

fn bench_config_resolution(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("scrim.yaml");
    fs::write(&config_path, "{}").unwrap();

    let child = dir.path().join("a/b/c/d/e");
    fs::create_dir_all(&child).unwrap();

    c.bench_function("config_resolution", |b| {
        b.iter(|| config::load_config(black_box(&child), Some(dir.path()), None))
    });
}

fn bench_config_parsing(c: &mut Criterion) {
    let yaml = r#"
telemetry: true
tools:
  node:
    system_path: /usr/local/bin/node
  go:
    url: https://go.dev/dl/go1.21.5.linux-amd64.tar.gz
    sha256: 285c1f0624022839446d32
"#;
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("scrim.yaml");
    fs::write(&config_path, yaml).unwrap();

    c.bench_function("config_parsing", |b| {
        b.iter(|| config::read_config(black_box(&config_path)))
    });
}

fn bench_resolver(c: &mut Criterion) {
    let yaml = r#"
telemetry: true
tools:
  node:
    system_path: /usr/local/bin/node
"#;
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("scrim.yaml");
    fs::write(&config_path, yaml).unwrap();
    let config = config::read_config(&config_path).unwrap();

    c.bench_function("resolve_tool", |b| {
        b.iter(|| {
            resolver::resolve_tool(
                black_box("node"),
                black_box(&config),
            )
        })
    });
}

fn bench_e2e_shim(c: &mut Criterion) {
    let scrim_bin = find_scrim_bin();
    let dir = tempdir().unwrap();
    
    // Create scrim.yaml pointing to 'true'
    let scrim_yaml = dir.path().join("scrim.yaml");
    let config_content = r#"
telemetry: false
tools:
  true:
    system_path: /bin/true
"#;
    fs::write(&scrim_yaml, config_content).unwrap();

    // Create symlink
    let shim_path = dir.path().join("true");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&scrim_bin, &shim_path).unwrap();

    c.bench_function("e2e_shim_overhead", |b| {
        b.iter(|| {
            let _ = Command::new(&shim_path)
                .current_dir(dir.path())
                .status()
                .unwrap();
        })
    });
}

criterion_group!(
    benches, 
    bench_config_resolution, 
    bench_config_parsing, 
    bench_resolver,
    bench_e2e_shim
);
criterion_main!(benches);

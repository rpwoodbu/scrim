use cli::{parse_management_cli, CliCommand, CliParseResult};
use config::Config;
use fetcher::{cached_binary_path, fetch_tool};
use linker::create_links;
use logger::scrim_error;
use lookup::find_in_path;
use resolver::{resolve_tool_for_platform_with_fallback, Resolution};
use version::format_version;

use std::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

// Replace musl's default allocator (mallocng) with dlmalloc to avoid mmap/munmap
// syscall churn during configuration parsing on process startup.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

fn main() {
    let args: Vec<std::ffi::OsString> = env::args_os().collect();
    if args.is_empty() {
        return;
    }

    let program_name = Path::new(&args[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("scrim")
        .to_string();

    let app_env = AppEnvironment::capture();

    if program_name == "scrim" {
        handle_management_command(&args, &app_env);
    } else {
        proxy_command(&program_name, &args, &app_env);
    }
}

struct AppEnvironment {
    home_dir: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    xdg_cache_home: Option<PathBuf>,
    xdg_state_home: Option<PathBuf>,
    path: Option<std::ffi::OsString>,
    current_exe: Option<PathBuf>,
}

impl AppEnvironment {
    fn capture() -> Self {
        Self {
            home_dir: env::var_os("HOME").map(PathBuf::from),
            xdg_config_home: env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            xdg_cache_home: env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
            xdg_state_home: env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            path: env::var_os("PATH"),
            current_exe: env::current_exe().ok(),
        }
    }
}

const BINARY_COMMIT: Option<&str> = option_env!("SCRIM_GIT_COMMIT");

fn load_configuration_with_writes(
    app_env: &AppEnvironment,
) -> (Config, Vec<config::PendingCacheWrite>) {
    let cwd = match env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            scrim_error!("Failed to get current directory: {}", e);
            std::process::exit(1);
        }
    };
    match config::load_config_with_cache(
        &cwd,
        app_env.home_dir.as_deref(),
        app_env.xdg_config_home.as_deref(),
        app_env.xdg_cache_home.as_deref(),
        BINARY_COMMIT,
    ) {
        Ok(res) => res,
        Err(e) => {
            scrim_error!("{}", e);
            std::process::exit(1);
        }
    }
}

fn load_configuration(app_env: &AppEnvironment) -> Config {
    let (config, pending) = load_configuration_with_writes(app_env);
    for write in pending {
        write.persist();
    }
    config
}

fn handle_management_command(args: &[std::ffi::OsString], app_env: &AppEnvironment) {
    match parse_management_cli(args) {
        CliParseResult::Command(CliCommand::Version) => {
            println!("{}", format_version(BINARY_COMMIT));
        }
        CliParseResult::Command(CliCommand::Config {
            tool,
            platform,
            allow_fallback,
        }) => {
            let target_platform = match platform {
                Some(p_str) => match config::Platform::parse(&p_str) {
                    Ok(p) => p,
                    Err(e) => {
                        scrim_error!("{}", e);
                        std::process::exit(1);
                    }
                },
                None => match config::Platform::host() {
                    Ok(p) => p,
                    Err(e) => {
                        scrim_error!("{}", e);
                        std::process::exit(1);
                    }
                },
            };
            let config = load_configuration(app_env);
            if let Some(tool_name) = tool {
                match resolver::resolve_effective_tool_for_platform(
                    &tool_name,
                    &config,
                    &target_platform,
                    allow_fallback,
                ) {
                    Ok(Some(eff_tool)) => match eff_tool.to_yaml() {
                        Ok(yaml) => print!("{}", yaml),
                        Err(e) => {
                            scrim_error!("Failed to serialize configuration: {}", e);
                            std::process::exit(1);
                        }
                    },
                    Ok(None) => {
                        if config.tools.contains_key(&tool_name) {
                            scrim_error!(
                                "Tool '{}' is not configured for platform '{}'",
                                tool_name,
                                target_platform
                            );
                        } else {
                            scrim_error!("Tool '{}' not found in config", tool_name);
                        }
                        std::process::exit(1);
                    }
                    Err(e) => {
                        scrim_error!("{}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                let res = resolver::resolve_effective_config_for_platform(
                    &config,
                    &target_platform,
                    allow_fallback,
                );
                match res.config.to_yaml() {
                    Ok(yaml) => print!("{}", yaml),
                    Err(e) => {
                        scrim_error!("Failed to serialize configuration: {}", e);
                        std::process::exit(1);
                    }
                }
                if !res.errors.is_empty() {
                    for err in res.errors {
                        scrim_error!("{}", err);
                    }
                    std::process::exit(1);
                }
            }
        }
        CliParseResult::Command(CliCommand::Links {
            dir,
            dry_run,
            prune,
        }) => {
            let scrim_exe = match &app_env.current_exe {
                Some(exe) => exe.clone(),
                None => {
                    scrim_error!("Failed to resolve current executable path");
                    std::process::exit(1);
                }
            };
            let config = load_configuration(app_env);
            match create_links(&dir, &scrim_exe, &config, dry_run, prune) {
                Ok(modified) => {
                    if dry_run && modified {
                        std::process::exit(2);
                    }
                }
                Err(e) => {
                    scrim_error!("{}", e);
                    std::process::exit(1);
                }
            }
        }
        CliParseResult::Command(CliCommand::Run {
            tool,
            args,
            allow_fallback,
        }) => {
            execute_tool(&tool, &args, allow_fallback, app_env);
        }
        CliParseResult::Command(CliCommand::Which {
            tool,
            platform,
            allow_fallback,
        }) => {
            which_tool(&tool, platform.as_deref(), allow_fallback, app_env);
        }
        CliParseResult::Help(text) => {
            print!("{}", text);
        }
        CliParseResult::Error(err) => {
            scrim_error!("{}", err);
            std::process::exit(1);
        }
    }
}

fn proxy_command(program_name: &str, args: &[std::ffi::OsString], app_env: &AppEnvironment) -> ! {
    let tool_args = if args.len() > 1 { &args[1..] } else { &[] };
    let (config, pending_cache_writes) = load_configuration_with_writes(app_env);

    if !config.tools.contains_key(program_name) {
        if let Some(target_path) = find_in_path(
            program_name,
            app_env.path.as_deref(),
            app_env.current_exe.as_deref(),
        ) {
            execute_binary(
                program_name,
                &target_path,
                tool_args,
                &config.telemetry,
                &pending_cache_writes,
                app_env,
            );
        } else {
            scrim_error!("{}: command not found", program_name);
            std::process::exit(127);
        }
    }

    let host_platform = match config::Platform::host() {
        Ok(p) => p,
        Err(e) => {
            scrim_error!("{}", e);
            std::process::exit(1);
        }
    };
    let target_path =
        resolve_target_binary_path(program_name, &config, &host_platform, app_env, true, false);
    execute_binary(
        program_name,
        &target_path,
        tool_args,
        &config.telemetry,
        &pending_cache_writes,
        app_env,
    );
}

fn execute_tool(
    program_name: &str,
    tool_args: &[std::ffi::OsString],
    allow_fallback: bool,
    app_env: &AppEnvironment,
) -> ! {
    let host_platform = match config::Platform::host() {
        Ok(p) => p,
        Err(e) => {
            scrim_error!("{}", e);
            std::process::exit(1);
        }
    };
    let (config, pending_cache_writes) = load_configuration_with_writes(app_env);
    let target_path = resolve_target_binary_path(
        program_name,
        &config,
        &host_platform,
        app_env,
        true,
        allow_fallback,
    );
    execute_binary(
        program_name,
        &target_path,
        tool_args,
        &config.telemetry,
        &pending_cache_writes,
        app_env,
    );
}

fn execute_binary(
    program_name: &str,
    target_path: &Path,
    tool_args: &[std::ffi::OsString],
    telemetry: &config::TelemetryConfig,
    pending_cache_writes: &[config::PendingCacheWrite],
    app_env: &AppEnvironment,
) -> ! {
    // Prepare the command
    let mut cmd = Command::new(target_path);

    // Pass through all arguments
    if !tool_args.is_empty() {
        cmd.args(tool_args);
    }

    let telemetry_paths = telemetry.resolve_file_paths(
        app_env.xdg_state_home.as_deref(),
        app_env.home_dir.as_deref(),
    );
    if !telemetry_paths.is_empty() || !pending_cache_writes.is_empty() {
        spawn_auxiliary_child(program_name, target_path, &telemetry_paths, pending_cache_writes);
    }

    // Replace the current process with the target tool
    let err = cmd.exec();

    // If exec returns, it failed
    scrim_error!("Failed to execute tool: {}", err);
    std::process::exit(126);
}

fn spawn_auxiliary_child(
    program_name: &str,
    target_path: &Path,
    telemetry_paths: &[PathBuf],
    pending_cache_writes: &[config::PendingCacheWrite],
) {
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        telemetry::detach_stdio();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for write in pending_cache_writes {
                write.persist();
            }
            if !telemetry_paths.is_empty() {
                telemetry::write_usage_to_files(program_name, target_path, telemetry_paths);
            }
        }));
        unsafe { libc::_exit(0) };
    }
}

fn which_tool(
    program_name: &str,
    platform_arg: Option<&str>,
    allow_fallback: bool,
    app_env: &AppEnvironment,
) {
    let target_platform = match platform_arg {
        Some(p_str) => match config::Platform::parse(p_str) {
            Ok(p) => p,
            Err(e) => {
                scrim_error!("{}", e);
                std::process::exit(1);
            }
        },
        None => match config::Platform::host() {
            Ok(p) => p,
            Err(e) => {
                scrim_error!("{}", e);
                std::process::exit(1);
            }
        },
    };
    let config = load_configuration(app_env);
    let target_path = resolve_target_binary_path(
        program_name,
        &config,
        &target_platform,
        app_env,
        false,
        allow_fallback,
    );
    println!("{}", target_path.display());
}

fn resolve_target_binary_path(
    program_name: &str,
    config: &Config,
    platform: &config::Platform,
    app_env: &AppEnvironment,
    fetch_if_missing: bool,
    allow_fallback: bool,
) -> PathBuf {
    let resolution = match resolve_tool_for_platform_with_fallback(
        program_name,
        config,
        platform,
        allow_fallback,
    ) {
        Ok(Some(res)) => res,
        Ok(None) => {
            if allow_fallback {
                if let Some(target_path) = find_in_path(
                    program_name,
                    app_env.path.as_deref(),
                    app_env.current_exe.as_deref(),
                ) {
                    return target_path;
                }
            }
            if config.tools.contains_key(program_name) {
                scrim_error!(
                    "Tool '{}' is not configured for platform '{}'",
                    program_name,
                    platform
                );
            } else {
                scrim_error!("Tool '{}' not found in config", program_name);
            }
            std::process::exit(1);
        }
        Err(e) => {
            scrim_error!("{}", e);
            std::process::exit(1);
        }
    };

    let fetch_res = match resolution {
        Resolution::SystemPath { path, fallback } => {
            if path.exists() || fallback.is_none() {
                return path;
            }
            fallback.unwrap()
        }
        Resolution::Fetch(fetch_res) => fetch_res,
    };

    let base_cache = config::resolve_cache_dir(
        app_env.xdg_cache_home.as_deref(),
        app_env.home_dir.as_deref(),
    );

    if fetch_if_missing {
        match fetch_tool(
            program_name,
            &fetch_res.url,
            fetch_res.digest.algorithm.name(),
            &fetch_res.digest.value,
            fetch_res.archive_path.as_deref(),
            &base_cache,
        ) {
            Ok(p) => p,
            Err(e) => {
                scrim_error!("Failed to fetch tool '{}': {}", program_name, e);
                std::process::exit(1);
            }
        }
    } else {
        match cached_binary_path(
            program_name,
            &fetch_res.url,
            fetch_res.digest.algorithm.name(),
            &fetch_res.digest.value,
            fetch_res.archive_path.as_deref(),
            &base_cache,
        ) {
            Ok(p) => p,
            Err(e) => {
                scrim_error!("{}", e);
                std::process::exit(1);
            }
        }
    }
}

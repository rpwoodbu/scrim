use cli::{parse_management_cli, CliCommand, CliParseResult};
use config::Config;
use fetcher::{cached_binary_path, fetch_tool};
use linker::create_links;
use logger::scrim_error;
use resolver::{resolve_tool_for_platform, Resolution};
use telemetry::report_usage;
use version::format_version;

use std::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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
}

impl AppEnvironment {
    fn capture() -> Self {
        Self {
            home_dir: env::var_os("HOME").map(PathBuf::from),
            xdg_config_home: env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            xdg_cache_home: env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        }
    }
}

const BINARY_COMMIT: Option<&str> = option_env!("SCRIM_GIT_COMMIT");

fn load_configuration(app_env: &AppEnvironment) -> Config {
    let cwd = match env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            scrim_error!("Failed to get current directory: {}", e);
            std::process::exit(1);
        }
    };
    match config::load_config(
        &cwd,
        app_env.home_dir.as_deref(),
        app_env.xdg_config_home.as_deref(),
    ) {
        Ok(c) => c,
        Err(e) => {
            scrim_error!("{}", e);
            std::process::exit(1);
        }
    }
}

fn handle_management_command(args: &[std::ffi::OsString], app_env: &AppEnvironment) {
    match parse_management_cli(args) {
        CliParseResult::Command(CliCommand::Version) => {
            println!("{}", format_version(BINARY_COMMIT));
        }
        CliParseResult::Command(CliCommand::Config { tool, platform }) => {
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
            match config.format_yaml_for_platform(tool.as_deref(), &target_platform) {
                Ok(yaml) => print!("{}", yaml),
                Err(e) => {
                    scrim_error!("{}", e);
                    std::process::exit(1);
                }
            }
        }
        CliParseResult::Command(CliCommand::Links { dir, dry_run }) => {
            let scrim_exe = match env::current_exe() {
                Ok(exe) => exe,
                Err(e) => {
                    scrim_error!("Failed to resolve current executable path: {}", e);
                    std::process::exit(1);
                }
            };
            let config = load_configuration(app_env);
            match create_links(&dir, &scrim_exe, &config, dry_run) {
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
        CliParseResult::Command(CliCommand::Run { tool, args }) => {
            execute_tool(&tool, &args, app_env);
        }
        CliParseResult::Command(CliCommand::Which { tool, platform }) => {
            which_tool(&tool, platform.as_deref(), app_env);
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
    execute_tool(program_name, tool_args, app_env);
}

fn execute_tool(program_name: &str, tool_args: &[std::ffi::OsString], app_env: &AppEnvironment) -> ! {
    let host_platform = match config::Platform::host() {
        Ok(p) => p,
        Err(e) => {
            scrim_error!("{}", e);
            std::process::exit(1);
        }
    };
    let config = load_configuration(app_env);
    let target_path =
        resolve_target_binary_path(program_name, &config, &host_platform, app_env, true);

    // Prepare the command
    let mut cmd = Command::new(&target_path);

    // Pass through all arguments
    if !tool_args.is_empty() {
        cmd.args(tool_args);
    }

    // Report usage in the background (non-blocking) if enabled in config
    if config.telemetry_enabled() {
        report_usage(program_name, &target_path);
    }

    // Replace the current process with the target tool
    let err = cmd.exec();

    // If exec returns, it failed
    scrim_error!("Failed to execute tool: {}", err);
    std::process::exit(1);
}

fn which_tool(program_name: &str, platform_arg: Option<&str>, app_env: &AppEnvironment) {
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
    let target_path =
        resolve_target_binary_path(program_name, &config, &target_platform, app_env, false);
    println!("{}", target_path.display());
}

fn resolve_target_binary_path(
    program_name: &str,
    config: &Config,
    platform: &config::Platform,
    app_env: &AppEnvironment,
    fetch_if_missing: bool,
) -> PathBuf {
    let resolution = match resolve_tool_for_platform(program_name, config, platform) {
        Ok(Some(res)) => res,
        Ok(None) => {
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

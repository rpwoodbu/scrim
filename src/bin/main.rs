use cli::{parse_management_cli, CliCommand, CliParseResult};
use config::Config;
use fetcher::fetch_tool;
use linker::create_links;
use logger::scrim_error;
use resolver::{resolve_tool, FetchResolution, Resolution};
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
    let cwd = env::current_dir().expect("Failed to get current directory");
    match config::load_config(
        &cwd,
        app_env.home_dir.as_deref(),
        app_env.xdg_config_home.as_deref(),
    ) {
        Ok(c) => c,
        Err(e) => {
            scrim_error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_management_command(args: &[std::ffi::OsString], app_env: &AppEnvironment) {
    match parse_management_cli(args) {
        CliParseResult::Command(CliCommand::Version) => {
            println!("{}", format_version(BINARY_COMMIT));
        }
        CliParseResult::Command(CliCommand::Config { tool }) => {
            let config = load_configuration(app_env);
            if let Some(tool_name) = tool {
                match config.tools.get(&tool_name) {
                    Some(tool_config) => match serde_yaml::to_string(tool_config) {
                        Ok(yaml) => print!("{}", yaml),
                        Err(e) => {
                            scrim_error!("Failed to serialize configuration: {}", e);
                            std::process::exit(1);
                        }
                    },
                    None => {
                        scrim_error!("Tool '{}' not found in config", tool_name);
                        std::process::exit(1);
                    }
                }
            } else {
                match serde_yaml::to_string(&config) {
                    Ok(yaml) => print!("{}", yaml),
                    Err(e) => {
                        scrim_error!("Failed to serialize configuration: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        CliParseResult::Command(CliCommand::Links { dir, dry_run }) => {
            let dir_arg = match dir {
                Some(ref d) => d.as_path(),
                None => {
                    scrim_error!("directory argument required for links command");
                    std::process::exit(1);
                }
            };
            let scrim_exe = match env::current_exe() {
                Ok(exe) => exe,
                Err(e) => {
                    scrim_error!("Failed to resolve current executable path: {}", e);
                    std::process::exit(1);
                }
            };
            let config = load_configuration(app_env);
            match create_links(dir_arg, &scrim_exe, &config, dry_run) {
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
            let tool_name = match tool {
                Some(t) => t,
                None => {
                    scrim_error!("tool name argument required for run command");
                    std::process::exit(1);
                }
            };
            execute_tool(&tool_name, &args, app_env);
        }
        CliParseResult::Help(text) => {
            print!("{}", text);
        }
        CliParseResult::Error(err) => {
            let trimmed_err = err.strip_prefix("error: ").unwrap_or(&err);
            scrim_error!("{}", trimmed_err);
            std::process::exit(1);
        }
    }
}

fn proxy_command(program_name: &str, args: &[std::ffi::OsString], app_env: &AppEnvironment) -> ! {
    let tool_args = if args.len() > 1 { &args[1..] } else { &[] };
    execute_tool(program_name, tool_args, app_env);
}

fn execute_tool(program_name: &str, tool_args: &[std::ffi::OsString], app_env: &AppEnvironment) -> ! {
    let config = load_configuration(app_env);
    let resolution = resolve_tool(program_name, Some(&config));

    match resolution {
        Ok(Some(res)) => {
            let target_path = match res {
                Resolution::SystemPath { path, fallback } => {
                    if path.exists() || fallback.is_none() {
                        path
                    } else {
                        execute_fetch(program_name, &fallback.unwrap(), app_env)
                    }
                }
                Resolution::Fetch(fetch_res) => execute_fetch(program_name, &fetch_res, app_env),
            };

            // Prepare the command
            let mut cmd = Command::new(&target_path);

            // Pass through all arguments
            if !tool_args.is_empty() {
                cmd.args(tool_args);
            }

            // Fork for telemetry (non-blocking) if enabled in config
            let enable_telemetry = config.telemetry_enabled();
            if enable_telemetry {
                unsafe {
                    let pid = libc::fork();
                    if pid == 0 {
                        // Child process: handle telemetry and exit
                        report_usage(program_name, &target_path);
                        libc::_exit(0);
                    }
                    // Parent process continues to exec
                }
            }

            // Replace the current process with the target tool
            let err = cmd.exec();

            // If exec returns, it failed
            scrim_error!("Failed to execute tool: {}", err);
            std::process::exit(1);
        }
        Ok(None) => {
            scrim_error!("Tool '{}' not found in config", program_name);
            std::process::exit(1);
        }
        Err(e) => {
            scrim_error!("{}", e);
            std::process::exit(1);
        }
    }
}

fn execute_fetch(program_name: &str, fetch: &FetchResolution, app_env: &AppEnvironment) -> PathBuf {
    let base_cache = config::resolve_cache_dir(
        app_env.xdg_cache_home.as_deref(),
        app_env.home_dir.as_deref(),
    );
    let tools_dir = base_cache.join("tools");
    let cache_dir_name = format!("{}:{}", fetch.digest.algorithm.name(), fetch.digest.value);
    let cache_dir = tools_dir.join(cache_dir_name);
    match fetch_tool(
        program_name,
        &fetch.url,
        fetch.digest.algorithm.name(),
        &fetch.digest.value,
        fetch.archive_path.as_deref(),
        &cache_dir,
    ) {
        Ok(p) => p,
        Err(e) => {
            scrim_error!("Failed to fetch tool '{}': {}", program_name, e);
            std::process::exit(1);
        }
    }
}

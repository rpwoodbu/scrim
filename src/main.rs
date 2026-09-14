use scrim_lib::{config, fetcher, resolver, telemetry};

use std::env;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::path::{Path, PathBuf};

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

    if program_name == "scrim" {
        handle_management_command(&args);
    } else {
        proxy_command(&program_name, &args);
    }
}

fn get_home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

const BINARY_COMMIT: Option<&str> = option_env!("SCRIM_GIT_COMMIT");


fn load_configuration() -> (config::Config, Option<PathBuf>) {
    let cwd = env::current_dir().expect("Failed to get current directory");
    let home_dir = get_home_dir();
    let config = match config::load_config(&cwd, home_dir.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            scrim_lib::scrim_error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };
    (config, home_dir)
}

fn handle_management_command(args: &[std::ffi::OsString]) {
    match scrim_lib::cli::parse_management_cli(args) {
        scrim_lib::cli::CliParseResult::Command(scrim_lib::cli::CliCommand::Version) => {
            println!("{}", scrim_lib::format_version(BINARY_COMMIT));
        }
        scrim_lib::cli::CliParseResult::Command(scrim_lib::cli::CliCommand::Config) => {
            let (config, _) = load_configuration();
            match serde_yaml::to_string(&config) {
                Ok(yaml) => print!("{}", yaml),
                Err(e) => {
                    scrim_lib::scrim_error!("Failed to serialize configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }
        scrim_lib::cli::CliParseResult::Command(scrim_lib::cli::CliCommand::Links { dir }) => {
            let dir_arg = match dir {
                Some(ref d) => d.as_path(),
                None => {
                    scrim_lib::scrim_error!("directory argument required for links command");
                    std::process::exit(1);
                }
            };
            let scrim_exe = match env::current_exe() {
                Ok(exe) => exe,
                Err(e) => {
                    scrim_lib::scrim_error!("Failed to resolve current executable path: {}", e);
                    std::process::exit(1);
                }
            };
            let (config, _) = load_configuration();
            if let Err(e) = scrim_lib::linker::create_links(dir_arg, &scrim_exe, &config) {
                scrim_lib::scrim_error!("{}", e);
                std::process::exit(1);
            }
        }
        scrim_lib::cli::CliParseResult::Command(scrim_lib::cli::CliCommand::Run { tool, args }) => {
            let tool_name = match tool {
                Some(t) => t,
                None => {
                    scrim_lib::scrim_error!("tool name argument required for run command");
                    std::process::exit(1);
                }
            };
            execute_tool(&tool_name, &args);
        }
        scrim_lib::cli::CliParseResult::Help(text) => {
            print!("{}", text);
        }
        scrim_lib::cli::CliParseResult::Error(err) => {
            let trimmed_err = err.strip_prefix("error: ").unwrap_or(&err);
            scrim_lib::scrim_error!("{}", trimmed_err);
            std::process::exit(1);
        }
    }
}

fn proxy_command(program_name: &str, args: &[std::ffi::OsString]) -> ! {
    let tool_args = if args.len() > 1 { &args[1..] } else { &[] };
    execute_tool(program_name, tool_args);
}

fn execute_tool(program_name: &str, tool_args: &[std::ffi::OsString]) -> ! {
    let (config, home_dir) = load_configuration();
    let resolution = resolver::resolve_tool(program_name, Some(&config));

    match resolution {
        Ok(Some(res)) => {
            let target_path = match res {
                resolver::Resolution::LocalPath(p) => p,
                resolver::Resolution::Fetch { url, sha256, archive_path } => {
                    let tools_dir = home_dir.as_ref().map(|h| h.join(".cache/scrim/tools")).unwrap_or_else(|| PathBuf::from(".cache/scrim/tools"));
                    let cache_dir = tools_dir.join(&sha256);
                    match fetcher::fetch_tool(program_name, &url, &sha256, archive_path.as_deref(), &cache_dir) {
                        Ok(p) => p,
                        Err(e) => {
                            scrim_lib::scrim_error!("Failed to fetch tool '{}': {}", program_name, e);
                            std::process::exit(1);
                        }
                    }
                }
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
                        telemetry::report_usage(program_name, &target_path);
                        libc::_exit(0);
                    }
                    // Parent process continues to exec
                }
            }

            // Replace the current process with the target tool
            let err = cmd.exec();
            
            // If exec returns, it failed
            scrim_lib::scrim_error!("Failed to execute tool: {}", err);
            std::process::exit(1);
        }
        Ok(None) => {
            scrim_lib::scrim_error!("Tool '{}' not found in config", program_name);
            std::process::exit(1);
        }
        Err(e) => {
            scrim_lib::scrim_error!("{}", e);
            std::process::exit(1);
        }
    }
}

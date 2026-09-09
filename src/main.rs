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

fn handle_management_command(args: &[std::ffi::OsString]) {
    match args.get(1).and_then(|s| s.to_str()) {
        Some("help") => {
            println!("Scrim: The transparent tool proxy.");
            println!("Usage: scrim <command> [args]");
            println!("\nCommands:");
            println!("  help     Displays usage information and available commands.");
            println!("  version  Displays the version of Scrim.");
            println!("  config   Reports the resultant aggregated configuration after resolving all configuration layers.");
        }
        Some("version") => println!("scrim {}", scrim_lib::VERSION),
        Some("config") => {
            let cwd = env::current_dir().expect("Failed to get current directory");
            let home_dir = get_home_dir();
            let config = match config::load_config(&cwd, home_dir.as_deref()) {
                Ok(c) => c,
                Err(e) => {
                    scrim_lib::scrim_error!("Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            };
            match serde_yaml::to_string(&config) {
                Ok(yaml) => print!("{}", yaml),
                Err(e) => {
                    scrim_lib::scrim_error!("Failed to serialize configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => println!("Run 'scrim help' for usage."),
    }
}

fn proxy_command(program_name: &str, args: &[std::ffi::OsString]) {
    let cwd = env::current_dir().expect("Failed to get current directory");
    let home_dir = get_home_dir();
    let config = match config::load_config(&cwd, home_dir.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            scrim_lib::scrim_error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

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
            
            // Pass through all arguments (excluding the shim itself)
            if args.len() > 1 {
                cmd.args(&args[1..]);
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

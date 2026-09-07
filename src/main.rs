use scrim_lib::{config, fetcher, resolver, telemetry};

use std::env;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::path::Path;

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

fn handle_management_command(args: &[std::ffi::OsString]) {
    if args.len() < 2 {
        println!("Scrim: The transparent tool proxy.");
        println!("Usage: scrim <command> [args]");
        return;
    }

    match args[1].to_str() {
        Some("version") => println!("scrim 0.1.0"),
        Some(cmd) => println!("Unknown command: {}", cmd),
        None => println!("Unknown command"),
    }
}

fn proxy_command(program_name: &str, args: &[std::ffi::OsString]) {
    let cwd = env::current_dir().expect("Failed to get current directory");
    let config = match config::load_config(&cwd) {
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
                    match fetcher::fetch_tool(program_name, &url, &sha256, archive_path.as_deref()) {
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

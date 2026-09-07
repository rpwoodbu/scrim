use scrim_lib::{config, fetcher, resolver, telemetry};

use std::env;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.is_empty() {
        return;
    }

    let current_exe = env::current_exe().expect("Failed to get current executable path");
    let program_name = Path::new(&args[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("scrim");

    if program_name == "scrim" {
        handle_management_command(&args);
    } else {
        proxy_command(program_name, &args, &current_exe);
    }
}

fn handle_management_command(args: &[String]) {
    if args.len() < 2 {
        println!("Scrim: The transparent tool proxy.");
        println!("Usage: scrim <command> [args]");
        return;
    }

    match args[1].as_str() {
        "version" => println!("scrim 0.1.0"),
        _ => println!("Unknown command: {}", args[1]),
    }
}

fn proxy_command(program_name: &str, args: &[String], current_exe: &Path) {
    let cwd = env::current_dir().expect("Failed to get current directory");
    let config_path = config::find_config(&cwd);
    
    let config = config_path.and_then(|p| {
        config::read_config(&p).ok()
    });

    let path_env = env::var("PATH").unwrap_or_default();
    
    let resolution = resolver::resolve_tool(program_name, config.as_ref(), &path_env, current_exe);

    match resolution {
        Ok(Some(res)) => {
            let target_path = match res {
                resolver::Resolution::LocalPath(p) => p,
                resolver::Resolution::Fetch { url, sha256, archive_path } => {
                    match fetcher::fetch_tool(program_name, &url, &sha256, archive_path.as_deref()) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("Error: Failed to fetch tool '{}': {}", program_name, e);
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
            let enable_telemetry = config.as_ref().map(|c| c.telemetry).unwrap_or(true);
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
            eprintln!("Failed to execute tool: {}", err);
            std::process::exit(1);
        }
        Ok(None) => {
            eprintln!("Error: Tool '{}' not found in config or PATH", program_name);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

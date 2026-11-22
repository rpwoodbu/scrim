use cli::*;
use std::ffi::OsString;
use std::path::PathBuf;

#[test]
fn test_no_args_shows_help() {
    match parse_management_cli(["scrim"]) {
        CliParseResult::Help(text) => {
            assert!(text.contains("Scrim: The transparent tool proxy."));
            assert!(text.contains("Commands:"));
            assert!(text.contains("Environment Variables:"));
            assert!(text.contains("XDG_CACHE_HOME"));
            assert!(text.contains("XDG_CONFIG_HOME"));
            assert!(text.contains("XDG_STATE_HOME"));
        }
        other => panic!("Expected Help, got {:?}", other),
    }
}

#[test]
fn test_main_help_flag_shows_environment_variables() {
    match parse_management_cli(["scrim", "--help"]) {
        CliParseResult::Help(text) => {
            assert!(text.contains("Environment Variables:"));
            assert!(text.contains("XDG_CACHE_HOME"));
            assert!(text.contains("XDG_CONFIG_HOME"));
            assert!(text.contains("XDG_STATE_HOME"));
        }
        other => panic!("Expected Help, got {:?}", other),
    }
}

#[test]
fn test_unknown_command_errors() {
    match parse_management_cli(["scrim", "unknown"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unrecognized subcommand 'unknown'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
}

#[test]
fn test_help_subcommand() {
    match parse_management_cli(["scrim", "help"]) {
        CliParseResult::Help(text) => {
            assert!(text.contains("Scrim: The transparent tool proxy."));
        }
        other => panic!("Expected Help, got {:?}", other),
    }
    match parse_management_cli(["scrim", "help", "run"]) {
        CliParseResult::Help(text) => {
            assert!(text.contains("Runs a defined tool directly without requiring a link."));
        }
        other => panic!("Expected Help, got {:?}", other),
    }
    match parse_management_cli(["scrim", "help", "links"]) {
        CliParseResult::Help(text) => {
            assert!(text.to_lowercase().contains("exit code"));
        }
        other => panic!("Expected Help, got {:?}", other),
    }
}

#[test]
fn test_version_subcommand() {
    assert_eq!(
        parse_management_cli(["scrim", "version"]),
        CliParseResult::Command(CliCommand::Version)
    );
}

#[test]
fn test_config_subcommand() {
    assert_eq!(
        parse_management_cli(["scrim", "config"]),
        CliParseResult::Command(CliCommand::Config {
            tool: None,
            platform: None,
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "--allow-fallback"]),
        CliParseResult::Command(CliCommand::Config {
            tool: None,
            platform: None,
            allow_fallback: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "node"]),
        CliParseResult::Command(CliCommand::Config {
            tool: Some("node".to_string()),
            platform: None,
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "node", "--allow-fallback"]),
        CliParseResult::Command(CliCommand::Config {
            tool: Some("node".to_string()),
            platform: None,
            allow_fallback: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "--platform", "linux-x86_64"]),
        CliParseResult::Command(CliCommand::Config {
            tool: None,
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "-p", "linux-x86_64"]),
        CliParseResult::Command(CliCommand::Config {
            tool: None,
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "--platform", "linux-x86_64", "node"]),
        CliParseResult::Command(CliCommand::Config {
            tool: Some("node".to_string()),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "node", "--platform", "linux-x86_64"]),
        CliParseResult::Command(CliCommand::Config {
            tool: Some("node".to_string()),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "node", "-p", "linux-x86_64"]),
        CliParseResult::Command(CliCommand::Config {
            tool: Some("node".to_string()),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "config", "-p", "linux-x86_64", "node"]),
        CliParseResult::Command(CliCommand::Config {
            tool: Some("node".to_string()),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    match parse_management_cli(["scrim", "config", "node", "extra"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument 'extra'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
    match parse_management_cli(["scrim", "config", "--unknown"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument '--unknown'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
    match parse_management_cli(["scrim", "config", "--platform"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("a value is required for '--platform <PLATFORM>'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
}

#[test]
fn test_which_subcommand() {
    assert_eq!(
        parse_management_cli(["scrim", "which"]),
        CliParseResult::Error("tool name argument required for which command".to_string())
    );
    assert_eq!(
        parse_management_cli(["scrim", "which", "node"]),
        CliParseResult::Command(CliCommand::Which {
            tool: "node".to_string(),
            platform: None,
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "which", "--allow-fallback", "node"]),
        CliParseResult::Command(CliCommand::Which {
            tool: "node".to_string(),
            platform: None,
            allow_fallback: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "which", "--platform", "linux-x86_64", "node"]),
        CliParseResult::Command(CliCommand::Which {
            tool: "node".to_string(),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "which", "node", "--platform", "linux-x86_64"]),
        CliParseResult::Command(CliCommand::Which {
            tool: "node".to_string(),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "which", "-p", "linux-x86_64", "node"]),
        CliParseResult::Command(CliCommand::Which {
            tool: "node".to_string(),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "which", "node", "-p", "linux-x86_64"]),
        CliParseResult::Command(CliCommand::Which {
            tool: "node".to_string(),
            platform: Some("linux-x86_64".to_string()),
            allow_fallback: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "which", "--platform", "linux-x86_64"]),
        CliParseResult::Error("tool name argument required for which command".to_string())
    );
    match parse_management_cli(["scrim", "which", "node", "extra"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument 'extra'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
    match parse_management_cli(["scrim", "which", "--unknown", "node"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument '--unknown'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
    match parse_management_cli(["scrim", "which", "node", "--platform"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("a value is required for '--platform <PLATFORM>'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
}

#[test]
fn test_links_subcommand() {
    assert_eq!(
        parse_management_cli(["scrim", "links", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: false,
            prune: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "-n", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: true,
            prune: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "/tmp/bin", "-n"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: true,
            prune: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "--dry-run", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: true,
            prune: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "/tmp/bin", "--dry-run"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: true,
            prune: false,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "-p", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: false,
            prune: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "--prune", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: false,
            prune: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "/tmp/bin", "-p"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: false,
            prune: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "-n", "-p", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: true,
            prune: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "-np", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: true,
            prune: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "--dry-run", "--prune", "/tmp/bin"]),
        CliParseResult::Command(CliCommand::Links {
            dir: PathBuf::from("/tmp/bin"),
            dry_run: true,
            prune: true,
        })
    );
    assert_eq!(
        parse_management_cli(["scrim", "links"]),
        CliParseResult::Error("directory argument required for links command".to_string())
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "-n"]),
        CliParseResult::Error("directory argument required for links command".to_string())
    );
    assert_eq!(
        parse_management_cli(["scrim", "links", "-p"]),
        CliParseResult::Error("directory argument required for links command".to_string())
    );
}

#[test]
fn test_run_subcommand_positional() {
    assert_eq!(
        parse_management_cli(["scrim", "run", "node", "index.js", "foo"]),
        CliParseResult::Command(CliCommand::Run {
            tool: "node".to_string(),
            args: vec![OsString::from("index.js"), OsString::from("foo")],
            allow_fallback: false,
        })
    );
}

#[test]
fn test_run_subcommand_allow_fallback() {
    assert_eq!(
        parse_management_cli(["scrim", "run", "--allow-fallback", "node", "index.js"]),
        CliParseResult::Command(CliCommand::Run {
            tool: "node".to_string(),
            args: vec![OsString::from("index.js")],
            allow_fallback: true,
        })
    );
}

#[test]
fn test_run_subcommand_no_tool() {
    assert_eq!(
        parse_management_cli(["scrim", "run"]),
        CliParseResult::Error("tool name argument required for run command".to_string())
    );
}

#[test]
fn test_run_subcommand_with_double_dash_before_tool() {
    assert_eq!(
        parse_management_cli(["scrim", "run", "--", "node", "-v", "--flag"]),
        CliParseResult::Command(CliCommand::Run {
            tool: "node".to_string(),
            args: vec![OsString::from("-v"), OsString::from("--flag")],
            allow_fallback: false,
        })
    );
}

#[test]
fn test_run_subcommand_with_double_dash_after_tool() {
    assert_eq!(
        parse_management_cli(["scrim", "run", "node", "--", "-v", "--flag"]),
        CliParseResult::Command(CliCommand::Run {
            tool: "node".to_string(),
            args: vec![OsString::from("-v"), OsString::from("--flag")],
            allow_fallback: false,
        })
    );
}

#[test]
fn test_run_subcommand_with_double_dash_empty() {
    assert_eq!(
        parse_management_cli(["scrim", "run", "--"]),
        CliParseResult::Error("tool name argument required for run command".to_string())
    );
}

#[test]
fn test_undefined_flags_fail() {
    match parse_management_cli(["scrim", "--foo"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument '--foo'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }

    match parse_management_cli(["scrim", "-x"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument '-x'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }

    match parse_management_cli(["scrim", "run", "node", "--foo"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument '--foo'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }

    match parse_management_cli(["scrim", "config", "--foo"]) {
        CliParseResult::Error(err) => {
            assert!(!err.starts_with("error: "));
            assert!(err.contains("unexpected argument '--foo'"));
        }
        other => panic!("Expected Error, got {:?}", other),
    }
}

#[test]
fn test_help_and_version_flags() {
    match parse_management_cli(["scrim", "--help"]) {
        CliParseResult::Help(text) => {
            assert!(text.contains("Scrim: The transparent tool proxy."));
            assert!(text.contains("Commands:"));
            assert!(text.contains("run"));
        }
        other => panic!("Expected Help, got {:?}", other),
    }

    match parse_management_cli(["scrim", "-h"]) {
        CliParseResult::Help(text) => {
            assert!(text.contains("Scrim: The transparent tool proxy."));
        }
        other => panic!("Expected Help, got {:?}", other),
    }

    assert_eq!(
        parse_management_cli(["scrim", "-V"]),
        CliParseResult::Command(CliCommand::Version)
    );
    assert_eq!(
        parse_management_cli(["scrim", "--version"]),
        CliParseResult::Command(CliCommand::Version)
    );
}

#[test]
fn test_run_subcommand_with_h_flag() {
    assert_eq!(
        parse_management_cli(["scrim", "run", "foo", "--", "-h"]),
        CliParseResult::Command(CliCommand::Run {
            tool: "foo".to_string(),
            args: vec![OsString::from("-h")],
            allow_fallback: false,
        })
    );
}

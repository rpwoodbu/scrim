use clap::{error::ErrorKind, Arg, Command};
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum CliCommand {
    Version,
    Config,
    Links {
        dir: Option<PathBuf>,
    },
    Run {
        tool: Option<String>,
        args: Vec<OsString>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliParseResult {
    Command(CliCommand),
    Help(String),
    Error(String),
}

pub fn build_cli() -> Command {
    Command::new("scrim")
        .about("Scrim: The transparent tool proxy.")
        .version(crate::VERSION)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("version").about("Displays the version of Scrim."))
        .subcommand(
            Command::new("config").about(
                "Reports the resultant aggregated configuration after resolving all configuration layers.",
            ),
        )
        .subcommand(
            Command::new("links")
                .about("Updates links in a specified target directory for all defined tools.")
                .arg(Arg::new("dir").value_name("DIR")),
        )
        .subcommand(
            Command::new("run")
                .about("Runs a defined tool directly without requiring a link.")
                .arg(Arg::new("tool").value_name("TOOL"))
                .arg(
                    Arg::new("args")
                        .value_name("ARGS")
                        .value_parser(clap::builder::OsStringValueParser::new())
                        .num_args(0..),
                ),
        )
}

pub fn parse_management_cli<I, T>(itr: I) -> CliParseResult
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cmd = build_cli();
    match cmd.try_get_matches_from(itr) {
        Ok(matches) => match matches.subcommand() {
            Some(("version", _)) => CliParseResult::Command(CliCommand::Version),
            Some(("config", _)) => CliParseResult::Command(CliCommand::Config),
            Some(("links", sub_matches)) => {
                let dir = sub_matches.get_one::<String>("dir").map(PathBuf::from);
                CliParseResult::Command(CliCommand::Links { dir })
            }
            Some(("run", sub_matches)) => {
                let tool = sub_matches.get_one::<String>("tool").cloned();
                let args = sub_matches
                    .get_many::<OsString>("args")
                    .unwrap_or_default()
                    .cloned()
                    .collect();
                CliParseResult::Command(CliCommand::Run { tool, args })
            }
            _ => unreachable!(),
        },
        Err(err) => match err.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                CliParseResult::Help(err.to_string())
            }
            ErrorKind::DisplayVersion => CliParseResult::Command(CliCommand::Version),
            _ => CliParseResult::Error(err.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_args_shows_help() {
        match parse_management_cli(["scrim"]) {
            CliParseResult::Help(text) => {
                assert!(text.contains("Scrim: The transparent tool proxy."));
                assert!(text.contains("Commands:"));
            }
            other => panic!("Expected Help, got {:?}", other),
        }
    }

    #[test]
    fn test_unknown_command_errors() {
        match parse_management_cli(["scrim", "unknown"]) {
            CliParseResult::Error(err) => {
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
            CliParseResult::Command(CliCommand::Config)
        );
    }

    #[test]
    fn test_links_subcommand() {
        assert_eq!(
            parse_management_cli(["scrim", "links", "/tmp/bin"]),
            CliParseResult::Command(CliCommand::Links {
                dir: Some(PathBuf::from("/tmp/bin"))
            })
        );
        assert_eq!(
            parse_management_cli(["scrim", "links"]),
            CliParseResult::Command(CliCommand::Links { dir: None })
        );
    }

    #[test]
    fn test_run_subcommand_positional() {
        assert_eq!(
            parse_management_cli(["scrim", "run", "node", "index.js", "foo"]),
            CliParseResult::Command(CliCommand::Run {
                tool: Some("node".to_string()),
                args: vec![OsString::from("index.js"), OsString::from("foo")],
            })
        );
    }

    #[test]
    fn test_run_subcommand_no_tool() {
        assert_eq!(
            parse_management_cli(["scrim", "run"]),
            CliParseResult::Command(CliCommand::Run {
                tool: None,
                args: vec![],
            })
        );
    }

    #[test]
    fn test_run_subcommand_with_double_dash_before_tool() {
        assert_eq!(
            parse_management_cli(["scrim", "run", "--", "node", "-v", "--flag"]),
            CliParseResult::Command(CliCommand::Run {
                tool: Some("node".to_string()),
                args: vec![OsString::from("-v"), OsString::from("--flag")],
            })
        );
    }

    #[test]
    fn test_run_subcommand_with_double_dash_after_tool() {
        assert_eq!(
            parse_management_cli(["scrim", "run", "node", "--", "-v", "--flag"]),
            CliParseResult::Command(CliCommand::Run {
                tool: Some("node".to_string()),
                args: vec![OsString::from("-v"), OsString::from("--flag")],
            })
        );
    }

    #[test]
    fn test_run_subcommand_with_double_dash_empty() {
        assert_eq!(
            parse_management_cli(["scrim", "run", "--"]),
            CliParseResult::Command(CliCommand::Run {
                tool: None,
                args: vec![],
            })
        );
    }

    #[test]
    fn test_undefined_flags_fail() {
        match parse_management_cli(["scrim", "--foo"]) {
            CliParseResult::Error(err) => assert!(err.contains("unexpected argument '--foo'")),
            other => panic!("Expected Error, got {:?}", other),
        }

        match parse_management_cli(["scrim", "-x"]) {
            CliParseResult::Error(err) => assert!(err.contains("unexpected argument '-x'")),
            other => panic!("Expected Error, got {:?}", other),
        }

        match parse_management_cli(["scrim", "run", "node", "--foo"]) {
            CliParseResult::Error(err) => assert!(err.contains("unexpected argument '--foo'")),
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
                tool: Some("foo".to_string()),
                args: vec![OsString::from("-h")],
            })
        );
    }
}

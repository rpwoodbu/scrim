use clap::{error::ErrorKind, Arg, Command};
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum CliCommand {
    Version,
    Config {
        tool: Option<String>,
        platform: Option<String>,
        allow_fallback: bool,
    },
    Links {
        dir: PathBuf,
        dry_run: bool,
        prune: bool,
    },
    Run {
        tool: String,
        args: Vec<OsString>,
        allow_fallback: bool,
    },
    Which {
        tool: String,
        platform: Option<String>,
        allow_fallback: bool,
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
        .version(version::VERSION)
        .after_help(
            "Environment Variables:\n  \
             XDG_CACHE_HOME   Base directory for user-specific cache files (default: $HOME/.cache)\n  \
             XDG_CONFIG_HOME  Base directory for user-specific configuration files (default: $HOME/.config)",
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("version").about("Displays the version of Scrim."))
        .subcommand(
            Command::new("config")
                .about(
                    "Reports the resultant aggregated configuration after resolving all configuration layers.",
                )
                .arg(
                    Arg::new("platform")
                        .long("platform")
                        .short('p')
                        .value_name("PLATFORM")
                        .help("Target platform to evaluate."),
                )
                .arg(
                    Arg::new("allow_fallback")
                        .long("allow-fallback")
                        .action(clap::ArgAction::SetTrue)
                        .help("Fall back to lower-precedence configuration layers if tool configuration is invalid."),
                )
                .arg(Arg::new("tool").value_name("TOOL")),
        )
        .subcommand(
            Command::new("links")
                .about("Updates links in a specified target directory for all defined tools.")
                .after_help(
                    "Exit codes in dry-run mode:\n  \
                     0  All links are already correct (no changes needed)\n  \
                     2  Changes would have been made\n  \
                     1  An error occurred",
                )
                .arg(
                    Arg::new("dry_run")
                        .long("dry-run")
                        .short('n')
                        .action(clap::ArgAction::SetTrue)
                        .help("Perform a dry run without modifying any links."),
                )
                .arg(
                    Arg::new("prune")
                        .long("prune")
                        .short('p')
                        .action(clap::ArgAction::SetTrue)
                        .help("Remove obsolete symbolic links pointing to the scrim binary."),
                )
                .arg(Arg::new("dir").value_name("DIR")),
        )
        .subcommand(
            Command::new("run")
                .about("Runs a defined tool directly without requiring a link.")
                .arg(
                    Arg::new("allow_fallback")
                        .long("allow-fallback")
                        .action(clap::ArgAction::SetTrue)
                        .help("Fall back to lower-precedence configuration layers or PATH if tool configuration is invalid."),
                )
                .arg(Arg::new("tool").value_name("TOOL"))
                .arg(
                    Arg::new("args")
                        .value_name("ARGS")
                        .value_parser(clap::builder::OsStringValueParser::new())
                        .num_args(0..),
                ),
        )
        .subcommand(
            Command::new("which")
                .about("Locates and prints the absolute path of the binary that would be executed for a tool without running it.")
                .arg(
                    Arg::new("platform")
                        .long("platform")
                        .short('p')
                        .value_name("PLATFORM")
                        .help("Target platform to evaluate."),
                )
                .arg(
                    Arg::new("allow_fallback")
                        .long("allow-fallback")
                        .action(clap::ArgAction::SetTrue)
                        .help("Fall back to lower-precedence configuration layers or PATH if tool configuration is invalid."),
                )
                .arg(Arg::new("tool").value_name("TOOL")),
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
            Some(("config", sub_matches)) => {
                let tool = sub_matches.get_one::<String>("tool").cloned();
                let platform = sub_matches.get_one::<String>("platform").cloned();
                let allow_fallback = sub_matches.get_flag("allow_fallback");
                CliParseResult::Command(CliCommand::Config {
                    tool,
                    platform,
                    allow_fallback,
                })
            }
            Some(("links", sub_matches)) => {
                let Some(dir) = sub_matches.get_one::<String>("dir").map(PathBuf::from) else {
                    return CliParseResult::Error(
                        "directory argument required for links command".to_string(),
                    );
                };
                let dry_run = sub_matches.get_flag("dry_run");
                let prune = sub_matches.get_flag("prune");
                CliParseResult::Command(CliCommand::Links {
                    dir,
                    dry_run,
                    prune,
                })
            }
            Some(("run", sub_matches)) => {
                let Some(tool) = sub_matches.get_one::<String>("tool").cloned() else {
                    return CliParseResult::Error(
                        "tool name argument required for run command".to_string(),
                    );
                };
                let allow_fallback = sub_matches.get_flag("allow_fallback");
                let args = sub_matches
                    .get_many::<OsString>("args")
                    .unwrap_or_default()
                    .cloned()
                    .collect();
                CliParseResult::Command(CliCommand::Run {
                    tool,
                    args,
                    allow_fallback,
                })
            }
            Some(("which", sub_matches)) => {
                let Some(tool) = sub_matches.get_one::<String>("tool").cloned() else {
                    return CliParseResult::Error(
                        "tool name argument required for which command".to_string(),
                    );
                };
                let platform = sub_matches.get_one::<String>("platform").cloned();
                let allow_fallback = sub_matches.get_flag("allow_fallback");
                CliParseResult::Command(CliCommand::Which {
                    tool,
                    platform,
                    allow_fallback,
                })
            }
            _ => unreachable!(),
        },
        Err(err) => match err.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                CliParseResult::Help(err.to_string())
            }
            ErrorKind::DisplayVersion => CliParseResult::Command(CliCommand::Version),
            _ => {
                let err_str = err.to_string();
                let trimmed = err_str
                    .strip_prefix("error: ")
                    .unwrap_or(&err_str)
                    .to_string();
                CliParseResult::Error(trimmed)
            }
        },
    }
}


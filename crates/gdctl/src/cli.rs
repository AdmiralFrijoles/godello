//! The command line surface.
//!
//! This defines the gdctl commands and their arguments with clap. Parsing only.
//! Each command is carried out in the commands module. Version requirements and
//! variants are parsed here so a bad value is rejected early with a clear message.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use godello_core::{Variant, VersionPattern};

/// Godello engine and project launcher.
#[derive(Debug, Parser)]
#[command(
    name = "gdctl",
    version,
    about = "Godello: a Godot engine and project launcher"
)]
pub struct Cli {
    /// Do not prompt. Take the safe default for each question so a command can
    /// run in a script or a CI job.
    #[arg(
        short = 'y',
        long = "yes",
        visible_alias = "non-interactive",
        global = true
    )]
    pub yes: bool,

    /// Suppress normal output. Errors are still shown on stderr. This also turns
    /// off prompts and takes the safe default for each one.
    #[arg(short = 's', long = "silent", global = true)]
    pub silent: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// A top level gdctl command.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install an engine version.
    Install {
        /// The version to install, for example 4.3 or 4.4-rc1.
        #[arg(value_parser = parse_pattern)]
        version: VersionPattern,
        #[command(flatten)]
        variant: VariantArg,
    },
    /// Remove an installed engine version.
    Remove {
        /// The version to remove, matched against what is installed.
        #[arg(value_parser = parse_pattern)]
        version: VersionPattern,
        #[command(flatten)]
        variant: VariantArg,
    },
    /// List installed engines, or available versions with --remote.
    List {
        /// List versions available to install instead of installed ones.
        #[arg(long)]
        remote: bool,
        /// Include rc, beta, and dev releases. Only affects --remote.
        #[arg(long)]
        pre: bool,
    },
    /// Search available versions by text.
    Search {
        /// The text to look for in a version tag.
        text: String,
    },
    /// Open the editor for a version with no project. Shows the project manager.
    Open {
        /// The version to open, matched against what is installed.
        #[arg(value_parser = parse_pattern)]
        version: VersionPattern,
        #[command(flatten)]
        variant: VariantArg,
        #[command(flatten)]
        detach: DetachArg,
    },
    /// Manage the projects you have added.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Clone a repository and add it as a project.
    Clone {
        /// The repository url to clone.
        url: String,
        /// The folder to clone into. Defaults to a name taken from the url.
        dir: Option<PathBuf>,
    },
    /// Run the project in the current folder without the editor.
    Run {
        /// Skip building the C# solution even when that setting is on.
        #[arg(long = "no-build")]
        no_build: bool,
        #[command(flatten)]
        detach: DetachArg,
    },
    /// Open the editor for the project in the current folder.
    Edit {
        /// Skip building the C# solution even when that setting is on.
        #[arg(long = "no-build")]
        no_build: bool,
        #[command(flatten)]
        detach: DetachArg,
    },
    /// Read or change a setting.
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
}

/// A project subcommand.
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Add a project and read its version pin.
    Add { path: PathBuf },
    /// List added projects.
    List,
    /// Write the required version pin into a project.
    Pin {
        path: PathBuf,
        #[arg(value_parser = parse_pattern)]
        version: VersionPattern,
    },
    /// Open the editor for a project.
    Edit {
        path: PathBuf,
        /// Skip building the C# solution even when that setting is on.
        #[arg(long = "no-build")]
        no_build: bool,
        #[command(flatten)]
        detach: DetachArg,
    },
    /// Run a project without the editor.
    Run {
        path: PathBuf,
        /// Skip building the C# solution even when that setting is on.
        #[arg(long = "no-build")]
        no_build: bool,
        #[command(flatten)]
        detach: DetachArg,
    },
    /// Forget a project.
    Remove { path: PathBuf },
    /// Show the branch, sync state, and local changes.
    Status { path: PathBuf },
    /// Bring a project up to date with its tracked remote.
    Update {
        path: PathBuf,
        /// Hard reset to the remote. This loses local changes and local commits.
        #[arg(long)]
        reset: bool,
    },
}

/// A settings subcommand.
#[derive(Debug, Subcommand)]
pub enum SettingsCommand {
    /// List every setting and its current value.
    List,
    /// Read a setting by name.
    Get { key: String },
    /// Change a setting by name.
    Set { key: String, value: String },
}

/// How a command names the build flavor. Either spelled out with --variant or
/// the -m shorthand for mono. The two cannot be given together.
#[derive(Debug, Args)]
pub struct VariantArg {
    /// The build flavor, standard or mono. Defaults to the configured one.
    #[arg(long, value_parser = parse_variant)]
    variant: Option<Variant>,
    /// Shorthand for --variant mono.
    #[arg(short = 'm', long = "mono", conflicts_with = "variant")]
    mono: bool,
}

impl VariantArg {
    /// The variant the user named, or None to fall back to the configured one.
    pub fn selected(&self) -> Option<Variant> {
        if self.mono {
            Some(Variant::Mono)
        } else {
            self.variant
        }
    }
}

/// A per launch override of the detached setting. At most one of the two may be
/// given. Neither means use the configured default.
#[derive(Debug, Args)]
pub struct DetachArg {
    /// Launch detached so the command returns right away.
    #[arg(short = 'd', long = "detached")]
    detached: bool,
    /// Stay attached and wait for the editor to close.
    #[arg(short = 'a', long = "attached", conflicts_with = "detached")]
    attached: bool,
}

impl DetachArg {
    /// The chosen detached value, or None to use the configured default.
    pub fn selected(&self) -> Option<bool> {
        if self.detached {
            Some(true)
        } else if self.attached {
            Some(false)
        } else {
            None
        }
    }
}

/// Parse a version requirement, turning the error into a message clap can show.
fn parse_pattern(text: &str) -> Result<VersionPattern, String> {
    text.parse()
        .map_err(|err: godello_core::VersionParseError| err.to_string())
}

/// Parse a build flavor, turning the error into a message clap can show.
fn parse_variant(text: &str) -> Result<Variant, String> {
    text.parse()
        .map_err(|err: godello_core::VersionParseError| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn install_takes_a_version_and_default_variant() {
        let cli = parse(&["gdctl", "install", "4.3"]);
        match cli.command.unwrap() {
            Command::Install { version, variant } => {
                assert_eq!(version, "4.3".parse().unwrap());
                assert_eq!(variant.selected(), None);
            }
            other => panic!("expected install, got {other:?}"),
        }
    }

    #[test]
    fn install_accepts_an_explicit_variant() {
        let cli = parse(&["gdctl", "install", "4.4-rc1", "--variant", "mono"]);
        match cli.command.unwrap() {
            Command::Install { version, variant } => {
                assert_eq!(version, "4.4-rc1".parse().unwrap());
                assert_eq!(variant.selected(), Some(Variant::Mono));
            }
            other => panic!("expected install, got {other:?}"),
        }
    }

    #[test]
    fn variant_accepts_the_csharp_alias() {
        let cli = parse(&["gdctl", "remove", "4.3", "--variant", "csharp"]);
        match cli.command.unwrap() {
            Command::Remove { variant, .. } => assert_eq!(variant.selected(), Some(Variant::Mono)),
            other => panic!("expected remove, got {other:?}"),
        }
    }

    #[test]
    fn the_m_shorthand_means_mono() {
        for command in ["install", "remove", "open"] {
            let cli = parse(&["gdctl", command, "4.3", "-m"]);
            let selected = match cli.command.unwrap() {
                Command::Install { variant, .. } => variant.selected(),
                Command::Remove { variant, .. } => variant.selected(),
                Command::Open { variant, .. } => variant.selected(),
                other => panic!("expected an engine command, got {other:?}"),
            };
            assert_eq!(selected, Some(Variant::Mono), "for {command}");
        }
        // The long form spells the same thing.
        let cli = parse(&["gdctl", "install", "4.3", "--mono"]);
        match cli.command.unwrap() {
            Command::Install { variant, .. } => assert_eq!(variant.selected(), Some(Variant::Mono)),
            other => panic!("expected install, got {other:?}"),
        }
    }

    #[test]
    fn m_and_variant_together_are_rejected() {
        let result =
            Cli::try_parse_from(["gdctl", "install", "4.3", "-m", "--variant", "standard"]);
        assert!(result.is_err());
    }

    #[test]
    fn list_flags_parse() {
        let cli = parse(&["gdctl", "list", "--remote", "--pre"]);
        match cli.command.unwrap() {
            Command::List { remote, pre } => {
                assert!(remote);
                assert!(pre);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn list_defaults_to_local() {
        let cli = parse(&["gdctl", "list"]);
        match cli.command.unwrap() {
            Command::List { remote, pre } => {
                assert!(!remote);
                assert!(!pre);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn the_yes_flag_and_its_alias_work_anywhere() {
        assert!(parse(&["gdctl", "-y", "list"]).yes);
        assert!(parse(&["gdctl", "list", "--yes"]).yes);
        assert!(parse(&["gdctl", "--non-interactive", "list"]).yes);
        assert!(!parse(&["gdctl", "list"]).yes);
    }

    #[test]
    fn no_subcommand_is_allowed() {
        let cli = parse(&["gdctl"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn project_pin_takes_a_path_and_version() {
        let cli = parse(&["gdctl", "project", "pin", "/games/one", "4.3-stable"]);
        match cli.command.unwrap() {
            Command::Project {
                command: ProjectCommand::Pin { path, version },
            } => {
                assert_eq!(path, PathBuf::from("/games/one"));
                assert_eq!(version, "4.3-stable".parse().unwrap());
            }
            other => panic!("expected project pin, got {other:?}"),
        }
    }

    #[test]
    fn project_update_reset_flag_parses() {
        let cli = parse(&["gdctl", "project", "update", "/games/one", "--reset"]);
        match cli.command.unwrap() {
            Command::Project {
                command: ProjectCommand::Update { reset, .. },
            } => assert!(reset),
            other => panic!("expected project update, got {other:?}"),
        }
        let cli = parse(&["gdctl", "project", "update", "/games/one"]);
        match cli.command.unwrap() {
            Command::Project {
                command: ProjectCommand::Update { reset, .. },
            } => assert!(!reset),
            other => panic!("expected project update, got {other:?}"),
        }
    }

    #[test]
    fn clone_dir_is_optional() {
        let cli = parse(&["gdctl", "clone", "https://example.test/game.git"]);
        match cli.command.unwrap() {
            Command::Clone { url, dir } => {
                assert_eq!(url, "https://example.test/game.git");
                assert_eq!(dir, None);
            }
            other => panic!("expected clone, got {other:?}"),
        }
        let cli = parse(&["gdctl", "clone", "https://example.test/game.git", "mygame"]);
        match cli.command.unwrap() {
            Command::Clone { dir, .. } => assert_eq!(dir, Some(PathBuf::from("mygame"))),
            other => panic!("expected clone, got {other:?}"),
        }
    }

    #[test]
    fn settings_list_parses() {
        let cli = parse(&["gdctl", "settings", "list"]);
        assert!(matches!(
            cli.command,
            Some(Command::Settings {
                command: SettingsCommand::List
            })
        ));
    }

    #[test]
    fn settings_set_takes_a_key_and_value() {
        let cli = parse(&["gdctl", "settings", "set", "default_variant", "mono"]);
        match cli.command.unwrap() {
            Command::Settings {
                command: SettingsCommand::Set { key, value },
            } => {
                assert_eq!(key, "default_variant");
                assert_eq!(value, "mono");
            }
            other => panic!("expected settings set, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_version_is_rejected() {
        let result = Cli::try_parse_from(["gdctl", "install", "not-a-version"]);
        assert!(result.is_err());
    }

    #[test]
    fn a_bad_variant_is_rejected() {
        let result = Cli::try_parse_from(["gdctl", "install", "4.3", "--variant", "msbuild"]);
        assert!(result.is_err());
    }

    #[test]
    fn run_and_edit_take_no_path() {
        assert!(matches!(
            parse(&["gdctl", "run"]).command,
            Some(Command::Run {
                no_build: false,
                ..
            })
        ));
        assert!(matches!(
            parse(&["gdctl", "edit"]).command,
            Some(Command::Edit {
                no_build: false,
                ..
            })
        ));
    }

    #[test]
    fn run_and_edit_take_no_build() {
        assert!(matches!(
            parse(&["gdctl", "run", "--no-build"]).command,
            Some(Command::Run { no_build: true, .. })
        ));
        assert!(matches!(
            parse(&["gdctl", "edit", "--no-build"]).command,
            Some(Command::Edit { no_build: true, .. })
        ));
    }

    #[test]
    fn project_edit_and_run_take_no_build() {
        let cli = parse(&["gdctl", "project", "edit", "/games/one", "--no-build"]);
        match cli.command.unwrap() {
            Command::Project {
                command: ProjectCommand::Edit { path, no_build, .. },
            } => {
                assert_eq!(path, PathBuf::from("/games/one"));
                assert!(no_build);
            }
            other => panic!("expected project edit, got {other:?}"),
        }
        let cli = parse(&["gdctl", "project", "run", "/games/one"]);
        match cli.command.unwrap() {
            Command::Project {
                command: ProjectCommand::Run { no_build, .. },
            } => assert!(!no_build),
            other => panic!("expected project run, got {other:?}"),
        }
    }

    #[test]
    fn run_and_edit_take_a_detached_override() {
        // The flags resolve to an explicit choice, and neither means default.
        let detached = match parse(&["gdctl", "run", "--detached"]).command.unwrap() {
            Command::Run { detach, .. } => detach.selected(),
            other => panic!("expected run, got {other:?}"),
        };
        assert_eq!(detached, Some(true));

        let attached = match parse(&["gdctl", "edit", "--attached"]).command.unwrap() {
            Command::Edit { detach, .. } => detach.selected(),
            other => panic!("expected edit, got {other:?}"),
        };
        assert_eq!(attached, Some(false));

        let neither = match parse(&["gdctl", "run"]).command.unwrap() {
            Command::Run { detach, .. } => detach.selected(),
            other => panic!("expected run, got {other:?}"),
        };
        assert_eq!(neither, None);
    }

    #[test]
    fn detached_and_attached_together_are_rejected() {
        let result = Cli::try_parse_from(["gdctl", "run", "--detached", "--attached"]);
        assert!(result.is_err());
    }

    #[test]
    fn project_run_takes_a_detached_override() {
        let cli = parse(&["gdctl", "project", "run", "/games/one", "--detached"]);
        match cli.command.unwrap() {
            Command::Project {
                command: ProjectCommand::Run { detach, .. },
            } => assert_eq!(detach.selected(), Some(true)),
            other => panic!("expected project run, got {other:?}"),
        }
    }

    #[test]
    fn open_takes_a_detached_override_alongside_the_variant() {
        let cli = parse(&["gdctl", "open", "4.3", "-m", "--attached"]);
        match cli.command.unwrap() {
            Command::Open {
                variant, detach, ..
            } => {
                assert_eq!(variant.selected(), Some(Variant::Mono));
                assert_eq!(detach.selected(), Some(false));
            }
            other => panic!("expected open, got {other:?}"),
        }
        // With neither override, open falls back to the configured default.
        let cli = parse(&["gdctl", "open", "4.3"]);
        match cli.command.unwrap() {
            Command::Open { detach, .. } => assert_eq!(detach.selected(), None),
            other => panic!("expected open, got {other:?}"),
        }
    }
}

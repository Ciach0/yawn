use crate::commands;
use crate::env::{Caps, ColorChoice, color_enabled, resolve_output};
use crate::error::CliError;
use crate::event::Level;
use crate::session::{self, Presentation};
use clap::{Args, Parser, Subcommand};
use std::ffi::OsString;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "yawn",
    version,
    about = "Yawn command line tool",
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args, Debug)]
pub struct Global {
    /// When to use colour
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto, value_name = "WHEN")]
    pub color: ColorChoice,
    /// Never start the terminal UI; print plain text lines
    #[arg(long, global = true)]
    pub plain: bool,
    /// Print one JSON object per line (implies no terminal UI)
    #[arg(long, global = true, conflicts_with = "plain")]
    pub json: bool,
    /// Minimum level to show (trace, debug, info, warn, error)
    #[arg(long, global = true, value_parser = clap::value_parser!(Level), default_value = "info", value_name = "LEVEL")]
    pub level: Level,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Develop the project in the current directory: build, serve, rebuild on change
    Dev(DevArgs),
    /// Compile the project's `.rsx` files to Rust and report errors
    Rsx,
    /// Build the project for production into `dist/`
    Build(BuildArgs),
    /// Create a new project in a new directory
    #[command(visible_alias = "init")]
    New(NewArgs),
    /// Remove what builds leave in the project: `target/` and `.yawn/` (not `dist/`)
    Clean,
    /// Install the latest release of the `yawn` tool
    Upgrade(UpgradeArgs),
    /// Show the environment Yawn sees
    Info,
}

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Directory to create; its name is the project name
    pub path: std::path::PathBuf,
    /// Depend on a local checkout of `crates/yawn-web`
    #[arg(long, value_name = "DIR")]
    pub yawn_path: Option<std::path::PathBuf>,
    /// Enable Tailwind CSS (`[tailwind] enabled = true` in Yawn.toml)
    #[arg(long)]
    pub tailwind: bool,
}

#[derive(Args, Debug)]
pub struct UpgradeArgs {
    /// Only say whether a newer release exists
    #[arg(long)]
    pub check: bool,
}

#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Directory to write (default `dist`, inside the project)
    #[arg(long, short, value_name = "DIR")]
    pub out: Option<std::path::PathBuf>,
    /// Render the pages to `.html` for a static host, instead of a server: the pages in
    /// `src/routes/` without parameters, plus `[build] prerender` in Yawn.toml
    #[arg(long = "static")]
    pub static_site: bool,
}

#[derive(Args, Debug)]
pub struct DevArgs {
    /// Port to serve on
    #[arg(long, short, value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
    pub port: Option<u16>,
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let cli = Cli::parse_from(args);
    match execute(cli, &Caps::detect()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            e.exit_code()
        }
    }
}

pub fn execute(cli: Cli, caps: &Caps) -> Result<ExitCode, CliError> {
    let color = color_enabled(cli.global.color, caps);
    let output = resolve_output(cli.global.plain, cli.global.json, caps);
    match cli.command {
        None => unreachable!("clap shows help when no subcommand is given"),
        Some(Command::Info) => commands::info(caps, color, cli.global.json)?,
        Some(Command::New(args)) => {
            let dir = crate::new::create(&args.path, args.yawn_path.as_deref(), args.tailwind)?;
            println!("created {}\n\n  cd {}\n  yawn dev", dir.display(), dir.display());
        }
        Some(Command::Upgrade(args)) => crate::upgrade::run(args.check)?,
        Some(Command::Clean) => commands::clean()?,
        Some(Command::Rsx) => return commands::rsx(cli.global.json),
        Some(Command::Build(args)) => return crate::build::run(&args),
        Some(Command::Dev(args)) => {
            let session = commands::dev_session(&args, cli.global.level)?;
            let p = Presentation { output, color, caps: caps.clone() };
            session::run(session, &p)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("yawn").chain(args.iter().copied()))
    }

    #[test]
    fn global_defaults() {
        let c = parse(&["info"]).unwrap();
        assert_eq!(c.global.level, Level::Info);
        assert_eq!(c.global.color, ColorChoice::Auto);
    }

    #[test]
    fn global_flags_work_before_and_after_the_subcommand() {
        assert!(parse(&["--plain", "info"]).unwrap().global.plain);
        assert!(parse(&["info", "--plain"]).unwrap().global.plain);
        assert_eq!(parse(&["info", "--level", "warn"]).unwrap().global.level, Level::Warn);
        assert_eq!(parse(&["--color", "never", "info"]).unwrap().global.color, ColorChoice::Never);
    }

    #[test]
    fn init_is_another_name_for_new() {
        let Some(Command::New(n)) = parse(&["init", "my-app", "--tailwind"]).unwrap().command
        else {
            panic!()
        };
        assert!(n.tailwind && n.path.ends_with("my-app"));
        assert!(parse(&["upgrade", "--check"]).unwrap().command.is_some());
        assert!(parse(&["clean"]).unwrap().command.is_some());
    }

    #[test]
    fn dev_port_flag() {
        let c = parse(&["dev", "--port", "4000"]).unwrap();
        let Some(Command::Dev(d)) = c.command else { panic!() };
        assert_eq!(d.port, Some(4000));
        let Some(Command::Dev(d)) = parse(&["dev"]).unwrap().command else { panic!() };
        assert_eq!(d.port, None);
        assert!(parse(&["dev", "--port", "0"]).is_err());
        assert!(parse(&["dev", "--port", "70000"]).is_err());
    }

    #[test]
    fn invalid_input_is_rejected_with_a_message() {
        assert!(parse(&["info", "--level", "loud"]).is_err());
        assert!(parse(&["--plain", "--json", "info"]).is_err());
        assert!(parse(&["deploy"]).is_err(), "unknown commands are not accepted");
        assert!(parse(&["build"]).is_ok() && parse(&["build", "--out", "x"]).is_ok());
        assert!(parse(&[]).is_err(), "no subcommand shows help");
    }
}

use crate::cli::DevArgs;
use crate::dev::{DevSource, project::Project};
use crate::env::{Caps, EnvInfo};
use crate::error::CliError;
use crate::event::Level;
use crate::session::Session;

pub fn info(caps: &Caps, color: bool, json: bool) -> Result<(), CliError> {
    let env = EnvInfo::collect(caps, color);
    let mode = crate::env::resolve_output(false, false, caps);
    if json {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        println!(
            "{{\"version\":\"{}\",\"cwd\":\"{}\",\"cargo_project\":{},\"term\":\"{}\",\"stdin_tty\":{},\"stdout_tty\":{},\"ci\":{},\"color\":{},\"output\":\"{}\"}}",
            env.version,
            esc(&env.cwd),
            env.cargo_project.as_deref().map_or("null".to_string(), |p| format!("\"{}\"", esc(p))),
            esc(&env.term),
            caps.stdin_tty,
            caps.stdout_tty,
            caps.ci,
            color,
            format!("{mode:?}").to_lowercase()
        );
    } else {
        println!("yawn {}", env.version);
        println!("  cwd            {}", env.cwd);
        println!("  cargo project  {}", env.cargo_project.as_deref().unwrap_or("none found"));
        println!(
            "  terminal       {} (stdin tty: {}, stdout tty: {})",
            env.term, caps.stdin_tty, caps.stdout_tty
        );
        println!("  ci             {}", caps.ci);
        println!("  color          {}", if color { "on" } else { "off" });
        println!("  default output {}", format!("{mode:?}").to_lowercase());
    }
    Ok(())
}

pub fn dev_session(args: &DevArgs, level: Level) -> Result<Session, CliError> {
    let cwd = std::env::current_dir()?;
    let project = Project::detect(&cwd)?;
    Ok(Session {
        title: format!("dev · {}", project.display_name()),
        source: Box::new(DevSource { project, port: args.port }),
        level,
    })
}

/// `yawn rsx`: compiles the project's `.rsx` files and prints rustc-style errors
pub fn rsx(json: bool) -> Result<std::process::ExitCode, CliError> {
    use crate::dev::rsx;
    let cwd = std::env::current_dir()?;
    let project = Project::detect(&cwd)?;
    match rsx::compile(&project.root) {
        Ok(report) => {
            if json {
                println!(
                    "{{\"ok\":true,\"files\":{},\"written\":{}}}",
                    report.sources.len(),
                    report.written
                );
            } else {
                println!(
                    "compiled {} .rsx file(s) ({} generated file(s) updated)",
                    report.sources.len(),
                    report.written
                );
            }
            Ok(std::process::ExitCode::SUCCESS)
        }
        Err(diags) => {
            for d in &diags {
                if json {
                    let l = rsx::locate(d);
                    let esc =
                        |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
                    println!(
                        "{{\"ok\":false,\"file\":\"{}\",\"line\":{},\"col\":{},\"message\":\"{}\"}}",
                        esc(&l.file),
                        l.line,
                        l.col,
                        esc(&l.message)
                    );
                } else {
                    eprintln!("{}", rsx::locate(d).rendered);
                }
            }
            if !json {
                eprintln!("{} error(s) in .rsx files", diags.len());
            }
            Ok(std::process::ExitCode::from(1))
        }
    }
}

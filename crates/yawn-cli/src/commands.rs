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
/// Deletes `target/` and `.yawn/` of the project in the current directory, which the next build
/// makes again. A running `yawn dev` is in the way: it holds both
pub fn clean() -> Result<(), CliError> {
    let cwd = std::env::current_dir()?;
    let project = Project::detect(&cwd)?;
    if yawn_rsx::project::dev_running(&project.root) {
        return Err(CliError::new("yawn dev is running in this project")
            .hint("stop it first (Ctrl-C), it keeps files in target/ and .yawn/"));
    }
    let mut freed = 0u64;
    for name in ["target", ".yawn"] {
        let dir = project.root.join(name);
        if !dir.exists() {
            continue;
        }
        freed += size_of(&dir);
        std::fs::remove_dir_all(&dir)
            .map_err(|e| CliError::new(format!("cannot remove {}: {e}", dir.display())))?;
        println!("removed {}", dir.display());
    }
    println!("freed {:.1} MB", freed as f64 / 1_048_576.0);
    Ok(())
}

fn size_of(dir: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => size_of(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

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

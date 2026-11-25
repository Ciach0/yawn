use crate::cli::BuildArgs;
use crate::dev::project::Project;
use crate::dev::{rsx, tailwind::Tailwind, wasm_bindgen};
use crate::error::CliError;
use crate::finalize;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

const WASM_TARGET: &str = "wasm32-unknown-unknown";
pub(crate) const CLIENT_TARGET_DIR: &str = "target/yawn/cargo/client";
pub(crate) const SERVER_TARGET_DIR: &str = "target/yawn/cargo/server";

fn step(text: &str) {
    eprintln!("yawn: {text}");
}

fn cargo_args(root: &Path, args: &[&str], target_dir: &str) -> Vec<String> {
    let mut v: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    if root.join(yawn_rsx::manifest::MANIFEST).is_file() {
        v.extend(["--manifest-path".into(), yawn_rsx::manifest::MANIFEST.into()]);
    }
    v.extend(["--target-dir".into(), target_dir.into()]);
    v
}

fn cargo(root: &Path, args: &[&str], target_dir: &str, what: &str) -> Result<(), CliError> {
    let status = Command::new("cargo")
        .args(cargo_args(root, args, target_dir))
        .current_dir(root)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| CliError::new(format!("cannot run cargo: {e}")))?;
    yawn_rsx::manifest::sync_lock_back(root);
    if status.success() {
        Ok(())
    } else {
        let mut e = CliError::new(format!("{what} failed ({status})"));
        if what.starts_with("client") {
            e = e.hint(format!("if the target is missing: rustup target add {WASM_TARGET}"));
        }
        Err(e)
    }
}

fn size(path: &Path) -> String {
    let n = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", n.div_ceil(1024))
    }
}

const CLIENT_PROFILE: [(&str, &str); 4] =
    [("opt-level", "\"s\""), ("lto", "\"fat\""), ("codegen-units", "1"), ("panic", "\"abort\"")];

fn has_release_profile(root: &Path) -> bool {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("[profile.release")))
}

fn build_client(root: &Path, project: &Project, pkg: &Path) -> Result<(), CliError> {
    let mut args =
        vec!["rustc", "--release", "--lib", "--target", WASM_TARGET, "--crate-type", "cdylib"];
    let profile: Vec<String> = if has_release_profile(root) {
        Vec::new()
    } else {
        CLIENT_PROFILE
            .iter()
            .flat_map(|(k, v)| ["--config".to_string(), format!("profile.release.{k}={v}")])
            .collect()
    };
    args.extend(profile.iter().map(String::as_str));
    cargo(root, &args, CLIENT_TARGET_DIR, "client build")?;
    let lib = if root.join(yawn_rsx::manifest::MANIFEST).is_file() {
        yawn_rsx::manifest::LIB_NAME.to_string()
    } else {
        project.package.replace('-', "_")
    };
    let wasm =
        root.join(CLIENT_TARGET_DIR).join(WASM_TARGET).join("release").join(format!("{lib}.wasm"));
    if !wasm.is_file() {
        return Err(CliError::new(format!("cargo built no {}", wasm.display()))
            .hint("the project needs a library (src/lib.rs, or src/main.rsx)"));
    }
    let log = |m: &str| eprintln!("      {m}");
    let exe = wasm_bindgen::resolve(root, &log).map_err(CliError::new)?;
    let bindgen = Command::new(&exe)
        .args(["--target", "web", "--no-typescript", "--out-name", "app", "--out-dir"])
        .arg(pkg)
        .arg(&wasm)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| CliError::new(format!("cannot run {}: {e}", exe.display())))?;
    if !bindgen.success() {
        return Err(CliError::new(format!("wasm-bindgen failed ({bindgen})")));
    }
    Ok(())
}

fn build_server(root: &Path, project: &Project) -> Result<PathBuf, CliError> {
    cargo(root, &["build", "--release", "--bin", &project.package], "target", "server build")?;
    let exe = root.join("target/release").join(&project.package);
    if !exe.is_file() {
        return Err(CliError::new(format!("cargo built no {}", exe.display())));
    }
    Ok(exe)
}

pub fn run(args: &BuildArgs) -> Result<ExitCode, CliError> {
    let started = Instant::now();
    let project = Project::detect(&std::env::current_dir()?)?;
    let root = project.root.clone();
    let out: PathBuf = match &args.out {
        Some(o) if o.is_absolute() => o.clone(),
        Some(o) => std::env::current_dir()?.join(o),
        None => root.join("dist"),
    };
    // a separate directory from `yawn dev`'s, so a running dev server keeps its files
    let work = root.join("target/yawn/build");
    let pkg = work.join("pkg");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&pkg)?;

    step("compiling .rsx");
    match rsx::compile_whole(&root) {
        Ok(r) => eprintln!("      {} file(s)", r.sources.len()),
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", rsx::locate(d).rendered);
            }
            return Err(CliError::new(format!("{} error(s) in .rsx files", diags.len())));
        }
    }
    // CSS of the components' `style { }` blocks, written by the compile above
    let rsx_css = root.join(yawn_rsx::project::CSS_FILE);
    if rsx_css.is_file() {
        std::fs::copy(&rsx_css, pkg.join("rsx.css"))?;
    }

    if project.config.tailwind.enabled {
        step("building Tailwind CSS");
        let log = |m: &str| eprintln!("      {m}");
        let tw = Tailwind::prepare(&root, &work, &project.config.tailwind, &log)
            .map_err(CliError::new)?
            .minified();
        tw.build().map_err(CliError::new)?;
    }

    // the client (WASM) and the server are different targets, so cargo builds them side by side
    step("building the client (WASM) and the server (release), in parallel");
    let (client, server_build) = std::thread::scope(|s| {
        let client = s.spawn(|| build_client(&root, &project, &pkg));
        let server = build_server(&root, &project);
        (client.join().unwrap_or_else(|_| Err(CliError::new("client build panicked"))), server)
    });
    client?;
    let exe = server_build?;

    step("hashing files");
    std::fs::create_dir_all(&out)?;
    let public = out.join("public");
    let manifest =
        finalize::run(&finalize::Inputs { assets: &project.assets, pkg: &pkg, out: &public })
            .map_err(|e| CliError::new(format!("cannot write {}: {e}", public.display())))?;
    let server = out.join(&project.package);
    std::fs::copy(&exe, &server)?;
    if root.join(yawn::config::CONFIG_FILE).is_file() {
        std::fs::copy(root.join(yawn::config::CONFIG_FILE), out.join(yawn::config::CONFIG_FILE))?;
    }

    for dir in &project.config.build.copy {
        let from = root.join(dir);
        let name = from
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_owned()))
            .ok_or_else(|| CliError::new(format!("[build] copy: {dir} does not exist")))?;
        let to = out.join(name);
        if to.exists() {
            std::fs::remove_dir_all(&to)?;
        }
        copy_dir(&from, &to).map_err(|e| CliError::new(format!("cannot copy {dir}: {e}")))?;
    }

    eprintln!();
    eprintln!("yawn: built in {:.1}s", started.elapsed().as_secs_f32());
    eprintln!("      server   {} ({})", server.display(), size(&server));
    for key in ["/pkg/app_bg.wasm", "/pkg/app.js", "/pkg/tailwind.css"] {
        if let Some(served) = manifest.get(key) {
            eprintln!(
                "      {key:<18} {} ({})",
                served,
                size(&public.join(served.trim_start_matches('/')))
            );
        }
    }
    eprintln!("      {} file(s) in the manifest", manifest.iter().count());
    eprintln!();
    eprintln!("run it:  cd {} && ./{}", out.display(), project.package);
    Ok(ExitCode::SUCCESS)
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

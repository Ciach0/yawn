//! Downloading a pinned release of a tool into the cache, checked against its published SHA-256

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `<cache>/yawn/<tool>/<version>`, the cache being `$XDG_CACHE_HOME` or `~/.cache`
pub fn cache_dir(tool: &str, version: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".cache"))
        })?;
    Some(base.join("yawn").join(tool).join(version))
}

pub fn run_ok(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(super::cargo::strip_ansi(String::from_utf8_lossy(&out.stderr).trim()))
    }
}

pub fn sha256(file: &Path) -> Result<String, String> {
    let try_tool = |prog: &str, args: &[&str]| {
        run_ok(Command::new(prog).args(args).arg(file))
            .ok()
            .and_then(|o| o.split_whitespace().next().map(str::to_ascii_lowercase))
    };
    try_tool("sha256sum", &[]).or_else(|| try_tool("shasum", &["-a", "256"])).ok_or_else(|| {
        "cannot verify the download: neither `sha256sum` nor `shasum` is available".to_string()
    })
}

/// Downloads `url` into `to`, or returns its body when `to` is `None`; `instead` says what else the
/// user can do when it fails
pub fn curl(url: &str, to: Option<&Path>, instead: &str) -> Result<String, String> {
    let mut c = Command::new("curl");
    c.args(["--fail", "--silent", "--show-error", "--location", "--max-time", "300"]);
    match to {
        Some(p) => c.arg("--output").arg(p),
        None => c.arg("--output").arg("-"),
    };
    c.arg(url);
    run_ok(&mut c).map_err(|e| format!("download failed ({e}); install curl, or {instead}"))
}

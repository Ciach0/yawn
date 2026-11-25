use crate::error::CliError;
use std::process::{Command, Stdio};

const PACKAGE: &str = "yawn-cli";

/// Installs the newest release of the `yawn` tool, or with `check` only says whether there is one
pub fn run(check: bool) -> Result<(), CliError> {
    let current = env!("CARGO_PKG_VERSION");
    let latest = latest_release()?;
    if !is_newer(&latest, current) {
        println!("yawn {current} is the latest release");
        return Ok(());
    }
    if check {
        println!("yawn {latest} is available, this is {current}: run `yawn upgrade`");
        return Ok(());
    }
    let mut install = Command::new("cargo");
    if has_binstall() {
        install.args(["binstall", "--no-confirm", PACKAGE]);
    } else {
        install.args(["install", "--locked", PACKAGE]);
    }
    println!("upgrading yawn {current} -> {latest}");
    let status = install.status().map_err(|e| {
        CliError::new(format!("cannot run cargo: {e}")).hint("Cargo is needed to upgrade")
    })?;
    if status.success() { Ok(()) } else { Err(CliError::new("the upgrade failed")) }
}

fn latest_release() -> Result<String, CliError> {
    let out =
        Command::new("cargo").args(["search", PACKAGE, "--limit", "1"]).output().map_err(|e| {
            CliError::new(format!("cannot run cargo: {e}")).hint("Cargo is needed to upgrade")
        })?;
    if !out.status.success() {
        return Err(CliError::new("cannot look up the latest release")
            .hint(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    parse_search(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| CliError::new(format!("`{PACKAGE}` was not found in the registry")))
}

fn has_binstall() -> bool {
    Command::new("cargo")
        .args(["binstall", "-V"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The version in the first line of `cargo search`: `yawn-cli = "0.2.0"    # description`
fn parse_search(output: &str) -> Option<String> {
    let rest =
        output.lines().find_map(|l| l.strip_prefix(PACKAGE)?.trim_start().strip_prefix('='))?;
    let rest = rest.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// Whether `a` is a later version than `b`; a pre-release is earlier than its release
fn is_newer(a: &str, b: &str) -> bool {
    fn key(v: &str) -> (Vec<u64>, bool) {
        let (core, pre) = match v.split_once('-') {
            Some((core, _)) => (core, true),
            None => (v, false),
        };
        (core.split('.').map(|n| n.parse().unwrap_or(0)).collect(), !pre)
    }
    key(a) > key(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_out_of_cargo_search() {
        let out = "yawn-cli = \"0.2.1\"    # The `yawn` command line tool\nyawn-cli-extra = \"9.9.9\"  # other\n";
        assert_eq!(parse_search(out).as_deref(), Some("0.2.1"));
        assert_eq!(parse_search("yawn-kit = \"1.0.0\"\n"), None);
        assert_eq!(parse_search(""), None);
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.2.0", "0.2.0-rc.1"));
        assert!(!is_newer("0.2.0-rc.1", "0.2.0"));
    }
}

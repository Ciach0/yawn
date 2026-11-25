//! The `wasm-bindgen` CLI the project needs: it must be the same version as the `wasm-bindgen`
//! crate in its `Cargo.lock`. The one on `PATH` when it matches, else that release downloaded once
//! into the cache.

use super::fetch;
use std::path::{Path, PathBuf};
use std::process::Command;
use yawn::config;

const EXE: &str = if cfg!(windows) { "wasm-bindgen.exe" } else { "wasm-bindgen" };

/// Target triple of the release archive for a platform
pub fn target_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        // the only aarch64 Linux build in older releases
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// Version from `wasm-bindgen --version` output (`wasm-bindgen 0.2.129`)
pub fn parse_version(out: &str) -> Option<&str> {
    out.split_whitespace().nth(1)
}

fn install_hint(version: &str) -> String {
    format!("run: cargo install -f wasm-bindgen-cli --version {version}")
}

/// The executable to run for the project at `root`
pub fn resolve(root: &Path, log: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let on_path = fetch::run_ok(Command::new(EXE).arg("--version")).ok();
    let have = on_path.as_deref().and_then(parse_version);
    match config::locked_version(root, "wasm-bindgen") {
        Some(want) if have != Some(want.as_str()) => download(&want, log),
        // no lock to match: whatever is installed
        None if have.is_none() => {
            Err("wasm-bindgen not found. Install it: cargo install wasm-bindgen-cli".to_string())
        }
        _ => Ok(PathBuf::from(EXE)),
    }
}

fn download(version: &str, log: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let target = target_name(os, arch).ok_or_else(|| {
        format!(
            "no wasm-bindgen {version} release build for {os}/{arch}; {}",
            install_hint(version)
        )
    })?;
    let dir = fetch::cache_dir("wasm-bindgen", version)
        .ok_or("cannot find a cache directory (set HOME or XDG_CACHE_HOME)")?;
    let exe = dir.join(EXE);
    if exe.is_file() {
        return Ok(exe);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let name = format!("wasm-bindgen-{version}-{target}");
    let url = format!(
        "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/{version}/{name}.tar.gz"
    );
    log(&format!(
        "downloading wasm-bindgen {version} ({target}) from github.com/wasm-bindgen/wasm-bindgen, once"
    ));
    let instead = install_hint(version);
    // `<hex>  gh-release/<archive>`
    let sums = fetch::curl(&format!("{url}.sha256sum"), None, &instead)?;
    let want = sums
        .split_whitespace()
        .next()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| format!("{name}.tar.gz.sha256sum is empty"))?;
    let part = dir.join(format!("{name}.tar.gz.part"));
    fetch::curl(&url, Some(&part), &instead)?;
    let got = fetch::sha256(&part)?;
    if got != want {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "checksum mismatch for {name}.tar.gz: expected {want}, got {got}; the download was discarded"
        ));
    }
    let unpacked = fetch::run_ok(Command::new("tar").arg("-xzf").arg(&part).arg("-C").arg(&dir));
    let _ = std::fs::remove_file(&part);
    unpacked.map_err(|e| format!("cannot unpack {name}.tar.gz ({e}); {instead}"))?;
    std::fs::rename(dir.join(&name).join(EXE), &exe)
        .map_err(|e| format!("cannot install {}: {e}", exe.display()))?;
    let _ = std::fs::remove_dir_all(dir.join(&name));
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_names() {
        assert_eq!(target_name("linux", "x86_64"), Some("x86_64-unknown-linux-musl"));
        assert_eq!(target_name("macos", "aarch64"), Some("aarch64-apple-darwin"));
        assert_eq!(target_name("windows", "x86_64"), Some("x86_64-pc-windows-msvc"));
        assert_eq!(target_name("windows", "aarch64"), None);
        assert_eq!(target_name("freebsd", "x86_64"), None);
    }

    #[test]
    fn version_output() {
        assert_eq!(parse_version("wasm-bindgen 0.2.129\n"), Some("0.2.129"));
        assert_eq!(parse_version(""), None);
    }

    #[test]
    #[ignore]
    fn real_wasm_bindgen_download() {
        let cache = std::env::temp_dir().join(format!("yawn-wb-{}", std::process::id()));
        // SAFETY: an ignored test run on its own, no other thread reads the environment
        unsafe { std::env::set_var("XDG_CACHE_HOME", &cache) };
        let exe = download("0.2.129", &|m| eprintln!("{m}")).unwrap();
        let out = fetch::run_ok(Command::new(&exe).arg("--version")).unwrap();
        assert_eq!(parse_version(&out), Some("0.2.129"));
        assert_eq!(download("0.2.129", &|_| panic!("downloaded twice")).unwrap(), exe);
        std::fs::remove_dir_all(&cache).unwrap();
    }
}

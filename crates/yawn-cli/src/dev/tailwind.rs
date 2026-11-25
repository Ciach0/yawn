use super::fetch;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use yawn::config::{TAILWIND_FILE, TailwindSection};

/// The Tailwind release `yawn dev` downloads
pub const VERSION: &str = "4.3.5";

const DEFAULT_INPUT: &str = "@import \"tailwindcss\" source(none);\n@source \"../src\";\n";

pub struct Tailwind {
    minify: bool,
    bin: PathBuf,
    input: PathBuf,
    output: PathBuf,
    root: PathBuf,
}

/// What a successful run did
pub struct Built {
    pub duration: Duration,
    /// Whether the output differs from the previous one
    pub changed: bool,
}

/// Release asset for a platform, e.g. `tailwindcss-linux-x64`
pub fn asset_name(os: &str, arch: &str) -> Option<String> {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    match os {
        "linux" => Some(format!("tailwindcss-linux-{arch}")),
        "macos" => Some(format!("tailwindcss-macos-{arch}")),
        "windows" if arch == "x64" => Some("tailwindcss-windows-x64.exe".to_string()),
        _ => None,
    }
}

/// Hash listed for `asset` in a `sha256sums.txt` (`<hex>  ./<name>` per line)
pub fn checksum_for(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches("./").trim_start_matches('*') == asset)
            .then(|| hash.to_ascii_lowercase())
    })
}

fn download(log: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let asset = asset_name(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        format!(
            "no Tailwind standalone build for {}/{}; set `[tailwind] binary` in Yawn.toml",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let dir = fetch::cache_dir("tailwind", VERSION)
        .ok_or("cannot find a cache directory (set HOME or XDG_CACHE_HOME)")?;
    let exe = dir.join(if cfg!(windows) { "tailwindcss.exe" } else { "tailwindcss" });
    if exe.is_file() {
        return Ok(exe);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let base = format!("https://github.com/tailwindlabs/tailwindcss/releases/download/v{VERSION}");
    log(&format!(
        "downloading tailwindcss v{VERSION} ({asset}) from github.com/tailwindlabs/tailwindcss, once"
    ));
    let curl =
        |url: &str, to: Option<&Path>| fetch::curl(url, to, "set `[tailwind] binary` in Yawn.toml");
    let sums = curl(&format!("{base}/sha256sums.txt"), None)?;
    let want = checksum_for(&sums, &asset)
        .ok_or_else(|| format!("sha256sums.txt of v{VERSION} does not list {asset}"))?;
    let part = dir.join(format!("{asset}.part"));
    curl(&format!("{base}/{asset}"), Some(&part))?;
    let got = fetch::sha256(&part)?;
    if got != want {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "checksum mismatch for {asset}: expected {want}, got {got}; the download was discarded"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(&part, &exe).map_err(|e| format!("cannot install {}: {e}", exe.display()))?;
    Ok(exe)
}

impl Tailwind {
    /// Resolves the executable and input for a project
    pub fn prepare(
        root: &Path,
        dist: &Path,
        cfg: &TailwindSection,
        log: &dyn Fn(&str),
    ) -> Result<Tailwind, String> {
        let bin = match &cfg.binary {
            Some(b) => {
                let p = root.join(b);
                if !p.is_file() {
                    return Err(format!(
                        "[tailwind] binary = \"{b}\": no such file ({})",
                        p.display()
                    ));
                }
                p
            }
            None => download(log)?,
        };
        let input = match &cfg.input {
            Some(i) => {
                let p = root.join(i);
                if !p.is_file() {
                    return Err(format!(
                        "[tailwind] input = \"{i}\": no such file ({})",
                        p.display()
                    ));
                }
                p
            }
            None => {
                let p = root.join(".yawn").join("tailwind.css");
                std::fs::create_dir_all(p.parent().expect("has parent"))
                    .map_err(|e| e.to_string())?;
                if std::fs::read_to_string(&p).ok().as_deref() != Some(DEFAULT_INPUT) {
                    std::fs::write(&p, DEFAULT_INPUT)
                        .map_err(|e| format!("cannot write {}: {e}", p.display()))?;
                }
                p
            }
        };
        Ok(Tailwind {
            minify: false,
            bin,
            input,
            output: dist.join(TAILWIND_FILE),
            root: root.to_path_buf(),
        })
    }

    /// Production output: minified
    pub fn minified(mut self) -> Self {
        self.minify = true;
        self
    }

    pub fn build(&self) -> Result<Built, String> {
        let before = std::fs::read(&self.output).ok();
        std::fs::create_dir_all(self.output.parent().expect("has parent"))
            .map_err(|e| e.to_string())?;
        let t = Instant::now();
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(&self.root)
            .arg("-i")
            .arg(&self.input)
            .arg("-o")
            .arg(&self.output)
            .env("NO_COLOR", "1");
        if self.minify {
            cmd.arg("--minify");
        }
        let out = cmd
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {}: {e}", self.bin.display()))?;
        if !out.status.success() {
            let text = super::cargo::strip_ansi(&String::from_utf8_lossy(&out.stderr));
            let text = text.trim();
            return Err(if text.is_empty() {
                format!("tailwindcss exited with {}", out.status)
            } else {
                text.to_string()
            });
        }
        Ok(Built { duration: t.elapsed(), changed: std::fs::read(&self.output).ok() != before })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names() {
        assert_eq!(asset_name("linux", "x86_64").as_deref(), Some("tailwindcss-linux-x64"));
        assert_eq!(asset_name("macos", "aarch64").as_deref(), Some("tailwindcss-macos-arm64"));
        assert_eq!(asset_name("windows", "x86_64").as_deref(), Some("tailwindcss-windows-x64.exe"));
        assert_eq!(asset_name("linux", "riscv64"), None);
        assert_eq!(asset_name("freebsd", "x86_64"), None);
    }

    #[test]
    fn checksum_lines() {
        let sums = "AABB  ./tailwindcss-linux-x64\nccdd  ./tailwindcss-linux-x64-musl\n";
        assert_eq!(checksum_for(sums, "tailwindcss-linux-x64").as_deref(), Some("aabb"));
        assert_eq!(checksum_for(sums, "tailwindcss-linux-x64-musl").as_deref(), Some("ccdd"));
        assert_eq!(checksum_for(sums, "tailwindcss-macos-x64"), None);
    }

    #[test]
    fn missing_custom_binary_is_explained() {
        let cfg = TailwindSection { enabled: true, binary: Some("nope".into()), input: None };
        let err = Tailwind::prepare(
            Path::new("/nonexistent"),
            Path::new("/nonexistent/d"),
            &cfg,
            &|_| {},
        )
        .err()
        .unwrap();
        assert!(err.contains("[tailwind] binary"), "{err}");
    }

    #[test]
    #[ignore]
    fn real_tailwind_download_and_build() {
        let dir = std::env::temp_dir().join(format!("yawn-tw-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/app.rsx"),
            "component App() { view { <p class=\"flex p-4 md:text-xl text-red-500\">\"hi\"</p> } }",
        )
        .unwrap();
        let cfg = TailwindSection { enabled: true, ..Default::default() };
        let t = Tailwind::prepare(&dir, &dir.join("dist"), &cfg, &|m| eprintln!("{m}")).unwrap();
        assert!(t.build().unwrap().changed);
        let css = std::fs::read_to_string(dir.join("dist").join(TAILWIND_FILE)).unwrap();
        for want in [".flex", ".p-4", ".text-red-500", "md\\:text-xl"] {
            assert!(css.contains(want), "missing {want} in {css}");
        }
        assert!(!t.build().unwrap().changed, "unchanged sources give identical output");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

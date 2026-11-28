use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A stylesheet under the assets directory; the value is its URL (`/assets/...`)
    Css(String),
    /// Any other asset: the page reloads
    Asset,
    /// A stylesheet under `src/` (Tailwind input): rebuild the Tailwind CSS, swap it live
    Tailwind,
    /// Client (WASM) sources
    Client,
    /// Server sources (`src/main.rs`, `src/bin/`)
    Server,
    /// `Cargo.toml` or `build.rs`: both sides
    Both,
    /// `Yawn.toml` or a `.env` file in the project root: the server restarts (it reads both when it
    /// starts)
    Config,
    /// A file under a `[dev] watch` directory: the server's hooks run, the page reloads, nothing is
    /// built
    Data,
}

fn is_noise(name: &str) -> bool {
    name.starts_with('.')
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with(".tmp")
        || name == "4913"
}

pub fn classify(
    root: &Path,
    assets: &Path,
    data: &[PathBuf],
    libs: &[PathBuf],
    path: &Path,
) -> Option<Change> {
    if data.iter().any(|d| path.starts_with(d)) {
        let name = path.file_name()?.to_str()?;
        return (!is_noise(name)).then_some(Change::Data);
    }
    // A component library
    if libs.iter().any(|l| path.starts_with(l.join("src"))) {
        let name = path.file_name()?.to_str()?;
        return (!is_noise(name)).then_some(Change::Both);
    }
    let rel = path.strip_prefix(root).ok()?;
    let name = rel.file_name()?.to_str()?;
    // `.env`, `.env.local`... in the root.
    if rel.components().count() == 1
        && (name == ".env" || name.starts_with(".env."))
        && !name.ends_with('~')
        && !name.ends_with(".swp")
        && !name.ends_with(".swx")
        && !name.ends_with(".tmp")
    {
        return Some(Change::Config);
    }
    if is_noise(name)
        || rel.components().any(|c| {
            matches!(c.as_os_str().to_str(), Some("target" | ".git" | ".yawn" | "node_modules"))
        })
    {
        return None;
    }
    if let Ok(in_assets) = path.strip_prefix(assets) {
        let url = format!("/assets/{}", in_assets.to_string_lossy().replace('\\', "/"));
        return Some(if in_assets.extension().is_some_and(|e| e == "css") {
            Change::Css(url)
        } else {
            Change::Asset
        });
    }
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    match rel_str.as_str() {
        "Cargo.toml" | "build.rs" => return Some(Change::Both),
        "Yawn.toml" => return Some(Change::Config),
        "src/main.rs" => return Some(Change::Server),
        _ => {}
    }
    if rel_str.starts_with("src/bin/") {
        return Some(Change::Server);
    }
    if rel_str.starts_with("src/") && rel.extension().is_some_and(|e| e == "css") {
        return Some(Change::Tailwind);
    }
    // A component file
    if rel_str.starts_with("src/")
        && rel.extension().is_some_and(|e| e == "rsx")
        && rel_str != "src/main.rsx"
        && !rel_str.starts_with("src/api/")
        && !rel_str.starts_with("src/server/")
    {
        return Some(Change::Client);
    }
    // A plain module whose functions a dev build can replace: a change of its body is the client's
    if rel_str.starts_with("src/")
        && rel.extension().is_some_and(|e| e == "rs")
        && let Ok(under_src) = rel.strip_prefix("src")
        && root
            .join(yawn_rsx::project::OUT_DIR)
            .join(yawn_rsx::mirror::DIR)
            .join(under_src)
            .is_file()
    {
        return Some(Change::Client);
    }
    // Handlers (when no mirror of them exists yet) and the server's own modules never reach the browser
    if (rel_str.starts_with("src/api/")
        || rel_str.starts_with("src/server/")
        || rel_str == "src/server.rs")
        && rel.extension().is_some_and(|e| e == "rs")
    {
        return Some(Change::Server);
    }

    // Any other source is both!!
    rel_str.starts_with("src/").then_some(Change::Both)
}

/// Everything that changed since the last cycle
#[derive(Debug, Default, Clone)]
pub struct ChangeSet {
    pub css: BTreeSet<String>,
    pub asset: bool,
    /// A stylesheet in `src/` changed
    pub tailwind: bool,
    pub client: bool,
    pub server: bool,
    pub restart: bool,
    /// Files changed under a `[dev] watch` directory
    pub data: BTreeSet<PathBuf>,
    /// somebody pressed `r`
    pub manual: bool,
    /// Project-relative paths, for the log
    pub files: BTreeSet<PathBuf>,
    /// When the first change of this set was seen
    pub since: Option<Instant>,
    /// First cycle
    pub initial: bool,
}

impl ChangeSet {
    pub fn initial() -> Self {
        ChangeSet { client: true, server: true, initial: true, ..Default::default() }
    }

    /// Adds a changed path; returns whether it was relevant
    pub fn add(
        &mut self,
        root: &Path,
        assets: &Path,
        data: &[PathBuf],
        libs: &[PathBuf],
        path: &Path,
        at: Instant,
    ) -> bool {
        let Some(change) = classify(root, assets, data, libs, path) else {
            return false;
        };
        match change {
            Change::Css(url) => {
                self.css.insert(url);
            }
            Change::Asset => self.asset = true,
            Change::Tailwind => self.tailwind = true,
            Change::Client => self.client = true,
            Change::Server => self.server = true,
            Change::Both => {
                self.client = true;
                self.server = true;
            }
            Change::Config => self.restart = true,
            Change::Data => {
                self.data.insert(path.to_path_buf());
            }
        }
        self.files.insert(path.strip_prefix(root).unwrap_or(path).to_path_buf());
        self.since = Some(self.since.map_or(at, |s| s.min(at)));
        true
    }

    /// Adds everything in `other` to this set. A cycle cut short passes its work on this way
    pub fn merge(&mut self, other: ChangeSet) {
        self.css.extend(other.css);
        self.asset |= other.asset;
        self.tailwind |= other.tailwind;
        self.client |= other.client;
        self.server |= other.server;
        self.restart |= other.restart;
        self.manual |= other.manual;
        self.data.extend(other.data);
        self.initial |= other.initial;
        self.files.extend(other.files);
        self.since = match (self.since, other.since) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    pub fn manual() -> Self {
        ChangeSet {
            client: true,
            server: true,
            manual: true,
            since: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && !self.initial && !self.manual
    }

    /// is tailwind output stale?
    pub fn needs_tailwind(&self) -> bool {
        self.tailwind || self.client
    }

    pub fn needs_build(&self) -> bool {
        self.client || self.server
    }

    pub fn needs_reload(&self) -> bool {
        self.needs_build() || self.asset || self.restart || !self.data.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/p";

    fn c(path: &str) -> Option<Change> {
        classify(
            Path::new(ROOT),
            Path::new("/p/assets"),
            &[PathBuf::from("/t")],
            &[PathBuf::from("/lib")],
            Path::new(path),
        )
    }

    #[test]
    fn classifies_by_location() {
        assert_eq!(c("/p/assets/style.css"), Some(Change::Css("/assets/style.css".into())));
        assert_eq!(c("/p/assets/css/a.css"), Some(Change::Css("/assets/css/a.css".into())));
        assert_eq!(c("/p/assets/logo.svg"), Some(Change::Asset));
        assert_eq!(c("/p/src/lib.rs"), Some(Change::Both));
        assert_eq!(
            c("/p/src/counter.rsx"),
            Some(Change::Client),
            "components only run in the browser; the server's render is switched off"
        );
        assert_eq!(c("/p/src/ui/card.rsx"), Some(Change::Client));
        assert_eq!(c("/p/src/main.rsx"), Some(Change::Both));
        assert_eq!(c("/p/src/api/x.rsx"), Some(Change::Both));
        assert_eq!(c("/p/src/ui/button.rs"), Some(Change::Both));
        assert_eq!(c("/p/src/main.rs"), Some(Change::Server));
        assert_eq!(c("/p/src/server/db.rs"), Some(Change::Server), "compiled for the server only");
        assert_eq!(c("/p/src/server.rs"), Some(Change::Server));
        assert_eq!(c("/p/src/api/users/[id].rs"), Some(Change::Server));
        assert_eq!(c("/p/Cargo.toml"), Some(Change::Both));
        assert_eq!(c("/lib/src/ui/card.rsx"), Some(Change::Both));
        assert_eq!(c("/lib/src/.hidden"), None);
        assert_eq!(c("/lib/target/x"), None);
        assert_eq!(c("/p/.env"), Some(Change::Config));
        assert_eq!(c("/p/.env.local"), Some(Change::Config));
        assert_eq!(c("/p/.env.swp"), None);
        assert_eq!(c("/p/src/.env"), None);
        assert_eq!(c("/p/Yawn.toml"), Some(Change::Config));
        assert_eq!(
            c("/t/pl/panel/a.ftl"),
            Some(Change::Data),
            "a watched directory may lie outside the project"
        );
        assert_eq!(c("/t/pl/.a.ftl.swp"), None);
    }

    #[test]
    fn ignores_generated_and_editor_files() {
        for p in [
            "/p/target/debug/x.rs",
            "/p/target/yawn/dist/pkg/app.js",
            "/p/.yawn/rsx/counter.rs",
            "/p/.git/index",
            "/p/src/.lib.rs.swp",
            "/p/src/lib.rs~",
            "/p/src/.#lib.rs",
            "/p/src/4913",
            "/p/Cargo.lock",
            "/p/README.md",
            "/elsewhere/src/lib.rs",
        ] {
            assert_eq!(c(p), None, "{p}");
        }
    }

    #[test]
    fn merge_keeps_what_an_abandoned_cycle_owed() {
        let (root, assets) = (Path::new(ROOT), Path::new("/p/assets"));
        let mut cut = ChangeSet::default();
        cut.add(root, assets, &[], &[], Path::new("/p/src/main.rs"), Instant::now());
        let mut next = ChangeSet::default();
        next.add(root, assets, &[], &[], Path::new("/p/assets/style.css"), Instant::now());
        next.merge(cut);
        assert!(next.server && !next.client, "the server rebuild survives");
        assert!(next.css.contains("/assets/style.css") && next.files.len() == 2);
        assert!(ChangeSet::default().is_empty());
        let mut first = ChangeSet::default();
        first.merge(ChangeSet::initial());
        assert!(first.initial && first.client && first.server && !first.is_empty());
    }

    #[test]
    fn merging_keeps_earliest_time_and_decides_actions() {
        let t0 = Instant::now();
        let t1 = t0 + std::time::Duration::from_millis(30);
        let (root, assets) = (Path::new(ROOT), Path::new("/p/assets"));
        let mut cs = ChangeSet::default();
        assert!(cs.add(root, assets, &[], &[], Path::new("/p/assets/style.css"), t1));
        assert!(!cs.needs_reload(), "css alone is a live swap");
        assert!(cs.add(root, assets, &[], &[], Path::new("/p/src/lib.rs"), t0));
        assert!(!cs.add(root, assets, &[], &[], Path::new("/p/target/x"), t0));
        assert_eq!(cs.since, Some(t0));
        assert!(cs.needs_build() && cs.needs_reload() && cs.server && cs.client);
        assert_eq!(cs.files.len(), 2);
        assert!(cs.files.contains(Path::new("src/lib.rs")));
        assert!(ChangeSet::default().is_empty() && !ChangeSet::initial().is_empty());
    }
}

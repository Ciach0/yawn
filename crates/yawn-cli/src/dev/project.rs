use crate::error::CliError;
use std::path::{Path, PathBuf};
use yawn::config::{self, CONFIG_FILE, DEV_DIR, DIST_DIR, ProjectConfig};

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: ProjectConfig,
    /// Cargo package name; also the server binary name
    pub package: String,
    /// Compiled client output (`<dist>/pkg/app.js`, `app_bg.wasm`)
    pub dist: PathBuf,
    /// What the dev session keeps for itself (the literals it patches in)
    pub dev_dir: PathBuf,
    pub assets: PathBuf,
    /// `[dev] watch` directories, made absolute; a change under one is a data change
    pub data: Vec<PathBuf>,
    /// Component libraries (path dependencies that are Yawn projects): their `src/` is watched like
    /// the project's own
    pub libs: Vec<PathBuf>,
    /// Plain Rust crates the project depends on by path (not Yawn projects): their `src/` is
    /// watched, and a change in it rebuilds
    pub crates: Vec<PathBuf>,
}

impl Project {
    /// The component libraries and the plain crates the project builds from: every source in their
    /// `src/` is used by both sides
    pub fn watched_crates(&self) -> Vec<PathBuf> {
        self.libs.iter().chain(self.crates.iter()).cloned().collect()
    }

    /// Finds the project containing `start` (a directory with `Yawn.toml`) and reads its config
    pub fn detect(start: &Path) -> Result<Project, CliError> {
        let root = config::find_project_root(start).ok_or_else(|| {
            CliError::new(format!("no {CONFIG_FILE} found in {} or its parents", start.display()))
                .hint("run `yawn dev` inside a Yawn project, for example examples/counter")
        })?;
        Self::open(root)
    }

    pub fn open(root: PathBuf) -> Result<Project, CliError> {
        let config = ProjectConfig::load(&root)
            .map_err(|e| CliError::new(e.to_string()))?
            .unwrap_or_default();
        let package = config::read_package_name(&root).map_err(|e| {
            CliError::new(e.to_string()).hint("a Yawn project is also a Cargo package")
        })?;
        let data = config
            .dev
            .watch
            .iter()
            .map(|w| {
                let p = root.join(w);
                std::fs::canonicalize(&p).unwrap_or(p)
            })
            .collect();
        let libs = yawn_rsx::project::libraries(&root);
        let crates = yawn_rsx::project::source_crates(&root);
        Ok(Project {
            data,
            libs,
            crates,
            dist: root.join(DIST_DIR),
            dev_dir: root.join(DEV_DIR),
            assets: root.join(config.assets_dir()),
            root,
            config,
            package,
        })
    }

    pub fn display_name(&self) -> &str {
        self.config.app.name.as_deref().unwrap_or(&self.package)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_reads_package_and_paths() {
        let dir = std::env::temp_dir().join(format!("yawn-proj-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(
            dir.join(CONFIG_FILE),
            "[app]\nname = \"Shop\"\n[dev]\nassets = \"public\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"shop-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let p = Project::detect(&dir.join("src/deep")).unwrap();
        assert_eq!(p.root, dir);
        assert_eq!(p.package, "shop-app");
        assert_eq!(p.display_name(), "Shop");
        assert_eq!(p.assets, dir.join("public"));
        assert!(p.dist.ends_with("target/yawn/dist"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_project_explains_what_to_do() {
        let e = Project::detect(Path::new("/")).unwrap_err();
        assert!(e.message.contains("Yawn.toml") && e.hint.is_some());
    }
}

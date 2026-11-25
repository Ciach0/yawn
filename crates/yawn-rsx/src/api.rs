use crate::routes::map_name;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Folder under `src/` that holds the endpoints
pub const DIR: &str = "api";

/// HTTP methods a file can answer, as the names of the handler functions
pub const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];

/// One file of `src/api/`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Module path below the generated `api` module (`["users", "param_id"]`)
    pub module: Vec<String>,
    pub file: PathBuf,
    /// The route in axum syntax: `/api/users/{id}`
    pub path: String,
    /// The methods the file defines, in `METHODS` order
    pub methods: Vec<&'static str>,
}

fn is_private(name: &str) -> bool {
    name.strip_prefix('_').is_some_and(|r| !r.starts_with('_') && !r.is_empty())
}

fn is_ident(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Which handlers `source` defines: top-level `pub fn get(` or `pub async fn get(` and so on
pub fn methods_in(source: &str) -> Vec<&'static str> {
    METHODS
        .into_iter()
        .filter(|m| {
            source.lines().any(|l| {
                let l = l.strip_prefix("pub ").unwrap_or("\0");
                let l = l.strip_prefix("async ").unwrap_or(l);
                l.strip_prefix("fn ").and_then(|r| r.strip_prefix(m)).is_some_and(|r| {
                    r.starts_with(['(', '<']) || r.trim_start().starts_with(['(', '<'])
                })
            })
        })
        .collect()
}

fn axum_segment(seg: &str) -> String {
    if let Some(name) = seg.strip_prefix(':') {
        format!("{{{name}}}")
    } else if let Some(name) = seg.strip_prefix('*') {
        format!("{{*{name}}}")
    } else {
        seg.to_string()
    }
}

/// Reads `src/api/`
pub fn scan(root: &Path) -> Result<Vec<Endpoint>, Vec<(String, String)>> {
    let base = root.join("src").join(DIR);
    let mut found = Vec::new();
    let mut errors = Vec::new();
    walk(&base, &mut Vec::new(), &mut Vec::new(), false, &mut found, &mut errors);
    found
        .sort_by(|a: &Endpoint, b| (a.path.as_str(), &a.module).cmp(&(b.path.as_str(), &b.module)));
    // `/api/users/{id}` and `/api/users/{name}` are the same route
    let shape = |p: &str| {
        p.split('/')
            .map(|s| {
                if s.starts_with("{*") {
                    "{*}"
                } else if s.starts_with('{') {
                    "{}"
                } else {
                    s
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    };
    let routes: Vec<&Endpoint> = found.iter().filter(|e| !e.methods.is_empty()).collect();
    for pair in routes.windows(2) {
        if shape(&pair[0].path) == shape(&pair[1].path) {
            errors.push((
                rel(root, &pair[1].file),
                format!("`{}` and `{}` are the same route", pair[0].path, pair[1].path),
            ));
        }
    }
    if errors.is_empty() { Ok(found) } else { Err(errors) }
}

fn rel(root: &Path, file: &Path) -> String {
    file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/")
}

fn walk(
    dir: &Path,
    modules: &mut Vec<String>,
    segs: &mut Vec<String>,
    private: bool,
    found: &mut Vec<Endpoint>,
    errors: &mut Vec<(String, String)>,
) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let shown = path.to_string_lossy().into_owned();
        if path.is_dir() && (private || is_private(&name)) {
            if !is_ident(&name) {
                errors.push((shown, "a private folder is named like a Rust module".into()));
                continue;
            }
            modules.push(name);
            walk(&path, modules, segs, true, found, errors);
            modules.pop();
        } else if path.is_dir() {
            match map_name(&name, true) {
                Ok((m, seg)) => {
                    modules.push(m);
                    let pushed = seg.is_some();
                    segs.extend(seg);
                    walk(&path, modules, segs, false, found, errors);
                    modules.pop();
                    if pushed {
                        segs.pop();
                    }
                }
                Err(e) => errors.push((shown, e)),
            }
        } else if let Some(stem) = name.strip_suffix(".rs").filter(|s| private || is_private(s)) {
            if !is_ident(stem) {
                errors.push((shown, "a private file is named like a Rust module".into()));
                continue;
            }
            let mut full = modules.clone();
            full.push(stem.to_string());
            found.push(Endpoint {
                module: full,
                file: path,
                path: String::new(),
                methods: Vec::new(),
            });
        } else if let Some(stem) = name.strip_suffix(".rs") {
            let mapped = if stem == "index" {
                Ok(("index".to_string(), None))
            } else {
                map_name(stem, false)
            };
            let (module, seg) = match mapped {
                Ok(x) => x,
                Err(e) => {
                    errors.push((shown, e));
                    continue;
                }
            };
            let Ok(source) = fs::read_to_string(&path) else {
                errors.push((shown, "cannot read file".into()));
                continue;
            };
            let methods = methods_in(&source);
            if methods.is_empty() {
                errors.push((shown, format!("an endpoint defines at least one handler: `pub async fn get(...)`, or one of {}", METHODS.join(", "))));
                continue;
            }
            let mut all_segs = segs.clone();
            all_segs.extend(seg);
            let url: String = all_segs.iter().map(|s| format!("/{}", axum_segment(s))).collect();
            let mut full = modules.clone();
            full.push(module);
            found.push(Endpoint {
                module: full,
                file: path,
                path: format!("/{DIR}{url}"),
                methods,
            });
        }
    }
}

#[derive(Default)]
struct Tree {
    files: BTreeMap<String, usize>,
    dirs: BTreeMap<String, Tree>,
}

/// Where the generated `api` module lives, below the output directory
pub const MODULE_DIR: &str = "_api";

/// The `mod api;` line for the crate root
pub fn mod_decl(dir: &Path) -> String {
    format!(
        "\n#[cfg(not(target_arch = \"wasm32\"))]\n#[path = {:?}]\npub mod api;\n",
        dir.join("mod.rs").to_string_lossy()
    )
}

/// The `api` module as real files in `dir`: a `mod.rs` per folder (the endpoints as modules by
/// `#[path]`, since `[id].rs` isn't a module name) and, in the root one, the router that serves
/// them
pub fn module_files(dir: &Path, endpoints: &[Endpoint]) -> Vec<(PathBuf, String)> {
    let mut root = Tree::default();
    for (i, e) in endpoints.iter().enumerate() {
        let (file, dirs) = e.module.split_last().expect("a module path");
        let mut t = &mut root;
        for d in dirs {
            t = t.dirs.entry(d.clone()).or_default();
        }
        t.files.insert(file.clone(), i);
    }
    fn mods(
        t: &Tree,
        endpoints: &[Endpoint],
        dir: &Path,
        out: &mut Vec<(PathBuf, String)>,
    ) -> String {
        let mut text = String::from("// @generated by yawn-rsx: the modules of this folder.\n");
        for (name, i) in &t.files {
            text.push_str(&format!(
                "#[path = {:?}]\npub mod {name};\n",
                endpoints[*i].file.to_string_lossy()
            ));
        }
        for (name, sub) in &t.dirs {
            let sub_dir = dir.join(name);
            let sub_text = mods(sub, endpoints, &sub_dir, out);
            let mod_rs = sub_dir.join("mod.rs");
            text.push_str(&format!("#[path = {:?}]\npub mod {name};\n", mod_rs.to_string_lossy()));
            out.push((mod_rs, sub_text));
        }
        text
    }
    let mut files = Vec::new();
    let mut text = mods(&root, endpoints, dir, &mut files);
    text.push_str("\n/// The routes of `src/api/`.\npub fn router() -> ::yawn::server::axum::Router {\n    ::yawn::server::axum::Router::new()\n");
    for e in endpoints.iter().filter(|e| !e.methods.is_empty()) {
        let handler = |m: &str| format!("self::{}::{m}", e.module.join("::"));
        let mut chain = String::new();
        for (i, m) in e.methods.iter().enumerate() {
            if i == 0 {
                chain.push_str(&format!("::yawn::server::axum::routing::{m}({})", handler(m)));
            } else {
                chain.push_str(&format!(".{m}({})", handler(m)));
            }
        }
        text.push_str(&format!("        .route({:?}, {chain})\n", e.path));
    }
    text.push_str("}\n");
    files.push((dir.join("mod.rs"), text));
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "yawn-api-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        for (name, text) in files {
            let path = root.join("src").join(DIR).join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        root
    }

    #[test]
    fn finds_the_handlers_a_file_defines() {
        let src = "use x;\npub async fn get(a: u8) {}\npub fn post() {}\nasync fn put() {}\npub async fn delete <T>() {}\n    pub fn patch() {}\npub fn getter() {}\n";
        assert_eq!(methods_in(src), ["get", "post", "delete"]);
        assert!(methods_in("fn helper() {}").is_empty());
    }

    #[test]
    fn maps_files_to_axum_routes() {
        let root = tree(&[
            ("index.rs", "pub async fn get() {}"),
            ("health.rs", "pub async fn get() {}\npub async fn post() {}"),
            ("users/index.rs", "pub async fn get() {}"),
            ("users/[id].rs", "pub async fn get() {}"),
            ("files/[...path].rs", "pub async fn get() {}"),
            ("(admin)/stats.rs", "pub async fn get() {}"),
        ]);
        let found = scan(&root).unwrap();
        let paths: Vec<_> = found.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/api",
                "/api/files/{*path}",
                "/api/health",
                "/api/stats",
                "/api/users",
                "/api/users/{id}"
            ]
        );
        let health = found.iter().find(|e| e.path == "/api/health").unwrap();
        assert_eq!(health.methods, ["get", "post"]);
        let files = module_files(Path::new("/out"), &found);
        let get = |p: &str| {
            files.iter().find(|(f, _)| f == Path::new(p)).map(|(_, t)| t.clone()).unwrap_or_else(
                || panic!("no {p} in {:?}", files.iter().map(|f| &f.0).collect::<Vec<_>>()),
            )
        };
        let src = get("/out/mod.rs") + &get("/out/users/mod.rs");
        assert!(src.contains("pub mod users;") && src.contains("pub mod param_id;"), "{src}");
        assert!(src.contains(".route(\"/api/users/{id}\", ::yawn::server::axum::routing::get(self::users::param_id::get))"), "{src}");
        assert!(src.contains("routing::get(self::health::get).post(self::health::post)"), "{src}");
        assert!(src.contains("pub mod group_admin;"), "{src}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn underscore_names_are_modules_without_routes() {
        let root = tree(&[
            ("(auth)/login.rs", "pub async fn get() {}"),
            ("(auth)/_oauth.rs", "pub fn helper() {}"),
            ("(auth)/__callback.rs", "pub async fn get() {}"),
            ("_lib/deep/x.rs", "pub fn y() {}"),
            ("_lib/index.rs", "pub fn z() {}"),
        ]);
        let found = scan(&root).unwrap();
        let routes: Vec<_> =
            found.iter().filter(|e| !e.methods.is_empty()).map(|e| e.path.as_str()).collect();
        assert_eq!(routes, ["/api/__callback", "/api/login"]);
        let files = module_files(Path::new("/out"), &found);
        let all: String = files.iter().map(|(_, t)| t.as_str()).collect();
        assert!(
            all.contains("pub mod _oauth;")
                && all.contains("pub mod _lib;")
                && all.contains("pub mod deep;"),
            "{all}"
        );
        assert_eq!(all.matches(".route(").count(), 2, "{all}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_files_that_cannot_work() {
        let root = tree(&[("empty.rs", "fn helper() {}"), ("9bad.rs", "pub async fn get() {}")]);
        let errors = scan(&root).unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        let _ = fs::remove_dir_all(root);
        let root = tree(&[("a/[id].rs", "pub fn get() {}"), ("a/[name].rs", "pub fn get() {}")]);
        let errors = scan(&root).unwrap_err();
        assert!(errors[0].1.contains("same route"), "{errors:?}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_folder_no_endpoints() {
        let root = std::env::temp_dir().join(format!("yawn-api-none-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        assert_eq!(scan(&root).unwrap(), vec![]);
        let _ = fs::remove_dir_all(root);
    }
}

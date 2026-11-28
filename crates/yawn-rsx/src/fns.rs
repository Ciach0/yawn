//! `src/fns/`: files whose `yawn::server_fn!` blocks the server routes and the client calls over
//! HTTP. The macro is in yawn-web; this finds the functions and writes the router.

use std::fs;
use std::path::{Path, PathBuf};

/// Folder under `src/` that holds the server functions
pub const DIR: &str = "fns";
/// Where the generated `fns` module lives, below the output directory
pub const MODULE_DIR: &str = "_fns";

/// One file of `src/fns/`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    /// Module path below `fns` (`["admin", "users"]`)
    pub module: Vec<String>,
    pub file: PathBuf,
    /// Names of the functions in its `server_fn!` blocks
    pub fns: Vec<String>,
}

/// Functions declared by the top-level `server_fn!` blocks of `source`. A file that doesn't parse
/// gives none; rustc reports it.
pub fn names_in(source: &str) -> Vec<String> {
    let Ok(file) = syn::parse_file(source) else { return Vec::new() };
    let fns = |input: syn::parse::ParseStream| {
        let mut out = Vec::new();
        while !input.is_empty() {
            out.push(input.parse::<syn::ItemFn>()?.sig.ident.to_string());
        }
        Ok(out)
    };
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Macro(m) if m.mac.path.segments.last()?.ident == "server_fn" => {
                syn::parse::Parser::parse2(fns, m.mac.tokens.clone()).ok()
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// Reads `src/fns/`
pub fn scan(root: &Path) -> Result<Vec<Module>, Vec<(String, String)>> {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    walk(root, &root.join("src").join(DIR), &mut Vec::new(), &mut found, &mut errors);
    if errors.is_empty() { Ok(found) } else { Err(errors) }
}

fn walk(
    root: &Path,
    dir: &Path,
    modules: &mut Vec<String>,
    found: &mut Vec<Module>,
    errors: &mut Vec<(String, String)>,
) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let shown = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let stem = if path.is_dir() { Some(name.as_str()) } else { name.strip_suffix(".rs") };
        let Some(stem) = stem else { continue };
        // syn refuses keywords too: `mod.rs` would be `pub mod mod;`
        if syn::parse_str::<syn::Ident>(stem).is_err() {
            errors.push((shown, format!("`{stem}` isn't a Rust module name")));
            continue;
        }
        modules.push(stem.to_string());
        if path.is_dir() {
            if path.with_extension("rs").is_file() {
                errors.push((shown, format!("`{stem}` is both a file and a folder")));
            } else {
                walk(root, &path, modules, found, errors);
            }
        } else {
            match fs::read_to_string(&path) {
                Ok(text) => {
                    found.push(Module { module: modules.clone(), fns: names_in(&text), file: path })
                }
                Err(e) => errors.push((shown, format!("cannot read: {e}"))),
            }
        }
        modules.pop();
    }
}

/// The `mod fns;` lines for the crate root: the module is on both sides, the client calls through it
pub fn mod_decl(dir: &Path) -> String {
    format!("\n#[path = {:?}]\npub mod fns;\n", crate::project::slashed(&dir.join("mod.rs")))
}

/// The `fns` module as real files in `dir`, with the router of every function in the root one
pub fn module_files(dir: &Path, modules: &[Module]) -> Vec<(PathBuf, String)> {
    // don't point these at the hot-fn mirror (`mirror::redirect`, as `api` does): it can't swap a
    // body inside `server_fn!`, and a change to one has to rebuild the server
    let paths: Vec<_> = modules.iter().map(|m| (m.module.as_slice(), m.file.clone())).collect();
    let mut files = crate::api::module_tree(dir, &paths);
    let text = &mut files.last_mut().expect("the root module").1;
    text.push_str("\n/// The routes of the server functions in `src/fns/`.\n#[cfg(not(target_arch = \"wasm32\"))]\npub fn router() -> ::yawn::server::axum::Router {\n    ::yawn::server::axum::Router::new()\n");
    for m in modules {
        let path = m.module.join("::");
        for f in &m.fns {
            // the client's stub builds the same URL from its own `module_path!()`
            text.push_str(&format!(
                "        .route(&::yawn::fns::url(concat!(module_path!(), \"::{path}\"), \"{f}\"), ::yawn::fns::handler(self::{path}::{f}))\n"
            ));
        }
    }
    text.push_str("}\n");
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "yawn-fns-{}-{}",
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
    fn finds_the_functions_of_server_fn_blocks() {
        let src = "use x::Y;\n\
            yawn::server_fn! {\n    /// docs\n    pub async fn add(a: i32, b: i32) -> i32 { a + b }\n    async fn ping() {}\n}\n\
            server_fn! { pub async fn list() -> Vec<Y> { vec![] } }\n\
            pub async fn plain() {}\n\
            mod inner { yawn::server_fn! { pub async fn hidden() {} } }\n";
        assert_eq!(names_in(src), ["add", "ping", "list"]);
        assert!(names_in("fn broken(").is_empty());
    }

    #[test]
    fn maps_files_to_modules_and_routes() {
        let root = tree(&[
            (
                "todos.rs",
                "yawn::server_fn! { pub async fn add(t: String) -> u32 { 1 } pub async fn clear() {} }",
            ),
            ("admin/users.rs", "yawn::server_fn! { pub async fn ban(id: u64) {} }"),
            ("shared.rs", "pub struct Todo;"),
        ]);
        let found = scan(&root).unwrap();
        let mods: Vec<_> = found.iter().map(|m| (m.module.join("::"), m.fns.clone())).collect();
        assert_eq!(
            mods,
            [
                ("admin::users".to_string(), vec!["ban".to_string()]),
                ("shared".to_string(), vec![]),
                ("todos".to_string(), vec!["add".to_string(), "clear".to_string()]),
            ]
        );
        let files = module_files(Path::new("/out"), &found);
        let root_mod = &files.last().unwrap().1;
        assert!(
            root_mod.contains("pub mod todos;") && root_mod.contains("pub mod admin;"),
            "{root_mod}"
        );
        assert!(root_mod.contains("::yawn::fns::url(concat!(module_path!(), \"::admin::users\"), \"ban\"), ::yawn::fns::handler(self::admin::users::ban))"), "{root_mod}");
        assert_eq!(root_mod.matches(".route(").count(), 3, "{root_mod}");
        assert!(
            files
                .iter()
                .any(|(p, t)| p == Path::new("/out/admin/mod.rs") && t.contains("pub mod users;"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_names_that_cannot_be_modules() {
        let root =
            tree(&[("mod.rs", ""), ("9x.rs", ""), ("a.rs", ""), ("a/b.rs", ""), ("notes.txt", "")]);
        let mut errors: Vec<_> = scan(&root).unwrap_err().into_iter().map(|e| e.0).collect();
        errors.sort();
        assert_eq!(errors, ["src/fns/9x.rs", "src/fns/a", "src/fns/mod.rs"]);
        let _ = fs::remove_dir_all(root);
    }
}

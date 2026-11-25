use std::path::{Path, PathBuf};
use yawn_rsx::SourceMap;
use yawn_rsx::project::{self, FileDiagnostic, Report};

/// Compiles the project's `.rsx` files
pub fn compile(root: &Path) -> Result<Report, Vec<FileDiagnostic>> {
    project::compile_project(root)
}

/// [`compile`] for a release build: the project stays one crate, which comes out a little smaller
/// than the split one
pub fn compile_whole(root: &Path) -> Result<Report, Vec<FileDiagnostic>> {
    project::compile_project_with(root, false)
}

/// One diagnostic as the dev session reports it
pub struct Located {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
    /// Multi-line rendering with the offending source line, for the browser overlay
    pub rendered: String,
}

pub fn locate(d: &FileDiagnostic) -> Located {
    let (line, col) = if d.source.is_empty() { (1, 1) } else { d.diagnostic.position(&d.source) };
    let rendered = if d.source.is_empty() {
        format!("error: {}\n  --> {}", d.diagnostic.message, d.path)
    } else {
        d.render()
    };
    Located {
        file: d.path.clone(),
        line: line as u32,
        col: col as u32,
        message: d.diagnostic.message.clone(),
        rendered,
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Maps a location in generated code (`.yawn/rsx/**.rs`, `.yawn/client.rs`) to the `.rsx` file
pub fn remap(root: &Path, file: &str, line: u32, col: u32) -> Option<(String, u32, u32)> {
    let path = Path::new(file);
    let base = if root.join(yawn_rsx::manifest::MANIFEST).is_file() {
        root.join(".yawn")
    } else {
        root.to_path_buf()
    };
    let abs = normalize(&if path.is_absolute() { path.to_path_buf() } else { base.join(path) });
    let rel = abs.strip_prefix(root).ok()?.to_str()?.replace('\\', "/");
    // the client crate root is generated from `main.rsx`
    let stem = if rel == project::CLIENT_ROOT {
        "main"
    } else {
        rel.strip_prefix(project::OUT_DIR)?.strip_prefix('/')?.strip_suffix(".rs")?
    };
    let map_text =
        std::fs::read_to_string(root.join(project::OUT_DIR).join(format!("{stem}.map"))).ok()?;
    let map = SourceMap::parse(&map_text)?;
    let (l, c) = map.map(line, col)?;
    Some((map.source, l, c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yawn-cli-rsx-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("src")).unwrap();
        fs::write(d.join("Cargo.toml"), "[package]\nname = \"t\"\nversion = \"0.1.0\"\n").unwrap();
        d
    }

    #[test]
    fn remaps_generated_locations_to_the_rsx_file() {
        let root = temp("remap");
        fs::write(
            root.join("src/a.rsx"),
            "component A() {\n    let n = signal(1);\n    view { <p>{n}</p> }\n}\n",
        )
        .unwrap();
        compile(&root).unwrap();
        let r#gen = fs::read_to_string(root.join(".yawn/rsx/a.rs")).unwrap();
        let (gen_line, gen_col) = r#gen
            .lines()
            .enumerate()
            .find_map(|(i, l)| l.find("let n = signal(1)").map(|c| (i as u32 + 1, c as u32 + 1)))
            .unwrap();
        // rustc reports both relative and absolute file names
        for file in
            [".yawn/rsx/a.rs".to_string(), root.join(".yawn/rsx/a.rs").display().to_string()]
        {
            assert_eq!(remap(&root, &file, gen_line, gen_col), Some(("src/a.rsx".into(), 2, 5)));
        }
        assert_eq!(remap(&root, "src/lib.rs", 3, 3), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn main_rsx_is_the_crate_root_of_the_client() {
        let root = temp("entry");
        fs::write(
            root.join("src/main.rsx"),
            "use crate::card::Card;\n\ncomponent App() {\n    view { <Card /> }\n}\n",
        )
        .unwrap();
        fs::write(root.join("src/card.rsx"), "component Card() { view { <b /> } }\n").unwrap();
        compile(&root).unwrap();
        let client = fs::read_to_string(root.join(project::CLIENT_ROOT)).unwrap();
        assert!(
            client.contains("pub struct App")
                && client.contains("include!(\"rsx/modules.rs\");")
                && client.contains("yawn::client!(App);"),
            "{client}"
        );
        let server = fs::read_to_string(root.join(project::SERVER_ROOT)).unwrap();
        assert!(server.contains("fn main()") && server.contains("CARGO_PKG_NAME"), "{server}");
        let modules = fs::read_to_string(root.join(".yawn/rsx/modules.rs")).unwrap();
        assert!(modules.contains("mod card;") && !modules.contains("mod main;"), "{modules}");
        // rustc errors in the root map back to main.rsx
        let line = client.lines().position(|l| l.contains("pub struct App")).unwrap() as u32 + 1;
        // rustc names the file relative to the generated manifest
        for file in ["client.rs".to_string(), root.join(project::CLIENT_ROOT).display().to_string()]
        {
            assert!(remap(&root, &file, line, 1).is_some_and(|r| r.0 == "src/main.rsx"), "{file}");
        }
        assert!(fs::read_to_string(root.join(".yawn/Cargo.toml")).unwrap().contains("client.rs"));
        assert!(!root.join("src/lib.rs").exists());
        // going away removes the generated roots
        fs::remove_file(root.join("src/main.rsx")).unwrap();
        compile(&root).unwrap();
        assert!(
            !root.join(project::CLIENT_ROOT).exists()
                && !root.join(project::SERVER_ROOT).exists()
                && !root.join(".yawn/Cargo.toml").exists()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn src_routes_becomes_a_route_table_unless_switched_off() {
        let root = temp("routes");
        fs::create_dir_all(root.join("src/routes/users")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"t\"\nversion = \"0.1.0\"\n[dependencies]\nyawn-web = \"0.1\"\n",
        )
        .unwrap();
        fs::write(root.join("src/main.rsx"), "component App() { view { <b /> } }\n").unwrap();
        fs::write(root.join("src/routes/layout.rsx"), "component Layout() { view { <main /> } }\n")
            .unwrap();
        fs::write(
            root.join("src/routes/index.rsx"),
            "component Index() { view { <p>\"home\"</p> } }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/routes/users/[id].rsx"),
            "component Id() { view { <p>\"user\"</p> } }\n",
        )
        .unwrap();
        compile(&root).unwrap();
        let modules = fs::read_to_string(root.join(".yawn/rsx/modules.rs")).unwrap();
        assert!(modules.contains("pub mod routes;"), "{modules}");
        let table = fs::read_to_string(root.join(".yawn/rsx/routes/mod.rs")).unwrap();
        assert!(table.contains("pub fn table()"), "{table}");
        let users = fs::read_to_string(root.join(".yawn/rsx/routes/users/mod.rs")).unwrap();
        assert!(users.contains("pub mod param_id;"), "{users}");
        assert!(
            table.contains(".at::<crate::routes::users::param_id::Id>(\"/users/:id\")"),
            "{table}"
        );
        assert!(root.join(".yawn/rsx/routes/users/param_id.rs").exists());
        let manifest = fs::read_to_string(root.join(".yawn/Cargo.toml")).unwrap();
        assert!(manifest.contains("yawn-router = \"0.1\""), "{manifest}");
        // errors in a route file map back to it
        let r#gen = fs::read_to_string(root.join(".yawn/rsx/routes/users/param_id.rs")).unwrap();
        let line = r#gen.lines().position(|l| l.contains("pub struct Id")).unwrap() as u32 + 1;
        assert!(
            remap(&root, "rsx/routes/users/param_id.rs", line, 1)
                .is_some_and(|r| r.0 == "src/routes/users/[id].rsx")
        );
        // switched off: ordinary modules, and `[id]` isn't a module name
        fs::write(root.join("Yawn.toml"), "[router]\nfolders = false\n").unwrap();
        let errs = compile(&root).unwrap_err();
        assert!(
            errs.iter().any(|e| e.diagnostic.message.contains("not a valid Rust module name")),
            "{:?}",
            errs.iter().map(|e| &e.diagnostic.message).collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn hand_written_crate_roots_are_never_overwritten() {
        let root = temp("handwritten");
        fs::write(root.join("src/main.rsx"), "component App() { view { <b /> } }\n").unwrap();
        fs::write(root.join("src/lib.rs"), "// mine\n").unwrap();
        let errs = compile(&root).unwrap_err();
        assert!(
            errs[0].diagnostic.message.contains("src/lib.rs"),
            "{}",
            errs[0].diagnostic.message
        );
        assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), "// mine\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn main_rsx_needs_an_app_component() {
        let root = temp("noapp");
        fs::write(root.join("src/main.rsx"), "component Other() { view { <b /> } }\n").unwrap();
        let errs = compile(&root).unwrap_err();
        assert!(
            errs[0].diagnostic.message.contains("component App()"),
            "{}",
            errs[0].diagnostic.message
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn compile_errors_name_the_file_line_and_column() {
        let root = temp("errors");
        fs::write(
            root.join("src/b.rsx"),
            "component B() {\n    view {\n        <p>oops</p>\n    }\n}\n",
        )
        .unwrap();
        let errs = compile(&root).unwrap_err();
        let l = locate(&errs[0]);
        assert_eq!((l.file.as_str(), l.line, l.col), ("src/b.rsx", 3, 12));
        assert!(l.rendered.contains("<p>oops</p>"), "{}", l.rendered);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unchanged_output_is_not_rewritten() {
        let root = temp("stable");
        fs::write(root.join("src/c.rsx"), "component C() { view { <b /> } }\n").unwrap();
        assert!(compile(&root).unwrap().written > 0);
        assert_eq!(
            compile(&root).unwrap().written,
            0,
            "second run must leave files alone so cargo stays fresh"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn removed_rsx_files_lose_their_generated_module() {
        let root = temp("stale");
        fs::write(root.join("src/d.rsx"), "component D() { view { <b /> } }\n").unwrap();
        fs::write(root.join("src/e.rsx"), "component E() { view { <b /> } }\n").unwrap();
        compile(&root).unwrap();
        fs::remove_file(root.join("src/e.rsx")).unwrap();
        compile(&root).unwrap();
        assert!(!root.join(".yawn/rsx/e.rs").exists());
        let modules = fs::read_to_string(root.join(".yawn/rsx/modules.rs")).unwrap();
        assert!(modules.contains("mod d;") && !modules.contains("mod e;"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn module_conflicts_are_reported() {
        let root = temp("conflict");
        fs::write(root.join("src/f.rsx"), "component F() { view { <b /> } }\n").unwrap();
        fs::write(root.join("src/f.rs"), "").unwrap();
        let errs = compile(&root).unwrap_err();
        assert!(
            errs[0].diagnostic.message.contains("already exists"),
            "{}",
            errs[0].diagnostic.message
        );
        let _ = fs::remove_dir_all(&root);
    }
}

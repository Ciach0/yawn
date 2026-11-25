use std::fmt::Write as _;
use std::{env, fs, path::PathBuf};

fn main() {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("fixtures");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=fixtures");
    let mut modules = String::new();
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rsx"))
        .collect();
    names.sort();
    let mut known = yawn_rsx::codegen::Registry::new();
    for path in &names {
        for h in yawn_rsx::parser::headers(&fs::read_to_string(path).unwrap()) {
            if !h.defaults.is_empty() {
                known.insert(h.name, h.defaults);
            }
        }
    }
    for path in names {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let source = fs::read_to_string(&path).unwrap();
        match yawn_rsx::compile_with(&source, &format!("fixtures/{name}.rsx"), &known) {
            Ok(c) => {
                let target = out.join(format!("{name}.rs"));
                fs::write(&target, c.rust).unwrap();
                writeln!(modules, "#[path = {:?}]\npub mod {name};", target.to_string_lossy())
                    .unwrap();
            }
            Err(d) => panic!("\n{}", d.render(&format!("fixtures/{name}.rsx"), &source)),
        }
    }
    fs::write(out.join("modules.rs"), modules).unwrap();
}

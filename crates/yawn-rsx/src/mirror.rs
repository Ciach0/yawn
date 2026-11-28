//! The mirror of the project's plain modules that a dev build compiles instead of `src/`, with the
//! eligible functions made replaceable (see [`crate::hotfn`]).
//!
//! A module and everything under it is mirrored as a whole, or not at all when one of its files
//! can't be (see [`crate::hotfn::transform`]).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where the mirror lives, relative to the generated code
pub const DIR: &str = "_src";

struct State {
    root: PathBuf,
    on: bool,
    trees: BTreeMap<PathBuf, Option<Vec<(PathBuf, String)>>>,
}

fn state() -> &'static Mutex<BTreeMap<PathBuf, State>> {
    static STATE: Mutex<BTreeMap<PathBuf, State>> = Mutex::new(BTreeMap::new());
    &STATE
}

type Cache = BTreeMap<PathBuf, (u64, Option<String>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: Mutex<Cache> = Mutex::new(BTreeMap::new());
    &CACHE
}

/// Whether the compile in progress for `root` makes a mirror
pub fn active(root: &Path) -> bool {
    state().lock().is_ok_and(|s| s.get(root).is_some_and(|s| s.on))
}

/// Generated code, with what a mirror does to the statics of plain modules
pub fn generated(root: &Path, text: String, unit: &str) -> String {
    if !active(root) {
        return text;
    }
    let key = hash(&text);
    if let Ok(c) = cache().lock()
        && let Some((k, Some(out))) = c.get(Path::new(unit))
        && *k == key
    {
        return out.clone();
    }
    let dir = Path::new(unit)
        .strip_prefix("rsx")
        .ok()
        .and_then(Path::parent)
        .map(|d| root.join("src").join(d))
        .unwrap_or_else(|| root.join("src"));
    let out = crate::hotfn::transform(&text, &dir, unit)
        .or_else(|| crate::hotfn::statics_only(&text, unit))
        .unwrap_or(text);
    if let Ok(mut c) = cache().lock() {
        c.insert(PathBuf::from(unit), (key, Some(out.clone())));
    }
    out
}

pub fn enabled(root: &Path) -> bool {
    fs::read_to_string(root.join("Yawn.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("dev")?.get("hot_fns")?.as_bool())
        .unwrap_or(true)
}

/// Starts a compile of the project: which modules get mirrored is decided again
pub fn begin(root: &Path, on: bool) {
    let all = fs::read_to_string(root.join("Yawn.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("dev")?.get("hot_async")?.as_bool())
        .unwrap_or(true);
    crate::hotfn::ASYNC_ALL.store(all, std::sync::atomic::Ordering::Relaxed);
    let hot_pub = fs::read_to_string(root.join("Yawn.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("dev")?.get("hot_pub")?.as_bool())
        .unwrap_or(true);
    crate::hotfn::HOT_PUB.store(hot_pub, std::sync::atomic::Ordering::Relaxed);
    let lines = fs::read_to_string(root.join("Yawn.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("dev")?.get("split_lines")?.as_integer())
        .map_or(2000, |n| n.max(0) as usize);
    crate::split_nodes::MAX_LINES.store(lines, std::sync::atomic::Ordering::Relaxed);
    if on {
        crate::hotfn::index_words(&root.join("src"));
    }
    if let Ok(mut s) = state().lock() {
        s.insert(
            root.to_path_buf(),
            State { root: root.to_path_buf(), on, trees: BTreeMap::new() },
        );
    }
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn mirrored(root: &Path, file: &Path) -> Option<PathBuf> {
    let rel = file.strip_prefix(root.join("src")).ok()?;
    Some(root.join(crate::project::OUT_DIR).join(DIR).join(rel))
}

fn hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(text, &mut h);
    std::hash::Hasher::finish(&h)
}

fn transformed(root: &Path, file: &Path) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    let key = hash(&text);
    if let Ok(c) = cache().lock()
        && let Some((k, out)) = c.get(file)
        && *k == key
    {
        return out.clone();
    }
    let unit = file.strip_prefix(root.join("src")).ok()?.to_string_lossy().into_owned();
    let out = crate::hotfn::transform(&text, file.parent()?, &unit);
    if let Ok(mut c) = cache().lock() {
        c.insert(file.to_path_buf(), (key, out.clone()));
    }
    out
}

/// The file the generated code should name for the source file `file`: its mirror, or itself
pub fn redirect(file: &Path) -> PathBuf {
    let Ok(mut guard) = state().lock() else { return file.to_path_buf() };
    let Some(st) = guard.values_mut().filter(|s| s.on).find(|s| file.starts_with(&s.root)) else {
        return file.to_path_buf();
    };
    let root = st.root.clone();
    if file.strip_prefix(root.join("src")).is_err() {
        return file.to_path_buf();
    }
    let plain_mod = file.file_name().is_some_and(|n| n == "mod.rs");
    let (key, dir) = if plain_mod {
        let parent = file.parent().unwrap_or(file).to_path_buf();
        (parent.clone(), parent)
    } else {
        (file.to_path_buf(), file.with_extension(""))
    };
    if !st.trees.contains_key(&key) {
        let mut files = Vec::new();
        if !plain_mod {
            files.push(file.to_path_buf());
        }
        rs_files(&dir, &mut files);
        let mut done = Vec::new();
        let mut ok = true;
        for f in files {
            match transformed(&root, &f) {
                Some(text) => done.push((f, text)),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        st.trees.insert(key.clone(), ok.then_some(done));
    }
    match st.trees.get(&key) {
        Some(Some(_)) => mirrored(&root, file).unwrap_or_else(|| file.to_path_buf()),
        _ => file.to_path_buf(),
    }
}

/// Writes every mirror decided on since [`begin`]; the files it wrote go into `keep`
pub fn write_all(root: &Path, keep: &mut Vec<PathBuf>) -> std::io::Result<usize> {
    let Ok(guard) = state().lock() else { return Ok(0) };
    let Some(st) = guard.get(root).filter(|s| s.on) else { return Ok(0) };
    let mut written = 0;
    for files in st.trees.values().flatten() {
        for (file, text) in files {
            let Some(to) = mirrored(&st.root, file) else { continue };
            if let Some(dir) = to.parent() {
                fs::create_dir_all(dir)?;
            }
            if fs::read_to_string(&to).ok().as_deref() != Some(text.as_str()) {
                fs::write(&to, text)?;
                written += 1;
            }
            keep.push(to);
        }
    }
    Ok(written)
}

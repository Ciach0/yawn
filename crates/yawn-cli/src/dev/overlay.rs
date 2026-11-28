//! Public functions whose signature changed. A crate that calls one is compiled against the metadata
//! of the crate that has it, and so is every crate between the two, so the dev build rebuilds those
//! like cargo does (their code is the same) and the two ends as the modules the page and the
//! server load. The metadata lives in an overlay directory that replaces the files of the last full
//! build until the next one.

use super::cargo::{KillGroup, Parsed};
use super::dynamic::SideError;
use super::replay::{self, Recorded};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct Overlay {
    dir: PathBuf,
    sigs: Mutex<BTreeSet<String>>,
}

impl Overlay {
    pub fn new(dir: PathBuf) -> Arc<Overlay> {
        Arc::new(Overlay { dir, sigs: Mutex::new(BTreeSet::new()) })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn active(&self) -> bool {
        !self.sigs.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    pub fn add(&self, names: impl IntoIterator<Item = String>) {
        self.sigs.lock().unwrap_or_else(|e| e.into_inner()).extend(names);
    }

    pub fn sigs(&self) -> BTreeSet<String> {
        self.sigs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn clear(&self) {
        self.sigs.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn crate_name(package: &str) -> String {
    package.replace('-', "_")
}

pub fn prefix(app: &str) -> String {
    yawn_rsx::manifest::member_package(app, "").replace('-', "_")
}

fn externs<'a>(argv: &'a [String], prefix: &'a str) -> impl Iterator<Item = String> + 'a {
    argv.iter().enumerate().filter_map(move |(i, a)| {
        if i == 0 || argv[i - 1] != "--extern" {
            return None;
        }
        let name = a.split('=').next()?;
        name.starts_with(prefix).then(|| name.to_string())
    })
}

/// The recorded commands of `roots` and of every crate of the project they depend on, or the names
/// of the crates nothing was recorded of (cargo never ran rustc for them: they were fresh)
pub fn collect(
    prefix: &str,
    roots: &[String],
    rec: &mut dyn FnMut(&str) -> Option<Recorded>,
) -> Result<BTreeMap<String, Recorded>, Vec<String>> {
    let mut out = BTreeMap::new();
    let mut missing = BTreeSet::new();
    let mut queue: Vec<String> = roots.to_vec();
    while let Some(name) = queue.pop() {
        if out.contains_key(&name) || missing.contains(&name) {
            continue;
        }
        let Some(r) = rec(&name) else {
            missing.insert(name);
            continue;
        };
        queue.extend(externs(&r.argv, prefix));
        out.insert(name, r);
    }
    if missing.is_empty() { Ok(out) } else { Err(missing.into_iter().collect()) }
}

fn deps_of(recs: &BTreeMap<String, Recorded>, prefix: &str) -> BTreeMap<String, BTreeSet<String>> {
    recs.iter()
        .map(|(n, r)| {
            (n.clone(), externs(&r.argv, prefix).filter(|d| recs.contains_key(d)).collect())
        })
        .collect()
}

fn level(
    deps: &BTreeMap<String, BTreeSet<String>>,
    name: &str,
    memo: &mut BTreeMap<String, usize>,
) -> usize {
    if let Some(l) = memo.get(name) {
        return *l;
    }
    let l =
        deps.get(name).into_iter().flatten().map(|d| level(deps, d, memo) + 1).max().unwrap_or(0);
    memo.insert(name.to_string(), l);
    l
}

/// `names` with the crates they depend on first
pub fn sorted(prefix: &str, recs: &BTreeMap<String, Recorded>, names: &[String]) -> Vec<String> {
    let deps = deps_of(recs, prefix);
    let mut memo = BTreeMap::new();
    let mut out = names.to_vec();
    out.sort_by_key(|n| (level(&deps, n, &mut memo), n.clone()));
    out
}

const SUFFIX: &str = "ov";

/// A crate of the overlay is a second version of itself: its own identity, so it can sit next to
/// the one the crates between it and its callers were built against
fn reidentify(argv: &mut [String]) {
    let mut i = 0;
    while i < argv.len() {
        let hit = |a: &str, key: &str| a.strip_prefix(key).map(str::to_string);
        if argv[i] == "-C" && i + 1 < argv.len() {
            if let Some(v) = hit(&argv[i + 1], "metadata=") {
                argv[i + 1] = format!("metadata={v}{SUFFIX}");
            } else if let Some(v) = hit(&argv[i + 1], "extra-filename=") {
                argv[i + 1] = format!("extra-filename={v}{SUFFIX}");
            }
        } else if let Some(v) = hit(&argv[i], "-Cmetadata=") {
            argv[i] = format!("-Cmetadata={v}{SUFFIX}");
        } else if let Some(v) = hit(&argv[i], "-Cextra-filename=") {
            argv[i] = format!("-Cextra-filename={v}{SUFFIX}");
        }
        i += 1;
    }
}

/// `argv` with the metadata of the crates of the overlay in place of the files it was recorded with
fn overlaid(
    mut argv: Vec<String>,
    overlay: &Path,
    sigs: &BTreeSet<String>,
    own: bool,
) -> Vec<String> {
    if own {
        reidentify(&mut argv);
    }
    let mut out = Vec::with_capacity(argv.len() + 2);
    let mut i = 0;
    while i < argv.len() {
        if argv[i] == "--extern"
            && let Some(spec) = argv.get(i + 1)
            && let Some((name, path)) = spec.split_once('=')
            && sigs.contains(name)
        {
            let p = Path::new(path);
            let stem = p.file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let file = overlay.join(format!("{stem}{SUFFIX}.rmeta"));
            out.push("--extern".to_string());
            out.push(format!("{name}={}", file.display()));
            i += 2;
            continue;
        }
        out.push(argv[i].clone());
        i += 1;
    }
    out.extend(["-L".into(), format!("dependency={}", overlay.display())]);
    out
}

pub struct Build<'a> {
    pub prefix: &'a str,
    pub overlay: &'a Overlay,
    pub recs: &'a BTreeMap<String, Recorded>,
    /// How a crate is compiled as `(recorded command, output directory, incremental directory, emit)`
    pub argv: &'a (dyn Fn(&Recorded, &Path, &Path, &str) -> Vec<String> + Sync),
    pub incr: &'a (dyn Fn(&str) -> PathBuf + Sync),
}

/// Compiles `full` (as libraries) and every crate between them and the crates of the overlay (as
/// metadata). The libraries of `full` come back in the order they must be loaded in
pub fn run(
    b: &Build,
    full: &[String],
    kill: &KillGroup,
    on: &mut dyn FnMut(&Parsed),
) -> Result<Vec<(String, PathBuf, bool)>, SideError> {
    let fallback = |why: &str| SideError::Fallback(why.to_string());
    let deps = deps_of(b.recs, b.prefix);
    let sigs = b.overlay.sigs();
    let sigs = &sigs;
    let needed: BTreeSet<String> = full.iter().cloned().collect();
    let mut memo = BTreeMap::new();
    let mut levels: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for n in &needed {
        levels.entry(level(&deps, n, &mut memo)).or_default().push(n.clone());
    }
    std::fs::create_dir_all(b.overlay.dir()).map_err(|e| fallback(&e.to_string()))?;
    let mut failed = false;
    for names in levels.values() {
        let lt = std::time::Instant::now();
        let ran: Vec<(String, std::io::Result<bool>, Vec<Parsed>)> = std::thread::scope(|scope| {
            let jobs: Vec<_> = names
                .iter()
                .map(|n| {
                    scope.spawn(move || {
                        let rec = &b.recs[n];
                        let incr = (b.incr)(n);
                        let argv = overlaid(
                            (b.argv)(rec, b.overlay.dir(), &incr, "--emit=metadata,link"),
                            b.overlay.dir(),
                            sigs,
                            sigs.contains(n),
                        );
                        let mut seen = Vec::new();
                        let ok = replay::run_recorded(&argv, &rec.env, &rec.cwd, kill, &mut |p| {
                            seen.push(p.clone())
                        });
                        (n.clone(), ok, seen)
                    })
                })
                .collect();
            jobs.into_iter().filter_map(|j| j.join().ok()).collect()
        });
        if std::env::var_os("YAWN_PROFILE_LEVELS").is_some() {
            eprintln!("LEVEL {:?} {:?}", lt.elapsed(), names);
        }
        for (name, ok, seen) in ran {
            for p in &seen {
                on(p);
            }
            match ok {
                Ok(true) => {}
                Ok(false) => failed = true,
                Err(e) => return Err(fallback(&format!("{name}: {e}"))),
            }
        }
        if failed {
            return Err(SideError::Failed);
        }
    }
    let mut out = Vec::new();
    let all: Vec<String> = needed.iter().cloned().collect();
    for name in sorted(b.prefix, b.recs, &all) {
        let wanted = format!("lib{name}-");
        let rlib = std::fs::read_dir(b.overlay.dir())
            .ok()
            .and_then(|rd| {
                rd.flatten().map(|e| e.path()).find(|p| {
                    p.extension().is_some_and(|x| x == "rlib")
                        && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&wanted))
                })
            })
            .ok_or_else(|| fallback(&format!("no library of {name}")))?;
        let is_full = full.contains(&name);
        out.push((name, rlib, is_full));
    }
    Ok(out)
}

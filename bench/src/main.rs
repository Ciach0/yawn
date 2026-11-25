//! Yawn benchmarks. `run`: the reactivity of Yawn against other libraries on the same scenarios,
//! no rendering and no DOM. `compare`: tables from two sets of result files.
//!
//! Every scenario is written once against the `Engine` trait and runs the same deterministic
//! operation sequence on each library. It is first *verified*: after each operation all libraries
//! must report the same observable values. Timing covers the whole operation including the work
//! needed to finish it: Yawn and Sycamore settle inside `set`, and Dioxus needs
//! `VirtualDom::process_events` to run its memo tasks and effects.
//!
//! The counting allocator below is the only `unsafe` here.
mod compare;
mod engine;
mod engines;
mod harness;
mod scenarios;

use engines::{dioxus::Dioxus, sycamore::Sycamore, yawn::Yawn};
use harness::{ALLOCS, LIVE, Mode, Res};
use scenarios::{MEMORY_KINDS, SCENARIOS};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::Ordering::Relaxed;

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Relaxed);
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Relaxed);
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        LIVE.fetch_add(n as isize - l.size() as isize, Relaxed);
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const ENGINES: [&str; 3] = ["yawn", "dioxus", "sycamore"];

fn run_scenario(engine: &str, name: &str, mode: &Mode) -> Res {
    match engine {
        "yawn" => scenarios::run::<Yawn>(name, mode),
        "dioxus" => scenarios::run::<Dioxus>(name, mode),
        "sycamore" => scenarios::run::<Sycamore>(name, mode),
        _ => unreachable!(),
    }
}

fn run_memory(engine: &str, kind: &'static str, n: usize) -> (f64, f64) {
    match engine {
        "yawn" => scenarios::memory::<Yawn>(kind, n),
        "dioxus" => scenarios::memory::<Dioxus>(kind, n),
        "sycamore" => scenarios::memory::<Sycamore>(kind, n),
        _ => unreachable!(),
    }
}

const USAGE: &str = "usage:
  yawn-bench run [filter] [--engines yawn,dioxus,sycamore] [--verify] [out.csv]
  yawn-bench compare before.csv[,more.csv] after.csv[,more.csv]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("compare") => compare::run(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}

fn run(args: &[String]) {
    let mut engines: Vec<&str> = ENGINES.to_vec();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--engines" {
            let list = it.next().map(String::as_str).unwrap_or("");
            engines = list.split(',').filter(|e| !e.is_empty()).collect();
            for e in &engines {
                if !ENGINES.contains(e) {
                    eprintln!("unknown engine `{e}`; one of {}", ENGINES.join(", "));
                    std::process::exit(1);
                }
            }
        } else {
            rest.push(a);
        }
    }
    let filter = rest.iter().find(|a| !a.ends_with(".csv") && !a.starts_with("--")).cloned();
    let csv_path = rest.iter().find(|a| a.ends_with(".csv")).cloned();
    let verify_only = rest.iter().any(|a| *a == "--verify");

    let mut header = vec!["scenario".to_string(), "unit".to_string()];
    for what in ["ns", "memo_evals", "effect_runs", "allocs"] {
        header.extend(engines.iter().map(|e| format!("{e}_{what}")));
    }
    let mut rows = vec![header.join(",")];
    let mut mismatches = 0;
    for sc in &SCENARIOS {
        if filter.as_ref().is_some_and(|f| {
            if SCENARIOS.iter().any(|s| s.name == f.as_str()) {
                sc.name != f.as_str()
            } else {
                !sc.name.contains(f.as_str())
            }
        }) {
            continue;
        }
        let verified: Vec<Res> =
            engines.iter().map(|e| run_scenario(e, sc.name, &Mode::Verify)).collect();
        let checks: Vec<u64> = verified.iter().map(|r| r.check).collect();
        let ok = checks.windows(2).all(|w| w[0] == w[1]);
        if !ok {
            mismatches += 1;
        }
        let verdict = if ok { "same observable results" } else { "MISMATCH" };
        if verify_only {
            println!("{:<26} verify: {verdict}", sc.name);
            continue;
        }
        let res: Vec<Res> = engines.iter().map(|e| run_scenario(e, sc.name, &Mode::Time)).collect();
        let times: Vec<String> =
            engines.iter().zip(&res).map(|(e, r)| format!("{e} {:>12.1}", r.ns_per_op)).collect();
        println!("{:<26} {} {} | {verdict}", sc.name, sc.unit, times.join(" | "));
        let mut row = vec![sc.name.to_string(), sc.unit.to_string()];
        for get in [
            |r: &Res| r.ns_per_op,
            |r: &Res| r.memo_evals,
            |r: &Res| r.effect_runs,
            |r: &Res| r.allocs,
        ] {
            row.extend(res.iter().map(|r| format!("{:.2}", get(r))));
        }
        rows.push(row.join(","));
    }

    if !verify_only && filter.is_none() {
        println!("\nmemory and creation cost (100 000 nodes, settled):");
        let mut header = vec!["kind".to_string()];
        header.extend(engines.iter().map(|e| format!("{e}_bytes_per_node")));
        header.extend(engines.iter().map(|e| format!("{e}_create_ns")));
        let mut mem_rows = vec![header.join(",")];
        for kind in MEMORY_KINDS {
            let m: Vec<(f64, f64)> = engines.iter().map(|e| run_memory(e, kind, 100_000)).collect();
            let shown: Vec<String> = engines
                .iter()
                .zip(&m)
                .map(|(e, (b, ns))| format!("{e} {b:>8.1} B/node {ns:>9.0} ns"))
                .collect();
            println!("{kind:<14} {}", shown.join(" | "));
            let mut row = vec![kind.to_string()];
            row.extend(m.iter().map(|(b, _)| format!("{b:.1}")));
            row.extend(m.iter().map(|(_, ns)| format!("{ns:.0}")));
            mem_rows.push(row.join(","));
        }
        if let Some(p) = &csv_path {
            let mem_path = p.replace(".csv", "_memory.csv");
            std::fs::write(&mem_path, mem_rows.join("\n") + "\n").expect("write memory csv");
            println!("wrote {mem_path}");
        }
    }
    if let Some(p) = &csv_path {
        if !verify_only {
            std::fs::write(p, rows.join("\n") + "\n").expect("write csv");
            println!("wrote {p}");
        }
    }
    if mismatches > 0 {
        eprintln!("{mismatches} scenario(s) disagree between libraries");
        std::process::exit(2);
    }
}

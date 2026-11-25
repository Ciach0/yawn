use crate::engine::Engine;
use crate::harness::*;
use std::cell::Cell;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

pub fn chain<E: Engine>(n: usize, cut: bool, mode: &Mode) -> Res {
    let seen = cells(1);
    let s2 = seen.clone();
    let (mut run, root) = E::start(move |b| {
        let root = E::signal(b, 0);
        let mut prev = E::memo(b, move |rd| {
            bump_memo();
            let v = E::sig(rd, root);
            if cut { v & 0 } else { v + 1 }
        });
        for _ in 0..n {
            let p = prev;
            prev = E::memo(b, move |rd| {
                bump_memo();
                E::mem(rd, p) + 1
            });
        }
        let last = prev;
        E::effect(b, move |rd| {
            bump_eff();
            s2[0].set(E::mem(rd, last));
        });
        root
    });
    let mut op = |i: u64| E::set(&mut run, root, i + 1);
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

pub fn fanout<E: Engine>(n: usize, mode: &Mode) -> Res {
    let seen = cells(n);
    let s2 = seen.clone();
    let (mut run, s) = E::start(move |b| {
        let s = E::signal(b, 0);
        let parity = E::memo(b, move |rd| {
            bump_memo();
            E::sig(rd, s) & 1
        });
        for i in 0..n {
            let c = s2.clone();
            E::effect(b, move |rd| {
                bump_eff();
                c[i].set(E::mem(rd, parity));
            });
        }
        s
    });
    let mut op = |i: u64| E::set(&mut run, s, i + 1);
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

pub fn effects_on_signal<E: Engine>(n: usize, mode: &Mode) -> Res {
    let seen = cells(n);
    let s2 = seen.clone();
    let (mut run, s) = E::start(move |b| {
        let s = E::signal(b, 0);
        for i in 0..n {
            let c = s2.clone();
            E::effect(b, move |rd| {
                bump_eff();
                c[i].set(E::sig(rd, s));
            });
        }
        s
    });
    let mut op = |i: u64| E::set(&mut run, s, i + 1);
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

pub fn write_unobserved<E: Engine>(mode: &Mode) -> Res {
    let (mut run, s) = E::start(|b| E::signal(b, 0));
    let mut op = |i: u64| E::set(&mut run, s, i + 1);
    drive(mode, &mut op, &mut || 0)
}

pub fn read_memo<E: Engine>(mode: &Mode) -> Res {
    let (mut run, m) = E::start(|b| {
        let s = E::signal(b, 1);
        E::memo(b, move |rd| E::sig(rd, s) + 1)
    });
    let acc = Cell::new(0u64);
    let mut op = |_| acc.set(acc.get().wrapping_add(E::read_mem(&mut run, m)));
    drive(mode, &mut op, &mut || acc.get())
}

pub fn read_signal<E: Engine>(mode: &Mode) -> Res {
    let (mut run, s) = E::start(|b| E::signal(b, 1));
    let acc = Cell::new(0u64);
    let mut op = |_| acc.set(acc.get().wrapping_add(E::read_sig(&mut run, s)));
    drive(mode, &mut op, &mut || acc.get())
}

pub fn tree<E: Engine>(n: usize, mode: &Mode) -> Res {
    let seen = cells(1);
    let s2 = seen.clone();
    let (mut run, leaves) = E::start(move |b| {
        let leaves: Vec<E::Sig> = (0..n as u64).map(|i| E::signal(b, i)).collect();
        let mut cur: Vec<E::Memo> = leaves
            .chunks(2)
            .map(|c| {
                let (x, y) = (c[0], c[1]);
                E::memo(b, move |rd| {
                    bump_memo();
                    E::sig(rd, x) + E::sig(rd, y)
                })
            })
            .collect();
        while cur.len() > 1 {
            cur = cur
                .chunks(2)
                .map(|c| {
                    let (x, y) = (c[0], c[1]);
                    E::memo(b, move |rd| {
                        bump_memo();
                        E::mem(rd, x) + E::mem(rd, y)
                    })
                })
                .collect();
        }
        let top = cur[0];
        E::effect(b, move |rd| {
            bump_eff();
            s2[0].set(E::mem(rd, top));
        });
        leaves
    });
    let mut r = Rng(7);
    let mut op = |i: u64| {
        let k = r.below(n);
        E::set(&mut run, leaves[k], i + 1_000_000)
    };
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

const DW: usize = 100;
const DL: usize = 100;

fn dag_wiring() -> Vec<Vec<[usize; 3]>> {
    let mut r = Rng(11);
    (1..DL).map(|_| (0..DW).map(|_| [r.below(DW), r.below(DW), r.below(DW)]).collect()).collect()
}

pub fn dag<E: Engine>(mode: &Mode) -> Res {
    let wiring = dag_wiring();
    let seen = cells(DW);
    let s2 = seen.clone();
    let (mut run, srcs) = E::start(move |b| {
        let srcs: Vec<E::Sig> = (0..DW as u64).map(|i| E::signal(b, i & 1)).collect();
        let mut prev: Vec<E::Memo> = srcs
            .iter()
            .map(|&s| {
                E::memo(b, move |rd| {
                    bump_memo();
                    E::sig(rd, s)
                })
            })
            .collect();
        for layer in &wiring {
            let next: Vec<E::Memo> = layer
                .iter()
                .map(|&[x, y, z]| {
                    let (x, y, z) = (prev[x], prev[y], prev[z]);
                    E::memo(b, move |rd| {
                        bump_memo();
                        (E::mem(rd, x) + E::mem(rd, y) + E::mem(rd, z)) & 0xFFFF
                    })
                })
                .collect();
            prev = next;
        }
        for (i, &m) in prev.iter().enumerate() {
            let c = s2.clone();
            E::effect(b, move |rd| {
                bump_eff();
                c[i].set(E::mem(rd, m));
            });
        }
        srcs
    });
    let mut cur: Vec<u64> = (0..DW as u64).map(|i| i & 1).collect();
    let mut r2 = Rng(5);
    let mut op = |_| {
        let k = r2.below(DW);
        cur[k] ^= 1;
        E::set(&mut run, srcs[k], cur[k]);
    };
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

pub fn dynamic<E: Engine>(n: usize, mode: &Mode) -> Res {
    let seen = cells(n);
    let s2 = seen.clone();
    let (mut run, (a, sel)) = E::start(move |b| {
        let a: Vec<E::Sig> = (0..n as u64).map(|i| E::signal(b, i)).collect();
        let bb: Vec<E::Sig> = (0..n as u64).map(|i| E::signal(b, i + 1)).collect();
        let sel: Vec<E::Sig> = (0..n).map(|_| E::signal(b, 0)).collect();
        for i in 0..n {
            let (s, x, y) = (sel[i], a[i], bb[i]);
            let m = E::memo(b, move |rd| {
                bump_memo();
                if E::sig(rd, s) & 1 == 0 { E::sig(rd, x) } else { E::sig(rd, y) }
            });
            let c = s2.clone();
            E::effect(b, move |rd| {
                bump_eff();
                c[i].set(E::mem(rd, m));
            });
        }
        (a, sel)
    });
    let mut r = Rng(3);
    let mut cur = vec![0u64; n];
    let mut op = |_| {
        let k = r.below(n);
        if r.below(4) == 0 {
            cur[k] ^= 1;
            E::set(&mut run, sel[k], cur[k]);
        } else {
            E::set(&mut run, a[k], r.next() & 0xFFFF);
        }
    };
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

pub fn hotsub<E: Engine>(n: usize, mode: &Mode) -> Res {
    let seen = cells(n);
    let s2 = seen.clone();
    let (mut run, (shared, flags)) = E::start(move |b| {
        let shared = E::signal(b, 0);
        let flags: Vec<E::Sig> = (0..n).map(|_| E::signal(b, 0)).collect();
        for i in 0..n {
            let f = flags[i];
            let m = E::memo(b, move |rd| {
                bump_memo();
                if E::sig(rd, f) & 1 == 1 { E::sig(rd, shared) } else { 0 }
            });
            let c = s2.clone();
            E::effect(b, move |rd| {
                bump_eff();
                c[i].set(E::mem(rd, m));
            });
        }
        (shared, flags)
    });
    let mut r = Rng(9);
    let mut cur = vec![0u64; n];
    let mut op = |i: u64| {
        if i % 8 == 7 {
            E::set(&mut run, shared, i);
        } else {
            let k = r.below(n);
            cur[k] ^= 1;
            E::set(&mut run, flags[k], cur[k]);
        }
    };
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

pub fn bigdeps<E: Engine>(n: usize, mode: &Mode) -> Res {
    let seen = cells(1);
    let c = seen.clone();
    let (mut run, flags) = E::start(move |b| {
        let flags: Vec<E::Sig> = (0..n).map(|_| E::signal(b, 0)).collect();
        let vals: Vec<E::Sig> = (0..n as u64).map(|i| E::signal(b, i)).collect();
        let f2 = flags.clone();
        let agg = E::memo(b, move |rd| {
            bump_memo();
            let mut s = 0u64;
            for i in 0..f2.len() {
                if E::sig(rd, f2[i]) & 1 == 1 {
                    s = s.wrapping_add(E::sig(rd, vals[i]));
                }
            }
            s
        });
        E::effect(b, move |rd| {
            bump_eff();
            c[0].set(E::mem(rd, agg));
        });
        flags
    });
    let mut r = Rng(13);
    let mut cur = vec![0u64; n];
    let mut op = |_| {
        let k = r.below(n);
        cur[k] ^= 1;
        E::set(&mut run, flags[k], cur[k]);
    };
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

const BN: usize = 1000;

fn batch_pairs() -> Vec<(usize, usize)> {
    let mut r = Rng(17);
    (0..BN).map(|_| (r.below(BN), r.below(BN))).collect()
}

pub fn batch<E: Engine>(mode: &Mode) -> Res {
    let pairs = batch_pairs();
    let seen = cells(BN);
    let s2 = seen.clone();
    let (mut run, srcs) = E::start(move |b| {
        let srcs: Vec<E::Sig> = (0..BN as u64).map(|i| E::signal(b, i)).collect();
        for (i, &(x, y)) in pairs.iter().enumerate() {
            let (x, y) = (srcs[x], srcs[y]);
            let m = E::memo(b, move |rd| {
                bump_memo();
                E::sig(rd, x) ^ E::sig(rd, y)
            });
            let c = s2.clone();
            E::effect(b, move |rd| {
                bump_eff();
                c[i].set(E::mem(rd, m));
            });
        }
        srcs
    });
    let mut op = |i: u64| {
        E::set_many(&mut run, 64, |k| (srcs[(i as usize * 31 + k * 17) % BN], i + k as u64 + 7));
    };
    drive(mode, &mut op, &mut || sum_cells(&seen))
}

/// Live heap bytes per node and creation time per node, with the graph settled.
pub fn memory<E: Engine>(kind: &'static str, n: usize) -> (f64, f64) {
    let before = LIVE.load(Relaxed);
    let t = Instant::now();
    let (run, _) = E::start(move |b| match kind {
        "signal" => {
            for i in 0..n as u64 {
                E::signal(b, i);
            }
        }
        "memo_1dep" => {
            let s = E::signal(b, 0);
            for _ in 0..n {
                E::memo(b, move |rd| E::sig(rd, s) + 1);
            }
        }
        "effect_1dep" => {
            let s = E::signal(b, 0);
            for _ in 0..n {
                E::effect(b, move |rd| {
                    std::hint::black_box(E::sig(rd, s));
                });
            }
        }
        "memo+effect" => {
            let s = E::signal(b, 0);
            for _ in 0..n {
                let m = E::memo(b, move |rd| E::sig(rd, s) + 1);
                E::effect(b, move |rd| {
                    std::hint::black_box(E::mem(rd, m));
                });
            }
        }
        _ => unreachable!(),
    });
    let ns = t.elapsed().as_nanos() as f64 / n as f64;
    let per = (LIVE.load(Relaxed) - before) as f64 / n as f64;
    drop(run);
    (per, ns)
}

pub const MEMORY_KINDS: [&str; 4] = ["signal", "memo_1dep", "effect_1dep", "memo+effect"];

pub struct Scenario {
    pub name: &'static str,
    pub unit: &'static str,
}

pub const SCENARIOS: [Scenario; 15] = [
    Scenario { name: "chain_1000", unit: "ns/write" },
    Scenario { name: "chain_10000", unit: "ns/write" },
    Scenario { name: "chain_cut_10000", unit: "ns/write" },
    Scenario { name: "fanout_cut_10000", unit: "ns/write" },
    Scenario { name: "effects_on_signal_1000", unit: "ns/write" },
    Scenario { name: "write_one_effect", unit: "ns/write" },
    Scenario { name: "write_unobserved", unit: "ns/write" },
    Scenario { name: "tree_16384", unit: "ns/write" },
    Scenario { name: "dag_dense_100x100", unit: "ns/write" },
    Scenario { name: "dynamic_switch_10000", unit: "ns/write" },
    Scenario { name: "hotsub_10000", unit: "ns/write" },
    Scenario { name: "bigdeps_5000", unit: "ns/write" },
    Scenario { name: "batch_64_writes_dag", unit: "ns/batch" },
    Scenario { name: "read_clean_memo", unit: "ns/read" },
    Scenario { name: "read_signal", unit: "ns/read" },
];

pub fn run<E: Engine>(name: &str, mode: &Mode) -> Res {
    match name {
        "chain_1000" => chain::<E>(1000, false, mode),
        "chain_10000" => chain::<E>(10000, false, mode),
        "chain_cut_10000" => chain::<E>(10000, true, mode),
        "fanout_cut_10000" => fanout::<E>(10000, mode),
        "effects_on_signal_1000" => effects_on_signal::<E>(1000, mode),
        "write_one_effect" => effects_on_signal::<E>(1, mode),
        "write_unobserved" => write_unobserved::<E>(mode),
        "tree_16384" => tree::<E>(16384, mode),
        "dag_dense_100x100" => dag::<E>(mode),
        "dynamic_switch_10000" => dynamic::<E>(10000, mode),
        "hotsub_10000" => hotsub::<E>(10000, mode),
        "bigdeps_5000" => bigdeps::<E>(5000, mode),
        "batch_64_writes_dag" => batch::<E>(mode),
        "read_clean_memo" => read_memo::<E>(mode),
        "read_signal" => read_signal::<E>(mode),
        _ => unreachable!("unknown scenario {name}"),
    }
}

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

pub static LIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
pub static ALLOCS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

thread_local! {
    static MEMO_EVALS: Cell<u64> = const { Cell::new(0) };
    static EFFECT_RUNS: Cell<u64> = const { Cell::new(0) };
}

#[inline]
pub fn bump_memo() {
    MEMO_EVALS.with(|c| c.set(c.get() + 1));
}

#[inline]
pub fn bump_eff() {
    EFFECT_RUNS.with(|c| c.set(c.get() + 1));
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn counters() -> (u64, u64, usize) {
    (MEMO_EVALS.with(|c| c.get()), EFFECT_RUNS.with(|c| c.get()), ALLOCS.load(Relaxed))
}

#[derive(Clone, Default)]
pub struct Res {
    pub ns_per_op: f64,
    pub memo_evals: f64,
    pub effect_runs: f64,
    pub allocs: f64,
    pub check: u64,
}

pub enum Mode {
    Verify,
    Time,
}

fn median_ns(mut op: impl FnMut(u64)) -> f64 {
    let (reps, budget_ms) = (7usize, 30.0f64);
    let mut i = 100_000u64;
    let mut k = 1u64;
    for _ in 0..3 {
        op(i);
        i += 1;
    }
    loop {
        let t = Instant::now();
        for _ in 0..k {
            op(i);
            i += 1;
        }
        let ms = t.elapsed().as_secs_f64() * 1e3;
        if ms > 4.0 || k >= 1 << 26 {
            k = (((k as f64) * budget_ms / ms.max(0.01)) as u64).max(1);
            break;
        }
        k *= 4;
    }
    let mut v: Vec<f64> = (0..reps)
        .map(|_| {
            let t = Instant::now();
            for _ in 0..k {
                op(i);
                i += 1;
            }
            t.elapsed().as_nanos() as f64 / k as f64
        })
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[reps / 2]
}

/// Runs the scenario's operation either as a verification pass (hash of what `probe` observes
/// after every operation) or as a timing pass (time, computations and allocations per operation).
pub fn drive(mode: &Mode, op: &mut dyn FnMut(u64), probe: &mut dyn FnMut() -> u64) -> Res {
    match mode {
        Mode::Verify => {
            let mut h = 0xcbf29ce484222325u64;
            for i in 0..300 {
                op(i);
                h = (h ^ probe()).wrapping_mul(0x100000001b3);
            }
            Res { check: h, ..Default::default() }
        }
        Mode::Time => {
            let ns = median_ns(&mut *op);
            let n = ((5.0e7 / ns.max(1.0)) as u64).clamp(5, 2000);
            let (m0, e0, a0) = counters();
            for i in 0..n {
                op(500_000 + i);
            }
            let (m1, e1, a1) = counters();
            Res {
                ns_per_op: ns,
                memo_evals: (m1 - m0) as f64 / n as f64,
                effect_runs: (e1 - e0) as f64 / n as f64,
                allocs: (a1 - a0) as f64 / n as f64,
                ..Default::default()
            }
        }
    }
}

pub type Cells = Rc<Vec<Cell<u64>>>;

pub fn cells(n: usize) -> Cells {
    Rc::new((0..n).map(|_| Cell::new(0)).collect())
}

pub fn sum_cells(c: &Cells) -> u64 {
    c.iter().fold(0u64, |a, x| a.wrapping_mul(31).wrapping_add(x.get()))
}

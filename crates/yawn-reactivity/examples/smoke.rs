use std::time::Instant;
use yawn_reactivity::*;

fn main() {
    // chain: one write recomputes every node
    let mut rt = Runtime::new();
    let root = rt.signal(0u64);
    let mut prev = rt.memo(move |rt| rt.get(root) + 1);
    for _ in 0..10_000 {
        let p = prev;
        prev = rt.memo(move |rt| rt.get(p) + 1);
    }
    let last = prev;
    rt.effect(move |rt| {
        rt.get(last);
    });
    let t = Instant::now();
    for i in 1..=200u64 {
        rt.set(root, i);
    }
    println!("chain 10k:   {:>8.2} us/write", t.elapsed().as_secs_f64() * 1e6 / 200.0);

    // fan-out with cutoff: half the writes are absorbed by the parity memo
    let mut rt = Runtime::new();
    let s = rt.signal(0u64);
    let parity = rt.memo(move |rt| rt.get(s) & 1);
    for _ in 0..10_000 {
        rt.effect(move |rt| {
            rt.get(parity);
        });
    }
    let t = Instant::now();
    for i in 1..=2_000u64 {
        rt.set(s, i);
    }
    println!("fanout 10k:  {:>8.2} us/write", t.elapsed().as_secs_f64() * 1e6 / 2000.0);

    // tree: O(log n) recomputation per leaf write
    let mut rt = Runtime::new();
    let leaves: Vec<Signal<u64>> = (0..65_536u64).map(|i| rt.signal(i)).collect();
    let mut level: Vec<Memo<u64>> = leaves
        .chunks(2)
        .map(|c| {
            let (a, b) = (c[0], c[1]);
            rt.memo(move |rt| rt.get(a) + rt.get(b))
        })
        .collect();
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|c| {
                let (a, b) = (c[0], c[1]);
                rt.memo(move |rt| rt.get(a) + rt.get(b))
            })
            .collect();
    }
    let top = level[0];
    rt.effect(move |rt| {
        rt.get(top);
    });
    let t = Instant::now();
    for i in 0..20_000usize {
        rt.set(leaves[(i * 7919) % leaves.len()], i as u64 + 1_000_000);
    }
    println!("tree 64k:    {:>8.2} us/write", t.elapsed().as_secs_f64() * 1e6 / 20000.0);
}

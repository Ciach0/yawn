//! Data for the benchmark: rows with a label signal, built the way js-framework-benchmark does.

use std::cell::Cell;
use yawn::ui::{signal, Sig};

#[derive(Clone)]
pub struct Item {
    pub id: u32,
    pub label: Sig<String>,
}

thread_local! {
    static NEXT: Cell<u32> = const { Cell::new(1) };
    static SEED: Cell<u32> = const { Cell::new(12345) };
}

const ADJECTIVES: [&str; 12] = ["pretty", "large", "big", "small", "tall", "short", "long", "handsome", "plain", "quaint", "clean", "elegant"];
const COLOURS: [&str; 8] = ["red", "yellow", "blue", "green", "pink", "brown", "purple", "orange"];
const NOUNS: [&str; 10] = ["table", "chair", "house", "bbq", "desk", "car", "pony", "cookie", "sandwich", "burger"];

fn random(max: usize) -> usize {
    SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        s.set(x);
        x as usize % max
    })
}

/// `n` new rows with fresh ids.
pub fn build(n: usize) -> Vec<Item> {
    (0..n)
        .map(|_| {
            let id = NEXT.with(|c| {
                let id = c.get();
                c.set(id + 1);
                id
            });
            let label = format!("{} {} {}", ADJECTIVES[random(ADJECTIVES.len())], COLOURS[random(COLOURS.len())], NOUNS[random(NOUNS.len())]);
            Item { id, label: signal(label) }
        })
        .collect()
}

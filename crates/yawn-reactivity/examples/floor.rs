use std::any::Any;
use std::time::Instant;

type Slot = Option<Box<dyn Any>>;
type F = Box<dyn FnMut(&Vec<Slot>, &mut Slot) -> bool>;

fn main() {
    let n = 100_000usize;
    let mut slots: Vec<Slot> = (0..=n).map(|_| Some(Box::new(0u64) as Box<dyn Any>)).collect();
    let mut fs: Vec<F> = (1..=n)
        .map(|i| {
            Box::new(move |all: &Vec<Slot>, out: &mut Slot| {
                let v = *all[i - 1].as_ref().unwrap().downcast_ref::<u64>().unwrap() + 1;
                let o = out.as_mut().unwrap().downcast_mut::<u64>().unwrap();
                if *o == v {
                    false
                } else {
                    *o = v;
                    true
                }
            }) as F
        })
        .collect();
    let mut best = f64::MAX;
    for rep in 0..9u64 {
        let t = Instant::now();
        for r in 0..20u64 {
            *slots[0].as_mut().unwrap().downcast_mut::<u64>().unwrap() = rep * 20 + r + 1;
            for i in 1..=n {
                let mut out = slots[i].take();
                let changed = fs[i - 1](&slots, &mut out);
                slots[i] = out;
                std::hint::black_box(changed);
            }
        }
        best = best.min(t.elapsed().as_nanos() as f64 / 20.0 / n as f64);
    }
    println!("floor: dyn closure + Any slots, in-order chain: {best:.1} ns/node");
}

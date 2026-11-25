use crate::engine::Engine;
use yawn_reactivity::{Memo, Runtime, Signal};

pub struct Yawn;

impl Engine for Yawn {
    type Sig = Signal<u64>;
    type Memo = Memo<u64>;
    type Rd = Runtime;
    type Bld = Runtime;
    type Run = Runtime;

    fn start<R: 'static>(build: impl FnOnce(&mut Runtime) -> R + 'static) -> (Runtime, R) {
        let mut rt = Runtime::new();
        let r = build(&mut rt);
        (rt, r)
    }

    fn signal(b: &mut Runtime, v: u64) -> Self::Sig {
        b.signal(v)
    }

    fn memo(b: &mut Runtime, f: impl Fn(&mut Runtime) -> u64 + 'static) -> Self::Memo {
        b.memo(move |rt| f(rt))
    }

    fn effect(b: &mut Runtime, f: impl Fn(&mut Runtime) + 'static) {
        b.effect(move |rt| f(rt));
    }

    fn sig(rd: &mut Runtime, s: Self::Sig) -> u64 {
        rd.get(s)
    }

    fn mem(rd: &mut Runtime, m: Self::Memo) -> u64 {
        rd.get(m)
    }

    fn set(run: &mut Runtime, s: Self::Sig, v: u64) {
        run.set(s, v);
    }

    fn set_many(run: &mut Runtime, n: usize, f: impl Fn(usize) -> (Self::Sig, u64)) {
        run.batch(|rt| {
            for k in 0..n {
                let (s, v) = f(k);
                rt.set(s, v);
            }
        });
    }

    fn read_sig(run: &mut Runtime, s: Self::Sig) -> u64 {
        run.get(s)
    }

    fn read_mem(run: &mut Runtime, m: Self::Memo) -> u64 {
        run.get(m)
    }
}

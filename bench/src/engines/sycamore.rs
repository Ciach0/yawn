use crate::engine::Engine;
use std::cell::RefCell;
use std::rc::Rc;
use sycamore_reactive::{
    ReadSignal, RootHandle, Signal, batch, create_effect, create_root, create_selector,
    create_signal,
};

pub struct Sycamore;

pub struct Run(RootHandle);

impl Drop for Run {
    fn drop(&mut self) {
        self.0.dispose();
    }
}

impl Engine for Sycamore {
    type Sig = Signal<u64>;
    type Memo = ReadSignal<u64>;
    type Rd = ();
    type Bld = ();
    type Run = Run;

    fn start<R: 'static>(build: impl FnOnce(&mut ()) -> R + 'static) -> (Run, R) {
        let out: Rc<RefCell<Option<R>>> = Rc::new(RefCell::new(None));
        let o2 = out.clone();
        let root = create_root(move || {
            *o2.borrow_mut() = Some(build(&mut ()));
        });
        let r = out.borrow_mut().take().expect("root ran");
        (Run(root), r)
    }

    fn signal(_: &mut (), v: u64) -> Self::Sig {
        create_signal(v)
    }

    /// `create_selector`: skips the notification when the value did not change, like the other
    /// libraries' memos. `create_memo` always notifies.
    fn memo(_: &mut (), f: impl Fn(&mut ()) -> u64 + 'static) -> Self::Memo {
        create_selector(move || f(&mut ()))
    }

    fn effect(_: &mut (), f: impl Fn(&mut ()) + 'static) {
        create_effect(move || f(&mut ()));
    }

    fn sig(_: &mut (), s: Self::Sig) -> u64 {
        s.get()
    }

    fn mem(_: &mut (), m: Self::Memo) -> u64 {
        m.get()
    }

    fn set(run: &mut Run, s: Self::Sig, v: u64) {
        run.0.run_in(|| s.set(v));
    }

    fn set_many(run: &mut Run, n: usize, f: impl Fn(usize) -> (Self::Sig, u64)) {
        run.0.run_in(|| {
            batch(|| {
                for k in 0..n {
                    let (s, v) = f(k);
                    s.set(v);
                }
            })
        });
    }

    fn read_sig(run: &mut Run, s: Self::Sig) -> u64 {
        run.0.run_in(|| s.get_untracked())
    }

    fn read_mem(run: &mut Run, m: Self::Memo) -> u64 {
        run.0.run_in(|| m.get_untracked())
    }
}

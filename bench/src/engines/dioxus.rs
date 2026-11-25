use crate::engine::Engine;
use dioxus_core::{Element, VNode, VirtualDom};
use dioxus_hooks::use_effect;
use dioxus_signals::{Memo, ReadableExt, Signal, WritableExt};
use std::cell::RefCell;
use std::rc::Rc;

pub struct Dioxus;

thread_local! {
    static BUILD: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

fn app() -> Element {
    if let Some(f) = BUILD.with(|b| b.borrow_mut().take()) {
        f();
    }
    VNode::empty()
}

impl Engine for Dioxus {
    type Sig = Signal<u64>;
    type Memo = Memo<u64>;
    type Rd = ();
    type Bld = ();
    type Run = VirtualDom;

    /// Builds inside the root component's render, as an app would, and lets the scheduler run the
    /// initial effects.
    fn start<R: 'static>(build: impl FnOnce(&mut ()) -> R + 'static) -> (VirtualDom, R) {
        let mut dom = VirtualDom::new(app);
        let out: Rc<RefCell<Option<R>>> = Rc::new(RefCell::new(None));
        let o2 = out.clone();
        BUILD.with(|b| {
            *b.borrow_mut() = Some(Box::new(move || {
                *o2.borrow_mut() = Some(build(&mut ()));
            }))
        });
        dom.rebuild_in_place();
        dom.process_events();
        let r = out.borrow_mut().take().expect("root component ran");
        (dom, r)
    }

    fn signal(_: &mut (), v: u64) -> Self::Sig {
        Signal::new(v)
    }

    fn memo(_: &mut (), f: impl Fn(&mut ()) -> u64 + 'static) -> Self::Memo {
        Memo::new(move || f(&mut ()))
    }

    fn effect(_: &mut (), f: impl Fn(&mut ()) + 'static) {
        use_effect(move || f(&mut ()));
    }

    fn sig(_: &mut (), s: Self::Sig) -> u64 {
        *s.read()
    }

    fn mem(_: &mut (), m: Self::Memo) -> u64 {
        *m.read()
    }

    fn set(run: &mut VirtualDom, s: Self::Sig, v: u64) {
        let mut s = s;
        s.set(v);
        run.process_events();
    }

    /// dioxus-signals has no batch API: the writes are coalesced by the scheduler.
    fn set_many(run: &mut VirtualDom, n: usize, f: impl Fn(usize) -> (Self::Sig, u64)) {
        for k in 0..n {
            let (mut s, v) = f(k);
            s.set(v);
        }
        run.process_events();
    }

    fn read_sig(_: &mut VirtualDom, s: Self::Sig) -> u64 {
        *s.read()
    }

    fn read_mem(_: &mut VirtualDom, m: Self::Memo) -> u64 {
        *m.read()
    }
}

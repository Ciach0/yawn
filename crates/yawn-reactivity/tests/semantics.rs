#![allow(clippy::type_complexity, clippy::needless_range_loop)]
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use yawn_reactivity::*;

fn log<T: 'static>() -> (Rc<RefCell<Vec<T>>>, Rc<RefCell<Vec<T>>>) {
    let l = Rc::new(RefCell::new(Vec::new()));
    (l.clone(), l)
}

#[test]
fn values_are_arbitrary_types() {
    let mut rt = Runtime::new();
    let name = rt.signal(String::from("ada"));
    let tags = rt.signal(vec![1u8, 2]);
    let opt = rt.memo(move |rt| rt.with(name, |s| s.chars().next()));
    let len = rt.memo(move |rt| rt.with(tags, |v| v.len()));
    assert_eq!(rt.get(opt), Some('a'));
    assert_eq!(rt.get(len), 2);
    rt.set(name, "bob".into());
    rt.update(tags, |v| v.push(3));
    assert_eq!(rt.get(opt), Some('b'));
    assert_eq!(rt.get(len), 3);
}

#[test]
fn custom_equality_for_types_without_partial_eq() {
    struct Opaque(u32);
    let mut rt = Runtime::new();
    let s = rt.signal_with(Opaque(1), |_, _| false);
    let (l, l2) = log::<u32>();
    rt.effect(move |rt| l2.borrow_mut().push(rt.with(s, |o| o.0)));
    rt.set(s, Opaque(1)); // "never equal": still a change
    assert_eq!(*l.borrow(), vec![1, 1]);
}

#[test]
fn diamond_is_glitch_free() {
    let mut rt = Runtime::new();
    let a = rt.signal(1);
    let b = rt.memo(move |rt| rt.get(a) * 2);
    let c = rt.memo(move |rt| rt.get(a) * 3);
    let (l, l2) = log::<(i32, i32)>();
    rt.effect(move |rt| l2.borrow_mut().push((rt.get(b), rt.get(c))));
    rt.set(a, 2);
    // never an inconsistent pair such as (4, 3)
    assert_eq!(*l.borrow(), vec![(2, 3), (4, 6)]);
}

#[test]
fn cutoff_stops_propagation() {
    let mut rt = Runtime::new();
    let a = rt.signal(1);
    let parity = rt.memo(move |rt| rt.get(a) % 2);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    rt.effect(move |rt| {
        rt.get(parity);
        r.set(r.get() + 1);
    });
    rt.set(a, 3); // parity unchanged
    rt.set(a, 4); // parity changes
    assert_eq!(runs.get(), 2);
    rt.set(a, 4); // same value: no write at all
    assert_eq!(runs.get(), 2);
}

#[test]
fn dynamic_dependencies_follow_branches() {
    let mut rt = Runtime::new();
    let use_a = rt.signal(true);
    let a = rt.signal(1);
    let b = rt.signal(100);
    let evals = Rc::new(Cell::new(0));
    let e = evals.clone();
    let m = rt.memo(move |rt| {
        e.set(e.get() + 1);
        if rt.get(use_a) { rt.get(a) } else { rt.get(b) }
    });
    assert_eq!(rt.get(m), 1);
    rt.set(b, 101); // not a dependency yet
    assert_eq!(evals.get(), 1);
    rt.set(use_a, false);
    assert_eq!(rt.get(m), 101);
    rt.set(a, 2); // dropped dependency
    assert_eq!(evals.get(), 2);
    rt.set(b, 102);
    assert_eq!(rt.get(m), 102);
}

#[test]
fn gating_dependency_is_validated_first() {
    // the guarded read would panic if validated before the gate
    let mut rt = Runtime::new();
    let idx = rt.signal(0usize);
    let data = rt.signal(vec![10, 20, 30]);
    let len = rt.memo(move |rt| rt.with(data, |d| d.len()));
    let guard = rt.memo(move |rt| {
        let i = rt.get(idx);
        if i < rt.get(len) { rt.with(data, |d| d[i]) } else { -1 }
    });
    rt.batch(|rt| {
        rt.set(idx, 2);
        rt.set(data, vec![1]);
    });
    assert_eq!(rt.get(guard), -1);
}

#[test]
fn batch_runs_effects_once_and_reads_stay_consistent() {
    let mut rt = Runtime::new();
    let a = rt.signal(1);
    let b = rt.signal(1);
    let sum = rt.memo(move |rt| rt.get(a) + rt.get(b));
    let (l, l2) = log::<i32>();
    rt.effect(move |rt| l2.borrow_mut().push(rt.get(sum)));
    let seen = rt.batch(|rt| {
        rt.set(a, 5);
        let mid = rt.get(sum); // memo is validated on demand inside the batch
        rt.set(b, 7);
        mid
    });
    assert_eq!(seen, 6);
    assert_eq!(*l.borrow(), vec![2, 12]); // intermediate value never reached the effect
    // nested batches flush only at the outermost end
    rt.batch(|rt| {
        rt.batch(|rt| rt.set(a, 0));
        assert_eq!(l.borrow().len(), 2);
    });
    assert_eq!(l.borrow().len(), 3);
}

#[test]
fn effects_run_in_creation_order() {
    let mut rt = Runtime::new();
    let s = rt.signal(0);
    let (l, l2) = log::<&'static str>();
    let (a, b, c) = (l2.clone(), l2.clone(), l2);
    rt.effect(move |rt| {
        rt.get(s);
        a.borrow_mut().push("first");
    });
    rt.effect(move |rt| {
        rt.get(s);
        b.borrow_mut().push("second");
    });
    rt.effect(move |rt| {
        rt.get(s);
        c.borrow_mut().push("third");
    });
    l.borrow_mut().clear();
    rt.set(s, 1);
    assert_eq!(*l.borrow(), vec!["first", "second", "third"]);
}

#[test]
fn effect_writes_propagate_in_later_passes() {
    let mut rt = Runtime::new();
    let src = rt.signal(1);
    let mirror = rt.signal(0);
    rt.effect(move |rt| {
        let v = rt.get(src);
        rt.set(mirror, v * 10);
    });
    let (l, l2) = log::<i32>();
    rt.effect(move |rt| l2.borrow_mut().push(rt.get(mirror)));
    rt.set(src, 2);
    assert_eq!(*l.borrow(), vec![10, 20]);
}

#[test]
fn feedback_loop_panics_instead_of_hanging() {
    let mut rt = Runtime::new();
    let a = rt.signal(0u64);
    let b = rt.signal(0u64);
    rt.effect(move |rt| {
        let v = rt.get(a);
        rt.set(b, v + 1);
    });
    rt.effect(move |rt| {
        let v = rt.get(b);
        rt.set(a, v + 1);
    });
    let r = catch_unwind(AssertUnwindSafe(|| rt.set(a, 1000)));
    assert!(r.is_err());
}

#[test]
fn stale_handles_are_safe_and_generations_prevent_aliasing() {
    let mut rt = Runtime::new();
    let a = rt.signal(1);
    assert!(rt.dispose(a));
    assert!(!rt.dispose(a), "dispose is idempotent");
    assert!(!rt.is_alive(a));
    assert_eq!(rt.try_get(a), None);
    assert_eq!(rt.try_set(a, 5), None);
    // the slot is reused by a new signal; the old handle must not see it
    let b = rt.signal(99);
    assert_eq!(rt.try_get(a), None);
    assert_eq!(rt.get(b), 99);
    assert!(rt.try_set(a, 7).is_none());
    assert_eq!(rt.get(b), 99);
    assert!(catch_unwind(AssertUnwindSafe(|| rt.get(a))).is_err());
}

#[test]
fn disposing_a_dependency_recomputes_observers() {
    let mut rt = Runtime::new();
    let gate = rt.signal(true);
    let x = rt.signal(5);
    let m = rt.memo(move |rt| if rt.get(gate) { rt.try_get(x).unwrap_or(-1) } else { 0 });
    let (l, l2) = log::<i32>();
    rt.effect(move |rt| l2.borrow_mut().push(rt.get(m)));
    rt.dispose(x);
    assert_eq!(*l.borrow(), vec![5, -1]);
    assert_eq!(rt.get(m), -1);
}

#[test]
fn ownership_cleanup_and_scopes() {
    let mut rt = Runtime::new();
    let trigger = rt.signal(0);
    let cleaned = Rc::new(Cell::new(0));
    let c = cleaned.clone();
    let base = rt.node_count();
    let eff = rt.effect(move |rt| {
        rt.get(trigger);
        let c = c.clone();
        rt.on_cleanup(move || c.set(c.get() + 1));
        // children are recreated on every run, never accumulated
        let inner = rt.signal(1);
        rt.memo(move |rt| rt.get(inner) + 1);
    });
    assert_eq!(rt.node_count(), base + 3);
    rt.set(trigger, 1);
    rt.set(trigger, 2);
    assert_eq!(cleaned.get(), 2);
    assert_eq!(rt.node_count(), base + 3);
    rt.dispose(eff);
    assert_eq!(cleaned.get(), 3);
    assert_eq!(rt.node_count(), base);

    let kept = Rc::new(RefCell::new(None));
    let k = kept.clone();
    let sc = rt.scope(move |rt| {
        let s = rt.signal(1);
        *k.borrow_mut() = Some(s);
        rt.effect(move |rt| {
            rt.get(s);
        });
    });
    let s = kept.borrow().unwrap();
    assert!(rt.is_alive(s));
    rt.dispose(sc);
    assert!(!rt.is_alive(s));
    assert_eq!(rt.node_count(), base);
}

#[test]
fn disposed_effect_never_runs_again() {
    let mut rt = Runtime::new();
    let s = rt.signal(0);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    let e = rt.effect(move |rt| {
        rt.get(s);
        r.set(r.get() + 1);
    });
    rt.batch(|rt| {
        rt.set(s, 1); // queues the effect
        rt.dispose(e); // ...but it's gone before the flush
    });
    assert_eq!(runs.get(), 1);
}

#[test]
fn contract_violations_panic() {
    let mut rt = Runtime::new();
    let s = rt.signal(0);
    // writing from a memo
    let r = catch_unwind(AssertUnwindSafe(|| {
        rt.memo(move |rt| {
            rt.set(s, 1);
            0
        })
    }));
    assert!(r.is_err());

    let mut rt = Runtime::new();
    let other = Runtime::new();
    let _ = other;
    let a = rt.signal(1);
    let mut rt2 = Runtime::new();
    assert!(catch_unwind(AssertUnwindSafe(|| rt2.get(a))).is_err(), "foreign handle");

    let mut rt = Runtime::new();
    let r = catch_unwind(AssertUnwindSafe(|| rt.on_cleanup(|| {})));
    assert!(r.is_err(), "on_cleanup outside an owner");
}

#[test]
fn panicking_computation_poisons_the_runtime() {
    let mut rt = Runtime::new();
    let s = rt.signal(0);
    let r = catch_unwind(AssertUnwindSafe(|| {
        rt.effect(move |rt| {
            if rt.get(s) == 0 {
                panic!("boom");
            }
        })
    }));
    assert!(r.is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| rt.get(s))).is_err());
}

#[test]
fn self_dispose_while_running_panics() {
    let mut rt = Runtime::new();
    let cell: Rc<Cell<Option<Effect>>> = Rc::new(Cell::new(None));
    let c = cell.clone();
    let s = rt.signal(0);
    let e = rt.effect(move |rt| {
        if rt.get(s) == 1 {
            rt.dispose(c.get().unwrap());
        }
    });
    cell.set(Some(e));
    assert!(catch_unwind(AssertUnwindSafe(|| rt.set(s, 1))).is_err());
}

#[test]
fn deep_chain_does_not_overflow_the_stack() {
    let mut rt = Runtime::new();
    let root = rt.signal(0u64);
    let mut prev: Memo<u64> = rt.memo(move |rt| rt.get(root) + 1);
    for _ in 0..300_000 {
        let p = prev;
        prev = rt.memo(move |rt| rt.get(p) + 1);
    }
    let last = prev;
    let (l, l2) = log::<u64>();
    rt.effect(move |rt| l2.borrow_mut().push(rt.get(last)));
    rt.set(root, 5);
    assert_eq!(*l.borrow(), vec![300_001, 300_006]);
    drop(rt); // dropping a deep graph is iterative too
}

#[test]
fn wide_dependency_lists_are_cheap_to_edit() {
    let mut rt = Runtime::new();
    let n = 20_000;
    let flags: Vec<Signal<bool>> = (0..n).map(|_| rt.signal(false)).collect();
    let fl = flags.clone();
    let agg = rt.memo(move |rt| fl.iter().filter(|&&f| rt.get(f)).count());
    let t = std::time::Instant::now();
    for i in 0..n {
        rt.set(flags[i], true);
        rt.set(flags[i], false);
    }
    assert_eq!(rt.get(agg), 0);
    // quadratic edge handling would take far longer than this on any machine
    assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
}

#[test]
fn slots_are_reused_without_growth() {
    let mut rt = Runtime::new();
    let src = rt.signal(0);
    let base = rt.node_count();
    for i in 0..10_000 {
        let m = rt.memo(move |rt| rt.get(src) + i);
        let e = rt.effect(move |rt| {
            rt.get(m);
        });
        rt.dispose(e);
        rt.dispose(m);
    }
    assert_eq!(rt.node_count(), base);
    // observers lists of the long-lived source stayed clean too: a write touches nothing stale
    rt.set(src, 1);
}

#[test]
fn late_dependency_reorders_and_stays_glitch_free() {
    // `early` is created first but switches to read `late`, which is created afterwards: the
    // engine must repair its internal order, and observers must still never see a torn state.
    let mut rt = Runtime::new();
    let sw = rt.signal(false);
    let src = rt.signal(1);
    let slot: Rc<Cell<Option<Memo<i32>>>> = Rc::new(Cell::new(None));
    let s2 = slot.clone();
    let early = rt.memo(move |rt| {
        let base = rt.get(src);
        let on = rt.get(sw);
        match s2.get() {
            Some(late) if on => base + rt.get(late),
            _ => base,
        }
    });
    let late = rt.memo(move |rt| rt.get(src) * 100);
    slot.set(Some(late));
    let pair = rt.memo(move |rt| (rt.get(early), rt.get(late)));
    let (l, l2) = log::<(i32, i32)>();
    rt.effect(move |rt| l2.borrow_mut().push(rt.get(pair)));
    rt.set(sw, true); // adds the backward edge early -> late
    rt.set(src, 2);
    assert_eq!(*l.borrow(), vec![(1, 100), (101, 100), (202, 200)]);
    rt.set(sw, false);
    rt.set(src, 3);
    assert_eq!(l.borrow().last(), Some(&(3, 300)));
    rt.debug_check();
}

#[test]
fn dynamic_cycle_is_detected() {
    let mut rt = Runtime::new();
    let sw = rt.signal(false);
    let slot: Rc<Cell<Option<Memo<i32>>>> = Rc::new(Cell::new(None));
    let s2 = slot.clone();
    let a = rt.memo(move |rt| {
        let on = rt.get(sw);
        match s2.get() {
            Some(b) if on => rt.get(b),
            _ => 1,
        }
    });
    let b = rt.memo(move |rt| rt.get(a) + 1);
    slot.set(Some(b));
    // a would now read b, and b reads a
    let r = catch_unwind(AssertUnwindSafe(|| rt.set(sw, true)));
    assert!(r.is_err());
}

#[test]
fn unobserved_memos_are_lazy() {
    let mut rt = Runtime::new();
    let s = rt.signal(1);
    let evals = Rc::new(Cell::new(0));
    let e = evals.clone();
    let m = rt.memo(move |rt| {
        e.set(e.get() + 1);
        rt.get(s) * 2
    });
    assert_eq!(evals.get(), 1);
    for i in 2..10 {
        rt.set(s, i);
    }
    assert_eq!(evals.get(), 1, "no observer, no recomputation");
    assert_eq!(rt.get(m), 18);
    assert_eq!(evals.get(), 2);
    assert_eq!(rt.get(m), 18);
    assert_eq!(evals.get(), 2);
}

#[test]
fn effect_does_not_rerun_when_a_batch_restores_the_value() {
    let mut rt = Runtime::new();
    let a = rt.signal(1);
    let m = rt.memo(move |rt| rt.get(a) + 1);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    rt.effect(move |rt| {
        rt.get(m);
        r.set(r.get() + 1);
    });
    rt.batch(|rt| {
        rt.set(a, 50);
        rt.set(a, 1);
    });
    assert_eq!(runs.get(), 1);
}

#[test]
fn heavy_churn_renumbers_ranks_with_queued_work_pending() {
    let mut rt = Runtime::new();
    let a = rt.signal(1);
    let m = rt.memo(move |rt| rt.get(a) + 1);
    let n = rt.memo(move |rt| rt.get(m) * 2);
    let (l, l2) = log::<i32>();
    rt.effect(move |rt| l2.borrow_mut().push(rt.get(n)));
    rt.batch(|rt| {
        rt.set(a, 10); // queues m (and later n) before the churn below
        for i in 0..6_000 {
            let t = rt.memo(move |rt| rt.get(a) + i);
            rt.dispose(t);
        }
        rt.set(a, 20);
    });
    assert_eq!(*l.borrow(), vec![4, 42]);
    rt.debug_check();
    rt.set(a, 30);
    assert_eq!(l.borrow().last(), Some(&62));
}

#[test]
fn reading_inside_an_effect_after_it_writes_sees_the_write() {
    let mut rt = Runtime::new();
    let a = rt.signal(1);
    let m = rt.memo(move |rt| rt.get(a) * 10);
    let trigger = rt.signal(0);
    let (l, l2) = log::<i32>();
    rt.effect(move |rt| {
        if rt.get(trigger) > 0 {
            rt.set(a, 7);
            l2.borrow_mut().push(rt.get(m)); // m must already reflect a == 7
        }
    });
    rt.set(trigger, 1);
    assert_eq!(*l.borrow(), vec![70]);
}

#[test]
fn deep_backward_dynamic_chain_fails_cleanly_instead_of_overflowing() {
    // Reading the end of a long queued chain would recurse once per node: the engine must panic
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            let mut rt = Runtime::new();
            let sw = rt.signal(false);
            let root = rt.signal(0u64);
            let slot: Rc<Cell<Option<Memo<u64>>>> = Rc::new(Cell::new(None));
            let s2 = slot.clone();
            let early = rt.memo(move |rt| {
                let on = rt.get(sw);
                match s2.get() {
                    Some(end) if on => rt.get(end),
                    _ => 0,
                }
            });
            let mut prev = rt.memo(move |rt| rt.get(root) + 1);
            for _ in 0..50_000 {
                let p = prev;
                prev = rt.memo(move |rt| rt.get(p) + 1);
            }
            slot.set(Some(prev));
            let end = prev;
            rt.effect(move |rt| {
                rt.get(early);
                rt.get(end);
            });
            let r = catch_unwind(AssertUnwindSafe(|| {
                rt.batch(|rt| {
                    rt.set(root, 1);
                    rt.set(sw, true);
                })
            }));
            // either evaluated correctly or refused with a panic: never a crash
            if r.is_ok() {
                assert_eq!(rt.get(early), 50_002);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn scope_in_survives_reruns_of_the_running_effect_and_dies_with_its_owner() {
    let mut rt = Runtime::new();
    let owner = rt.scope(|_| {});
    let trigger = rt.signal(0);
    let child_cleanups = Rc::new(RefCell::new(0));
    let kept: Rc<RefCell<Vec<yawn_reactivity::Scope>>> = Rc::default();
    let (cc, k) = (child_cleanups.clone(), kept.clone());
    rt.effect(move |rt| {
        let n = rt.get(trigger);
        if n == 0 {
            let cc = cc.clone();
            // created while the effect runs, but owned by `owner`
            let s = rt.scope_in(owner, move |rt| rt.on_cleanup(move || *cc.borrow_mut() += 1));
            k.borrow_mut().push(s);
        }
    });
    rt.set(trigger, 1); // effect re-runs: ordinary children would be disposed here
    assert_eq!(*child_cleanups.borrow(), 0);
    assert!(rt.is_alive(kept.borrow()[0]));
    rt.dispose(owner);
    assert_eq!(*child_cleanups.borrow(), 1);
    assert!(!rt.is_alive(kept.borrow()[0]));
}

#[test]
fn scope_in_with_a_disposed_owner_panics_clearly() {
    let mut rt = Runtime::new();
    let owner = rt.scope(|_| {});
    rt.dispose(owner);
    let r = catch_unwind(AssertUnwindSafe(|| {
        rt.scope_in(owner, |_| {});
    }));
    assert!(r.is_err());
}

#[test]
fn an_owner_of_another_runtime_does_not_run_and_does_not_panic() {
    let mut old = Runtime::new();
    let mut owner = None;
    old.scope(|rt| owner = rt.current_scope());
    let mut now = Runtime::new();
    let ran = now.run_in(owner.expect("a scope"), |_| 1);
    assert_eq!(ran, None);
}

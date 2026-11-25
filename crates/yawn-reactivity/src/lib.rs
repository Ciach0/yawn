//! Single-threaded, fine-grained reactivity for Yawn.
//!
//! ```
//! use yawn_reactivity::Runtime;
//!
//! let mut rt = Runtime::new();
//! let a = rt.signal(1);
//! let b = rt.signal(2);
//! let sum = rt.memo(move |rt| rt.get(a) + rt.get(b));
//! let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
//! let l = log.clone();
//! rt.effect(move |rt| l.borrow_mut().push(rt.get(sum)));
//!
//! rt.set(a, 10);
//! rt.batch(|rt| {
//!     rt.set(a, 20);
//!     rt.set(b, 5);
//! });
//! assert_eq!(*log.borrow(), vec![3, 12, 25]);
//! ```
//!
//! # Model
//!
//! * **Values** are any `'static` Rust type. [`Signal`] and [`Memo`] stop propagation when a new
//!   value equals the old one (`PartialEq`, or a custom comparison via the `*_with` constructors).
//! * **Handles** ([`Signal`], [`Memo`], [`Effect`], [`Scope`]) are `Copy` and generational. Using a
//!   handle after its node was disposed never touches another node: `try_*` methods return `None`,
//!   plain `get`/`set` panic with a clear message, and `dispose` is idempotent. Handles from a
//!   different [`Runtime`] panic.
//! * **Reads are consistent.** `get` on a memo always returns a value that reflects every write
//!   made so far, even in the middle of a [`Runtime::batch`] (memos are validated on demand).
//!   Reads inside a memo or effect are tracked as dependencies; [`Runtime::untracked`] and
//!   `peek` read without tracking. Dependencies are dynamic and re-collected on every run.
//! * **Effects** run once at creation and again after the writes that invalidate them have been
//!   applied: immediately after a lone `set`, or when the outermost `batch` ends. Within a flush,
//!   effects run in creation order and each at most once per pass. An effect may write signals;
//!   the effects that triggers run in the next pass, and a runaway feedback loop panics. An effect
//!   only re-runs if one of its dependencies changed value.
//! * **Memos are pure.** Writing a signal from inside a memo panics. A memo is computed when it
//!   is created; afterwards it's recomputed eagerly while something observes it (another memo
//!   or an effect) and only on demand while nobody does, so unobserved branches cost nothing.
//! * **Ownership.** Nodes created while a memo, effect or [`Scope`] is running belong to it and
//!   are disposed before it re-runs and when it is disposed. [`Runtime::on_cleanup`] registers
//!   code for the same moments. Disposing a node removes it from the graph; observers that still
//!   read it lose the dependency and are recomputed (a recomputation that reads the dead handle
//!   again will panic, as it should).
//! * **Panics.** A panic inside a computation poisons the runtime, because the graph may be
//!   half-updated. Every later call panics with that explanation.
//!
//! The crate contains no `unsafe` code. The propagation algorithm lives in a private module
//! (rank-ordered push with cutoff, see `graph.rs`); the public surface is only what's described
//! here, so the algorithm can change freely.
//!
//! # Limits
//!
//! * A computation that dynamically starts reading a node *created later* while a long chain of
//!   still-queued nodes sits behind it's evaluated on demand, one nested call per node. Beyond
//!   4 000 such nested evaluations the runtime panics (and is poisoned). Graphs built bottom-up,
//!   which is the normal case, never hit this.

#![forbid(unsafe_code)]
#![allow(clippy::type_complexity)]

mod graph;

use graph::{Compute, NodeId};
use std::any::Any;
use std::marker::PhantomData;

pub use graph::{Runtime, TakenSignal};

macro_rules! handle_impls {
    ($name:ident) => {
        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<T> Copy for $name<T> {}
        impl<T> PartialEq for $name<T> {
            fn eq(&self, o: &Self) -> bool {
                self.id == o.id
            }
        }
        impl<T> Eq for $name<T> {}
        impl<T> std::hash::Hash for $name<T> {
            fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
                self.id.hash(h)
            }
        }
        impl<T> std::fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!(stringify!($name), "(#{}.{})"), self.id.idx, self.id.r#gen)
            }
        }
    };
}

/// A writable value
pub struct Signal<T> {
    id: NodeId,
    _t: PhantomData<fn() -> T>,
}
handle_impls!(Signal);

/// A derived value, recomputed when its dependencies change
pub struct Memo<T> {
    id: NodeId,
    _t: PhantomData<fn() -> T>,
}
handle_impls!(Memo);

/// A side effect that re-runs when what it read changes
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Effect {
    id: NodeId,
}

/// An ownership scope: disposing it disposes everything created inside it
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Scope {
    id: NodeId,
}

/// Anything that holds a readable value: [`Signal`] and [`Memo`]
pub trait Read<T: 'static>: Copy {
    #[doc(hidden)]
    fn read_with<O>(&self, rt: &mut Runtime, track: bool, f: impl FnOnce(&T) -> O) -> Option<O>;
}
impl<T: 'static> Read<T> for Signal<T> {
    fn read_with<O>(&self, rt: &mut Runtime, track: bool, f: impl FnOnce(&T) -> O) -> Option<O> {
        rt.with_signal::<T, O>(self.id, track, f)
    }
}
impl<T: 'static> Read<T> for Memo<T> {
    fn read_with<O>(&self, rt: &mut Runtime, track: bool, f: impl FnOnce(&T) -> O) -> Option<O> {
        rt.with_memo::<T, O>(self.id, track, f)
    }
}

/// Anything that can be disposed
pub trait Disposable: Copy {
    #[doc(hidden)]
    fn node(&self) -> NodeId;
}
impl<T> Disposable for Signal<T> {
    fn node(&self) -> NodeId {
        self.id
    }
}
impl<T> Disposable for Memo<T> {
    fn node(&self) -> NodeId {
        self.id
    }
}
impl Disposable for Effect {
    fn node(&self) -> NodeId {
        self.id
    }
}
impl Disposable for Scope {
    fn node(&self) -> NodeId {
        self.id
    }
}

const DISPOSED: &str = "read of a disposed signal or memo";

fn memo_compute<T: 'static>(
    mut f: impl FnMut(&mut Runtime) -> T + 'static,
    eq: impl Fn(&T, &T) -> bool + 'static,
) -> Compute {
    Box::new(move |rt, slot| {
        let new = f(rt);
        match slot.as_mut().and_then(|b| b.downcast_mut::<T>()) {
            Some(old) if eq(old, &new) => false,
            Some(old) => {
                *old = new;
                true
            }
            None => {
                *slot = Some(Box::new(new) as Box<dyn Any>);
                true
            }
        }
    })
}

impl Runtime {
    pub fn signal<T: PartialEq + 'static>(&mut self, value: T) -> Signal<T> {
        Signal { id: self.create_signal(value, Box::new(|a: &T, b: &T| a == b)), _t: PhantomData }
    }

    /// Creates a signal owned by the runtime: nothing disposes it until the runtime goes
    pub fn signal_global<T: PartialEq + 'static>(&mut self, value: T) -> Signal<T> {
        Signal {
            id: self.create_signal_unowned(value, Box::new(|a: &T, b: &T| a == b)),
            _t: PhantomData,
        }
    }

    /// Creates a signal with a custom "same value" test; use it for types without `PartialEq` (`|_,
    /// _| false` makes every write a change)
    pub fn signal_with<T: 'static>(
        &mut self,
        value: T,
        eq: impl Fn(&T, &T) -> bool + 'static,
    ) -> Signal<T> {
        Signal { id: self.create_signal(value, Box::new(eq)), _t: PhantomData }
    }

    /// Creates a memo and computes it once
    pub fn memo<T: PartialEq + 'static>(
        &mut self,
        f: impl FnMut(&mut Runtime) -> T + 'static,
    ) -> Memo<T> {
        Memo { id: self.create_memo(memo_compute(f, |a: &T, b: &T| a == b)), _t: PhantomData }
    }

    /// Memo with a custom comparison
    pub fn memo_with<T: 'static>(
        &mut self,
        f: impl FnMut(&mut Runtime) -> T + 'static,
        eq: impl Fn(&T, &T) -> bool + 'static,
    ) -> Memo<T> {
        Memo { id: self.create_memo(memo_compute(f, eq)), _t: PhantomData }
    }

    /// Creates an effect and runs it once right away
    pub fn effect(&mut self, mut f: impl FnMut(&mut Runtime) + 'static) -> Effect {
        let compute: Compute = Box::new(move |rt, _| {
            f(rt);
            false
        });
        Effect { id: self.create_effect(compute) }
    }

    /// Runs `f` as the owner of everything it creates
    pub fn scope(&mut self, f: impl FnOnce(&mut Runtime)) -> Scope {
        Scope { id: self.create_scope(f) }
    }

    /// Like [`Runtime::scope`], but owned by `owner`: the new scope is disposed with
    /// `owner`, and survives re-runs of the current effect or memo.
    /// Panics if `owner` was disposed
    pub fn scope_in(&mut self, owner: Scope, f: impl FnOnce(&mut Runtime)) -> Scope {
        Scope { id: self.create_scope_in(owner.id, f) }
    }

    /// The effect, memo or scope that's running (the owner of what is created now), if any
    pub fn current_scope(&self) -> Option<Scope> {
        self.current_scope_id().map(|id| Scope { id })
    }

    /// Runs `f` as if inside `owner`: what it creates belongs to `owner` and `on_cleanup` attaches
    /// to it
    pub fn run_in<R>(&mut self, owner: Scope, f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
        self.run_in_owner(owner.id, f)
    }

    /// Whether the code running now is a handler run by [`run_in`](Self::run_in)
    pub fn in_handler(&self) -> bool {
        self.in_handler_now()
    }

    /// Whether an effect, memo or scope is running, so [`on_cleanup`](Self::on_cleanup) has an
    /// owner to attach to
    pub fn has_owner(&self) -> bool {
        self.has_owner_now()
    }

    /// Registers code to run when the current effect, memo or scope re-runs or is disposed. Panics
    /// outside of any of them
    pub fn on_cleanup(&mut self, f: impl FnOnce() + 'static) {
        self.register_cleanup(Box::new(move |_| f()));
    }

    /// [`on_cleanup`](Self::on_cleanup) for code that needs the runtime: it's handed it when the
    /// cleanup runs
    pub fn on_cleanup_rt(&mut self, f: impl FnOnce(&mut Runtime) + 'static) {
        self.register_cleanup(Box::new(f));
    }

    /// Hands `value` down to everything the current effect, memo or scope owns, now and later (an
    /// `if` branch rebuilt in a minute still finds it). Panics outside of any of them
    pub fn provide_context<T: Clone + 'static>(&mut self, value: T) {
        self.provide_context_dyn(std::any::TypeId::of::<T>(), Box::new(value));
    }

    /// The nearest value of type `T` handed down by an owner of whatever is running now, if any
    pub fn context<T: Clone + 'static>(&self) -> Option<T> {
        self.context_dyn(std::any::TypeId::of::<T>()).and_then(|v| v.downcast_ref::<T>()).cloned()
    }

    /// Reads a value (cloned) and tracks the read. Panics if the node was disposed
    pub fn get<T: Clone + 'static>(&mut self, r: impl Read<T>) -> T {
        self.try_get(r).expect(DISPOSED)
    }

    /// Like [`get`](Self::get) but `None` for a disposed node
    pub fn try_get<T: Clone + 'static>(&mut self, r: impl Read<T>) -> Option<T> {
        r.read_with(self, true, |v: &T| v.clone())
    }

    /// Reads without tracking
    pub fn peek<T: Clone + 'static>(&mut self, r: impl Read<T>) -> T {
        r.read_with(self, false, |v: &T| v.clone()).expect(DISPOSED)
    }

    pub fn with<T: 'static, O>(&mut self, r: impl Read<T>, f: impl FnOnce(&T) -> O) -> O {
        r.read_with(self, true, f).expect(DISPOSED)
    }

    /// Like [`with`](Self::with) but `None` for a disposed node
    pub fn try_with<T: 'static, O>(
        &mut self,
        r: impl Read<T>,
        f: impl FnOnce(&T) -> O,
    ) -> Option<O> {
        r.read_with(self, true, f)
    }

    pub fn set<T: 'static>(&mut self, s: Signal<T>, value: T) {
        self.try_set(s, value).expect("write to a disposed signal");
    }

    /// Like [`set`](Self::set); returns false for a disposed signal
    pub fn try_set<T: 'static>(&mut self, s: Signal<T>, value: T) -> Option<()> {
        self.write_signal(s.id, value)
    }

    /// Takes a signal's value out to work on it with the runtime free; `None` for a disposed signal
    pub fn take_signal<T: 'static>(&mut self, s: Signal<T>, write: bool) -> Option<TakenSignal> {
        self.take_signal_id(s.id, write)
    }

    /// Gives a value from [`take_signal`](Self::take_signal) back; `changed` tells what depends on
    /// it
    pub fn put_signal(&mut self, taken: TakenSignal, changed: bool) {
        self.put_signal_id(taken, changed)
    }

    /// Mutates a signal in place
    pub fn update<T: 'static>(&mut self, s: Signal<T>, f: impl FnOnce(&mut T)) {
        self.update_signal(s.id, f).expect("write to a disposed signal");
    }

    /// Like [`update`](Self::update); returns None for a disposed signal
    pub fn try_update<T: 'static>(&mut self, s: Signal<T>, f: impl FnOnce(&mut T)) -> Option<()> {
        self.update_signal(s.id, f)
    }

    /// Groups writes: effects run once, when the outermost batch ends
    pub fn batch<R>(&mut self, f: impl FnOnce(&mut Runtime) -> R) -> R {
        self.batch_impl(f)
    }

    /// Runs `f` without recording reads as dependencies
    pub fn untracked<R>(&mut self, f: impl FnOnce(&mut Runtime) -> R) -> R {
        self.untracked_impl(f)
    }

    /// Disposes a node and everything it owns
    pub fn dispose(&mut self, h: impl Disposable) -> bool {
        self.dispose_node(h.node())
    }

    /// Whether the handle still names a live node
    pub fn is_alive(&self, h: impl Disposable) -> bool {
        self.node_alive(h.node())
    }

    /// Number of live nodes (for leak checks in tests and diagnostics)
    pub fn node_count(&self) -> usize {
        self.live_nodes()
    }
}

impl<T: 'static> Signal<T> {
    pub fn get(self, rt: &mut Runtime) -> T
    where
        T: Clone,
    {
        rt.get(self)
    }
    pub fn set(self, rt: &mut Runtime, v: T) {
        rt.set(self, v)
    }
}

impl<T: 'static> Memo<T> {
    pub fn get(self, rt: &mut Runtime) -> T
    where
        T: Clone,
    {
        rt.get(self)
    }
}

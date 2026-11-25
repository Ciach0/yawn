# yawn-reactivity

Single-threaded, fine-grained reactivity core for Yawn: typed signals, memos, effects, ownership
scopes, batching and generational handles. No `unsafe` (`#![forbid(unsafe_code)]`), no dependencies.

```rust
use yawn_reactivity::Runtime;

let mut rt = Runtime::new();
let count = rt.signal(1);
let double = rt.memo(move |rt| rt.get(count) * 2);
rt.effect(move |rt| println!("double = {}", rt.get(double)));
rt.set(count, 2);                       // prints "double = 4"
rt.batch(|rt| { rt.set(count, 3); rt.set(count, 4); });   // one run, prints "double = 8"
```

## Guarantees

| Topic | Behaviour |
|---|---|
| Values | any `'static` type; cutoff via `PartialEq` or a custom comparison (`signal_with`, `memo_with`) |
| Handles | `Copy`, generational. A disposed node is never confused with a later node reusing its slot; `try_get`/`try_set` return `None`, `get`/`set` panic, `dispose` is idempotent; handles of another `Runtime` panic |
| Reads | always consistent, also inside a `batch` (memos validate on demand); dependencies are dynamic and visited in read order, so a gate is checked before what it guards |
| Effects | run at creation and after the writes that invalidate them (right after a lone `set`, at the end of the outermost `batch`); creation order within a pass; at most once per pass; only if a dependency changed value; effect writes trigger further passes; feedback loops panic after 10 000 passes |
| Memos | pure: writing a signal inside a memo panics; computed eagerly on creation, lazily afterwards |
| Ownership | nodes created inside a memo/effect/`scope` are disposed before it re-runs and when it is disposed; `on_cleanup` runs at the same moments. `scope_in(owner, f)` creates a scope owned by `owner` instead, so it survives re-runs of the current effect (keyed list rows) |
| Panics | a panic inside a computation poisons the runtime (the graph may be half-updated); later calls panic with that explanation |
| Cycles | detected, panic |
| Stack | validation uses an explicit stack: a 300 000-node chain works; dropping a deep graph is iterative |

Not supported (by design, for now): threads (`Runtime` is `!Send`), non-`'static` borrowed values,
async effects, reading a disposed node from a recomputation without handling `None`.

## Engine

The graph lives in the private `graph` module; only `Runtime`, the four handle types and the
`Read`/`Disposable` traits are public, so the engine can be replaced without touching user code.
`tests/semantics.rs` and the randomized model test `tests/random.rs` (gated memos, rewiring to
later-created nodes, dispose, batches, mid-batch reads, effects, plus `Runtime::debug_check`
invariants after every step) are the contract an engine has to pass.

**Current engine: rank-ordered push:**

* every memo has a topological rank (dependency < observer); a write queues observers by rank and
  propagation recomputes in rank order, queuing observers only when the value really changed. No
  marking pass over the downstream cone, so a cutoff right after the write costs O(1);
* ranks come from a hierarchical bitset with a small register/sorted buffer in front; new nodes
  get the top rank (bottom-up construction never reorders); a dynamic dependency that breaks the
  order is repaired with Pearce–Kelly; ranks are compacted after heavy dispose churn;
* memos nobody observes aren't recomputed eagerly (stale flag, recomputed on read);
* effects are outside the rank order: collected during propagation, run afterwards in creation
  order;
* column-wise slot storage with generations, edges mirrored on both sides (O(1) removal), up to two
  edges inline per node, reads recorded in a pooled buffer, unchanged dependency lists detected
  without touching the graph.

## Performance (before/after on the same public API)

`cargo run --release --example perf` (i5-12400F; median of three runs after, two before). Before =
v0.1.0 push-dirty/pull-validate engine. Raw runs are in `bench/results/` (`engine-v0.1-*.csv` before, `engine-*.csv` after);
`yawn-bench compare engine-v0.1-1.csv,engine-v0.1-2.csv engine-1.csv,engine-2.csv,engine-3.csv` (run from `bench/`) prints the table.

| scenario | before | after | ratio |
|---|---:|---:|---:|
| chain of 1 000 memos, write root | 60.9 µs | 21.6 µs | 0.35 |
| chain of 100 000 | 10.6 ms | 2.17 ms | 0.20 |
| 100 000-chain behind a cutoff memo | 4.8 ms | 29 ns | ~0 |
| 10 000 effects behind a parity memo | 774 µs | 183 µs | 0.24 |
| 1 000 effects on one signal | 65.5 µs | 18.6 µs | 0.28 |
| tree of 65 536 leaves, one leaf write | 2.32 µs | 1.01 µs | 0.44 |
| layered DAG 100×100, 3 deps | 1.23 ms | 0.58 ms | 0.47 |
| dynamic switch (10 000 memos) | 252 ns | 144 ns | 0.57 |
| hot subscription flips (10 000) | 284 ns | 143 ns | 0.50 |
| 5 000-dependency aggregate, one flag flip | 136 µs | 81 µs | 0.60 |
| batch of 64 writes into a DAG | 25.4 µs | 8.4 µs | 0.33 |
| write with one effect | 101 ns | 35 ns | 0.35 |
| write, nothing observing | 10.4 ns | 8.3 ns | 0.79 |
| read of a clean memo | 3.6 ns | 4.0 ns | 1.11 |
| create + dispose memo and effect | 289 ns | 226 ns | 0.78 |
| scope ring churn (dispose + create a 4-node scope) | 12.1 µs | 5.2 µs | 0.43 |
| memory: signal | 260 B | 172 B | 0.66 |
| memory: memo with one dependency | 290 B | 184 B | 0.63 |
| memory: memo in a chain | 312 B | 168 B | 0.54 |
| memory: effect | 282 B | 232 B | 0.82 |

Geometric mean of all 22 rows: 0.235 (about 4.3× faster). The only regression is the clean-memo
read (+11%): a read now first checks that no queued work is pending.

### Where the time goes

Measured on a chain (ns per recomputed node): `cargo run --release --example floor` (same value
representation as this crate, `Box<dyn Any>` slots and boxed `FnMut`, but no graph at all) takes
**5.0**; this crate takes **21.6**. About 5 ns of that is the generic-value/`dyn`-closure floor,
and the engine adds scheduling and tracking on top. The rest was not isolated by experiment. Likely
contributors, unmeasured: generational validation on every read, ownership/cleanup bookkeeping
around every run, `Option`-wrapped values moved in and out, poison handling and bounds-checked
indexing. A micro-optimisation round (inline hints, cheaper read path, removing `catch_unwind` as an
experiment) moved nothing measurable; inline two-edge adjacency lists helped trees, hot
subscriptions and chains (−10…−20%) and memory, at the cost of a few percent on very wide nodes.

## Tests

`cargo test` runs the semantic suite, the randomized model test (300 seeds × 600 steps) and the doctest.

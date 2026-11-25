# yawn-reactivity against other reactive libraries

Reactivity-only comparison of `yawn-reactivity` (path dependency, this repository) with the real
`dioxus-signals 0.7.10` (+ `dioxus-core`/`dioxus-hooks`/`generational-box` 0.7.10) and
`sycamore-reactive 0.9.4`. Prereleases (`dioxus 0.8.0-alpha`, `sycamore-reactive 0.10.0-beta`) weren't
run. No rendering, no DOM.

Run: `cargo run --release -- run results/frameworks-N.csv` (a few minutes); `--verify` only checks
equivalence.

## Method
* Each scenario is written once against the `Engine` trait (`src/engine.rs`) and runs on each library
  through its real public API, with the same graph and the same writes. Dioxus graphs are built inside
  a root component render (`use_effect` needs hook context), not part of timing.
* Equivalence check before timing: after each of 300 operations all libraries must expose the same
  observable values (FNV hash over effect outputs). All 15 scenarios pass.
* Timed operation = write(s) **plus everything needed to finish the work**. Yawn and Sycamore flush
  inside `set`; Dioxus op = `set` + `VirtualDom::process_events()` (memo tasks + effects). Without
  that call Dioxus hasn't propagated anything.
* Release, 3 warm-ups, 7 repetitions per run, median; two full runs, geometric mean of both shown.
  `Instant`, same machine (i5-12400F), same deterministic inputs (xorshift seeds).
* Counters: memo evaluations, effect runs, allocations per operation (counting allocator); they are
  in `results/frameworks-*.csv`.

## Results (2 runs combined)
Time per operation in ns, lower is better.

| scenario | yawn | dioxus | sycamore | slower: dioxus | slower: sycamore |
|---|---:|---:|---:|---:|---:|
| chain_1000 | 22 733 | 524 478 | 58 376 | 23.1x | 2.6x |
| chain_10000 | 256 613 | 5 979 010 | 832 574 | 23.3x | 3.2x |
| chain_cut_10000 | 32.6 | 500 | 388 964 | 15.3x | 11 913x |
| fanout_cut_10000 | 335 500 | 25 794 627 | 24 424 674 | 76.9x | 72.8x |
| effects_on_signal_1000 | 22 640 | 911 633 | 289 189 | 40.3x | 12.8x |
| write_one_effect | 43.8 | 595 | 64.5 | 13.6x | 1.5x |
| write_unobserved | 8.7 | 42.0 | 19.5 | 4.9x | 2.2x |
| tree_16384 | 796 | 14 023 | 1 946 | 17.6x | 2.4x |
| dag_dense_100x100 | 789 702 | 15 429 625 | 2 684 770 | 19.5x | 3.4x |
| dynamic_switch_10000 | 224 | 2 098 | 393 | 9.4x | 1.8x |
| hotsub_10000 | 56 746 | 961 880 | 337 507 | 17.0x | 5.9x |
| bigdeps_5000 | 97 382 | 1 134 369 | 164 217 | 11.6x | 1.7x |
| batch_64_writes_dag | 12 371 | 309 088 | 29 360 | 25.0x | 2.4x |
| read_clean_memo | 5.6 | 12.9 | 7.8 | 2.3x | 1.4x |
| read_signal | 5.1 | 6.8 | 7.6 | 1.3x | 1.5x |

Geometric mean of the time ratios: dioxus 13.4x slower than yawn, sycamore 5.7x (3.3x
without `chain_cut_10000`, where it's an outlier). No library beats yawn in any time scenario. Closest
is `read_signal`: dioxus 1.3x, sycamore 1.5x slower.

Memory (100 000 nodes, settled, live heap bytes per node, lower is better):
| node | yawn | dioxus | sycamore |
|---|---:|---:|---:|
| signal | 172 | 147 | 249 |
| memo, 1 dep | 184 | 939 | 276 |
| effect, 1 dep | 256 | 754 | 268 |
| memo + effect | 424 | 1830 | 565 |

Creation time (ns per node, 100 000 nodes):
| node | yawn | dioxus | sycamore |
|---|---:|---:|---:|
| signal | 72.5 | 152 | 87.0 |
| memo, 1 dep | 158 | 14 144 | 168 |
| effect, 1 dep | 174 | 14 305 | 108 |
| memo + effect | 343 | 54 183 | 410 |

Allocations per op: yawn up to 102 (`dag_dense`), sycamore up to 19 000 (`dag_dense`), dioxus up to 50 000
(`chain_10000`, one per task wake / effect queue).

## Caveats / non-comparable parts
* Recompute counts match for the chains and trees (same cutoff, same work). Small differences in
  `dag`, `hotsub`, `fanout` counters are artefacts: counters are averaged over a different number of
  extra ops per library (the op count scales with speed), and ops alternate state. Equality of outputs
  is proven by the verify pass, not by these averages.
* `chain_cut_10000`: all three recompute one memo, but sycamore marks the whole chain dirty on every
  write before it finds the cutoff, which is where its time goes.
* Sycamore memos are `create_selector` (skips the notification on an equal value), like the other
  libraries' memos; `create_memo` always notifies. Sycamore needs a root to be current: every write
  and read goes through `RootHandle::run_in`, a small constant cost that shows most in
  `read_signal` and `write_unobserved`.
* Dioxus has no batch API in dioxus-signals; the "batch" scenario does N writes then one
  `process_events`. Yawn uses `batch`, Sycamore uses `batch`. Each does one flush.
* Create/dispose can't be compared at node level: Dioxus has no per-node dispose, objects die with
  their scope; only creation cost and memory are compared (not in the geometric mean).
* Dioxus memos are eager through spawned tasks, effects go through the render scheduler queue; that
  machinery is the cost measured here, and is inherent to its design (whole-VirtualDom integration),
  not necessarily avoidable. Dioxus effects/memos are also meant to be used with rendering, which is
  out of scope.
* Timings carry some run-to-run noise (tiny absolute times such as `dynamic_switch`,
  `write_one_effect` vary by 15-50%); the ratios stay far from 1.
* `yawn-reactivity` wasn't changed for this benchmark.

# bench

The benchmark tool of Yawn. It's an independent Cargo workspace (Dioxus is a heavy dependency), so
run everything from this directory.

| command | does |
|---|---|
| `cargo run --release -- run [filter] [out.csv]` | `yawn-reactivity` against `dioxus-signals` and `sycamore-reactive` on 15 scenarios; results in [`results.md`](results.md) |
| `cargo run --release -- run --engines yawn,sycamore` | the same with only the listed libraries |
| `cargo run --release -- run --verify` | checks only that all libraries give the same observable results |
| `cargo run --release -- compare before.csv,... after.csv,...` | table of medians, ratios and the geometric mean from two sets of result files |
| `cargo run --release -p yawn-reactivity --example perf out.csv` | the engine-only scenarios, run from the repository root |
| [`browser/`](browser/README.md) | what the framework costs in a real browser: DOM updates, startup, size, leaks |

`compare` reads any CSV that starts with `scenario,value,...`, so it works on the output of
`perf` and on the files in [`results/`](results/):

```sh
cargo run --release -- compare \
  results/engine-v0.1-1.csv,results/engine-v0.1-2.csv \
  results/engine-1.csv,results/engine-2.csv,results/engine-3.csv
```

| files in `results/` | what |
|---|---|
| `engine-v0.1-*.csv` | `perf` on the first engine of `yawn-reactivity` |
| `engine-*.csv` | `perf` on the current engine |
| `frameworks-*.csv`, `frameworks-*_memory.csv` | `run` on every library, and the memory table |

All numbers are from one machine (i5-12400F), single thread. Timings are medians of repeated runs;
differences under a few percent are noise, and long chains vary by about ±30% between runs.

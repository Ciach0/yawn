# Browser benchmark

What the framework costs in a real browser: time to build and update the DOM, startup, size and
leaks. It builds one small app (`app/`) against a snapshot of the framework and drives it in
headless Chromium over the DevTools protocol. Needs Node 22+ (no packages), a `chromium` on the
`PATH` and the `yawn` CLI you want to test.

```sh
# a snapshot is a copy of the framework (crates/ and Cargo.toml) at the revision to measure
./run.sh before  /path/to/snapshot-a  /path/to/yawn-a  /tmp/bench  15
./run.sh after   /path/to/snapshot-b  /path/to/yawn-b  /tmp/bench  15
node compare.mjs /tmp/bench/before.json /tmp/bench/after.json     # markdown tables, ratios vs the first
```

Never run two `run.sh` at once with the same work directory, and don't build or test anything
else while it runs: the numbers are CPU timings.

## What's measured

Every number is the median of 15 runs (3 warm-up runs dropped), each on a page set up the same way.
`js` is the time until `click()` returns: all the work the framework does synchronously. `to paint`
is until the frame after the change was drawn, which is what the user waits for.

| scenario | what it shows |
|---|---|
| create / replace / append 1k rows, create 10k | building keyed rows: many small components, two dynamic texts and a class each |
| update every 10th row, select, swap, remove, clear | updates of existing rows through signals, keyed moves, disposal |
| 1 signal → 1000 text nodes | fan-out of a single signal |
| mount / unmount 200 static cards | a view that's mostly literals, like a landing page |
| startup (cache off) | time until the first click works, first contentful paint, wasm download |
| size | wasm and JS glue raw, gzip and brotli, server binary |
| memory | DOM nodes, listeners and JS heap after creating, clearing and 20 create/clear cycles (a leak shows as growth) |

Each run also checks the page afterwards (row counts, classes, text), so a variant that's fast
because it's wrong fails instead of winning.

Chromium's timer is coarse (0.1 ms) and a laptop is noisy: read differences under ~5 % as noise,
and look at `p25`/`p75` in the JSON before believing a single ratio.

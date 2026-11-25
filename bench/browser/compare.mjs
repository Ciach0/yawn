// node compare.mjs a.json b.json [c.json ...]  ->  markdown table, ratios against the first file
import fs from "node:fs";
const runs = process.argv.slice(2).map((f) => JSON.parse(fs.readFileSync(f, "utf8")));
const f = (v) => (v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2));
const kb = (n) => (n / 1024).toFixed(0) + " KB";
const row = (label, get, fmt = f, ratio = true) => {
  const base = get(runs[0]);
  return `| ${label} | ` + runs.map((r, i) => { const v = get(r); return i === 0 || !ratio || !base ? fmt(v) : `${fmt(v)} (${(v / base).toFixed(2)}x)`; }).join(" | ") + " |";
};
const head = `| | ${runs.map((r) => r.name).join(" | ")} |\n|---|${runs.map(() => "---").join("|")}|`;
console.log(`Chromium: ${runs[0].userAgent}; ${runs[0].runs} runs per scenario (median)\n`);
console.log("### Time in the browser, ms (lower is better)\n\n" + head);
for (const s of Object.keys(runs[0].scenarios)) {
  console.log(row(s + " (js)", (r) => r.scenarios[s].js.median));
  console.log(row(s + " (to paint)", (r) => r.scenarios[s].paint.median));
}
console.log("\n### Startup, ms (cache off)\n\n" + head);
for (const [k, l] of [["interactive_ms", "interactive (first click works)"], ["first_contentful_paint_ms", "first contentful paint"], ["dom_content_loaded_ms", "DOMContentLoaded"], ["wasm_downloaded_ms", "wasm downloaded"]]) if (runs[0].startup[k]) console.log(row(l, (r) => r.startup[k].median));
console.log("\n### Size\n\n" + head);
for (const k of ["raw", "gzip", "brotli"]) console.log(row(`wasm ${k}`, (r) => r.sizes.wasm[k], kb));
for (const k of ["raw", "gzip"]) console.log(row(`js glue ${k}`, (r) => r.sizes.js[k], kb));
console.log(row("server binary", (r) => r.sizes.server_binary, (n) => (n / 1048576).toFixed(1) + " MB"));
console.log("\n### Memory and leaks\n\n" + head);
for (const k of Object.keys(runs[0].memory)) console.log(row(k, (r) => r.memory[k], (v) => (Number.isInteger(v) ? String(v) : v.toFixed(2)), false));

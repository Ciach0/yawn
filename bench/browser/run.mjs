// Drives a built bench app in headless Chromium over the DevTools protocol and writes the numbers
// to a JSON file. No dependencies: Node 22+ has fetch and WebSocket.
//
//   node run.mjs --dist <dist dir> --bin <server binary> --name <label> --out <file.json>
//                [--runs 15] [--port 4170] [--cdp 9333] [--chromium chromium]
//
// Times are medians over `--runs` runs, each on a page that was set up the same way:
//   js    click() returns: everything the framework does synchronously
//   paint click() until the frame after it was drawn (rAF + a task), what the user waits for

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";

const arg = (k, d) => { const i = process.argv.indexOf("--" + k); return i > 0 ? process.argv[i + 1] : d; };
const dist = path.resolve(arg("dist"));
const bin = arg("bin");
const name = arg("name", "run");
const out = arg("out", `${name}.json`);
const RUNS = +arg("runs", 15);
const PORT = +arg("port", 4170);
const CDP = +arg("cdp", 9333);
const chromium = arg("chromium", "chromium");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const median = (a) => { const s = [...a].sort((x, y) => x - y); const m = s.length >> 1; return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2; };
const quant = (a, q) => { const s = [...a].sort((x, y) => x - y); return s[Math.min(s.length - 1, Math.floor(q * s.length))]; };
const stats = (a) => ({ median: median(a), p25: quant(a, 0.25), p75: quant(a, 0.75), min: Math.min(...a), n: a.length });

class Page {
  constructor(ws) { this.ws = ws; this.id = 0; this.pending = new Map(); this.events = []; ws.onmessage = (m) => {
    const d = JSON.parse(m.data);
    if (d.id) { const p = this.pending.get(d.id); this.pending.delete(d.id); d.error ? p.reject(new Error(JSON.stringify(d.error))) : p.resolve(d.result); }
    else this.events.push(d);
  }; }
  send(method, params = {}) { const id = ++this.id; this.ws.send(JSON.stringify({ id, method, params })); return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject })); }
  /** Errors the page logged or threw (a wasm panic shows up here). */
  errors() { return this.events.filter((e) => e.method === "Runtime.exceptionThrown" || (e.method === "Runtime.consoleAPICalled" && e.params.type === "error")).map((e) => JSON.stringify(e.params).slice(0, 300)); }
  async eval(expression) {
    const r = await this.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error("page: " + JSON.stringify(r.exceptionDetails.exception?.description ?? r.exceptionDetails));
    return r.result.value;
  }
}

async function open(url) {
  const t = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  const page = new Page(ws);
  page.targetId = t.id;
  await page.send("Network.enable");
  await page.send("Network.setCacheDisabled", { cacheDisabled: true });
  await page.send("Page.enable");
  await page.send("Runtime.enable");
  await page.send("HeapProfiler.enable");
  // remember a server-rendered node: if it is still the page's node after the app started, the client
  // adopted the server's markup (hydration) and did not render a second copy
  await page.send("Page.addScriptToEvaluateOnNewDocument", { source: `document.addEventListener("DOMContentLoaded", () => { window.__marked = document.getElementById("controls"); });` });
  if (url) await page.send("Page.navigate", { url });
  return page;
}
async function close(page) { page.ws.close(); await fetch(`http://127.0.0.1:${CDP}/json/close/${page.targetId}`).catch(() => {}); }

const READY = `new Promise((res) => {
  const t0 = performance.now();
  const tick = () => {
    const b = document.getElementById("ping"), v = document.getElementById("pingv");
    if (b && v) { b.click(); if (v.textContent !== "0") return res(performance.now()); }
    if (performance.now() - t0 > 20000) return res(-1);
    setTimeout(tick, 2);
  };
  tick();
})`;

const HELPERS = `
window.__settle = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
window.__step = async (id) => { document.getElementById(id).click(); await window.__settle(); };
window.__measure = async (id) => {
  const el = document.getElementById(id);
  const t0 = performance.now(); el.click(); const t1 = performance.now();
  await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
  return [t1 - t0, performance.now() - t0];
};
true`;

const SCENARIOS = [
  { name: "create 1k rows", setup: ["clear"], measure: "create1k", check: `document.querySelectorAll("#rows .row").length === 1000` },
  { name: "replace 1k rows", setup: ["clear", "create1k"], measure: "create1k", check: `document.querySelectorAll("#rows .row").length === 1000` },
  { name: "update every 10th row", setup: ["clear", "create1k"], measure: "update10th", check: `document.querySelectorAll("#rows .lbl")[10].textContent.endsWith("!!!") && !document.querySelectorAll("#rows .lbl")[11].textContent.endsWith("!!!")` },
  { name: "select row", setup: ["clear", "create1k"], measure: "select", check: `document.querySelectorAll("#rows .row.danger").length === 1` },
  { name: "swap rows", setup: ["clear", "create1k"], measure: "swap", check: `document.querySelectorAll("#rows .row").length === 1000` },
  { name: "remove row", setup: ["clear", "create1k"], measure: "remove", check: `document.querySelectorAll("#rows .row").length === 999` },
  { name: "append 1k rows", setup: ["clear", "create1k"], measure: "append1k", check: `document.querySelectorAll("#rows .row").length === 2000` },
  { name: "clear 1k rows", setup: ["clear", "create1k"], measure: "clear", check: `document.querySelectorAll("#rows .row").length === 0` },
  { name: "create 10k rows", setup: ["clear"], measure: "create10k", check: `document.querySelectorAll("#rows .row").length === 10000`, runs: Math.max(5, RUNS >> 1) },
  { name: "1 signal -> 1000 text nodes", setup: [], measure: "fan", check: `document.querySelector(".fan").textContent !== "0"` },
  { name: "mount 200 static cards", setup: [], measure: "cards", check: `document.querySelectorAll("#cards .card").length === 200`, after: ["cards"] },
  { name: "unmount 200 static cards", setup: ["cards"], measure: "cards", check: `document.querySelectorAll("#cards .card").length === 0` },
];

async function main() {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "yawn-bench-"));
  const server = spawn(path.join(dist, bin), [], { cwd: dist, env: { ...process.env, PORT: String(PORT) }, stdio: "ignore" });
  const browser = spawn(chromium, ["--headless=new", "--no-sandbox", "--disable-gpu", "--disable-extensions", "--disable-background-timer-throttling", "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows", "--window-size=1280,800", `--remote-debugging-port=${CDP}`, `--user-data-dir=${tmp}`, "about:blank"], { stdio: "ignore" });
  const cleanup = () => { try { server.kill(); } catch {} try { browser.kill(); } catch {} fs.rmSync(tmp, { recursive: true, force: true }); };
  process.on("exit", cleanup);
  for (let i = 0; ; i++) { try { await fetch(`http://127.0.0.1:${PORT}/`); await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { if (i > 100) throw new Error("server or browser did not start"); await sleep(100); } }
  const url = `http://127.0.0.1:${PORT}/`;
  const result = { name, date: new Date().toISOString(), runs: RUNS, userAgent: null, startup: {}, scenarios: {}, memory: {}, sizes: {} };
  result.userAgent = (await (await fetch(`http://127.0.0.1:${CDP}/json/version`)).json()).Browser;

  // sizes
  const pkg = path.join(dist, "public", "pkg");
  const sz = (f) => { const b = fs.readFileSync(f); return { raw: b.length, gzip: zlib.gzipSync(b, { level: 9 }).length, brotli: zlib.brotliCompressSync(b, { params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 } }).length }; };
  for (const f of fs.readdirSync(pkg).filter((f) => /\.[0-9a-f]{12}\./.test(f))) { if (f.endsWith(".wasm")) result.sizes.wasm = sz(path.join(pkg, f)); if (f.endsWith(".js")) result.sizes.js = sz(path.join(pkg, f)); }
  result.sizes.server_binary = fs.statSync(path.join(dist, bin)).size;
  result.sizes.html = (await (await fetch(url)).text()).length;

  // startup: a fresh tab each time, cache off
  const interactive = [], fcp = [], dcl = [], wasmFetch = [];
  for (let i = 0; i < RUNS; i++) {
    const p = await open(url);
    const t = await p.eval(READY);
    if (t < 0) throw new Error("the app never became interactive");
    if (p.errors().length) throw new Error("the page reported errors on startup: " + p.errors()[0]);
    interactive.push(t);
    if (!(await p.eval(`window.__marked !== undefined && window.__marked === document.getElementById("controls")`))) throw new Error("the client did not adopt the server-rendered page (hydration fell back to a fresh render)");
    const nav = JSON.parse(await p.eval(`JSON.stringify({ dcl: performance.getEntriesByType("navigation")[0].domContentLoadedEventEnd, fcp: (performance.getEntriesByType("paint").find((e) => e.name === "first-contentful-paint") || {}).startTime, wasm: (performance.getEntriesByType("resource").find((e) => e.name.endsWith(".wasm")) || {}).responseEnd })`));
    dcl.push(nav.dcl); if (nav.fcp) fcp.push(nav.fcp); if (nav.wasm) wasmFetch.push(nav.wasm);
    await close(p);
  }
  result.startup = { interactive_ms: stats(interactive), first_contentful_paint_ms: fcp.length ? stats(fcp) : null, dom_content_loaded_ms: stats(dcl), wasm_downloaded_ms: wasmFetch.length ? stats(wasmFetch) : null };

  // scenarios
  const p = await open(url);
  await p.eval(READY);
  await p.eval(HELPERS);
  if (p.errors().length) throw new Error("the page reported errors on startup: " + p.errors()[0]);
  const gc = () => p.send("HeapProfiler.collectGarbage");
  for (const sc of SCENARIOS) {
    const js = [], paint = [];
    const runs = sc.runs ?? RUNS;
    for (let i = 0; i < runs + 3; i++) {   // three warm-up runs are dropped
      if (sc.name.includes("1 signal")) { /* state carries over on purpose: the signal only grows */ }
      for (const s of sc.setup) await p.eval(`__step(${JSON.stringify(s)})`);
      await gc();
      const [a, b] = await p.eval(`__measure(${JSON.stringify(sc.measure)})`);
      if (!(await p.eval(sc.check))) throw new Error(`${sc.name}: the page is not in the expected state after run ${i}`);
      if (i >= 3) { js.push(a); paint.push(b); }
      for (const s of sc.after ?? []) await p.eval(`__step(${JSON.stringify(s)})`);
    }
    result.scenarios[sc.name] = { js: stats(js), paint: stats(paint) };
    console.error(`${name}: ${sc.name}: js ${median(js).toFixed(2)} ms, paint ${median(paint).toFixed(2)} ms`);
  }

  if (p.errors().length) throw new Error("the page reported errors during the scenarios: " + p.errors()[0]);
  // memory and leaks: node and listener counts, JS heap
  const counters = async () => { await gc(); const c = await p.send("Memory.getDOMCounters"); const h = await p.send("Runtime.getHeapUsage"); return { nodes: c.nodes, listeners: c.jsEventListeners, heap_mb: h.usedSize / 1048576 }; };
  await p.eval(`__step("clear")`);
  const empty = await counters();
  await p.eval(`__step("create1k")`);
  const full = await counters();
  await p.eval(`__step("clear")`);
  const cleared = await counters();
  for (let i = 0; i < 20; i++) { await p.eval(`__step("create1k")`); await p.eval(`__step("clear")`); }
  const churned = await counters();
  result.memory = {
    nodes_empty: empty.nodes, nodes_1k: full.nodes, nodes_after_clear: cleared.nodes, nodes_after_20_cycles: churned.nodes,
    listeners_empty: empty.listeners, listeners_1k: full.listeners, listeners_after_clear: cleared.listeners, listeners_after_20_cycles: churned.listeners,
    js_heap_mb_empty: empty.heap_mb, js_heap_mb_1k: full.heap_mb, js_heap_mb_after_20_cycles: churned.heap_mb,
  };
  await close(p);
  fs.writeFileSync(out, JSON.stringify(result, null, 2));
  console.error(`${name}: written ${out}`);
  cleanup();
  process.exit(0);
}
main().catch((e) => { console.error(e); process.exit(1); });

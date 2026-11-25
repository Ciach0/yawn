// Samples the CPU while one scenario runs many times and prints where the time went, by function.
//   node profile.mjs --dist <dist> --bin <binary> --scenario create1k [--setup clear] [--times 40]
// Wasm functions show as `wasm-function[N]` (the release build has no names); the glue functions
// (`__wbg_*`) are the calls into the DOM, so their share is the cost of crossing into the browser.

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const arg = (k, d) => { const i = process.argv.indexOf("--" + k); return i > 0 ? process.argv[i + 1] : d; };
const dist = path.resolve(arg("dist")), bin = arg("bin"), scenario = arg("scenario", "create1k");
const setup = arg("setup", "clear").split(",").filter(Boolean);
const TIMES = +arg("times", 40), PORT = +arg("port", 4171), CDP = +arg("cdp", 9334);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "yawn-prof-"));
const server = spawn(path.join(dist, bin), [], { cwd: dist, env: { ...process.env, PORT: String(PORT) }, stdio: "ignore" });
const browser = spawn("chromium", ["--headless=new", "--no-sandbox", "--disable-gpu", `--remote-debugging-port=${CDP}`, `--user-data-dir=${tmp}`, "about:blank"], { stdio: "ignore" });
const done = (code) => { try { server.kill(); browser.kill(); } catch {} fs.rmSync(tmp, { recursive: true, force: true }); process.exit(code); };
for (let i = 0; ; i++) { try { await fetch(`http://127.0.0.1:${PORT}/`); await fetch(`http://127.0.0.1:${CDP}/json/version`); break; } catch { if (i > 100) done(1); await sleep(100); } }
const t = await (await fetch(`http://127.0.0.1:${CDP}/json/new?about:blank`, { method: "PUT" })).json();
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d.result ?? d); pending.delete(d.id); } };
const send = (method, params = {}) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expression) => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result?.value;

await send("Page.enable"); await send("Profiler.enable");
await send("Page.navigate", { url: `http://127.0.0.1:${PORT}/` });
await ev(`new Promise((res) => { const tick = () => { const b = document.getElementById("ping"), v = document.getElementById("pingv"); if (b && v) { b.click(); if (v.textContent !== "0") return res(1); } setTimeout(tick, 5); }; tick(); })`);
await ev(`window.__settle = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0))); window.__step = async (id) => { document.getElementById(id).click(); await window.__settle(); }; true`);
await send("Profiler.setSamplingInterval", { interval: 50 });
// a few runs first so the JIT is warm, then profile only the measured clicks
for (let i = 0; i < 5; i++) { for (const s of setup) await ev(`__step(${JSON.stringify(s)})`); await ev(`__step(${JSON.stringify(scenario)})`); }
await send("Profiler.start");
let measured = 0;
for (let i = 0; i < TIMES; i++) {
  for (const s of setup) await ev(`__step(${JSON.stringify(s)})`);
  const a = await ev(`performance.now()`);
  await ev(`document.getElementById(${JSON.stringify(scenario)}).click(); true`);
  measured += (await ev(`performance.now()`)) - a;
}
const { profile } = await send("Profiler.stop");
const self = new Map(); const byId = new Map(profile.nodes.map((n) => [n.id, n]));
const dt = profile.timeDeltas; let total = 0;
profile.samples.forEach((sid, i) => { const n = byId.get(sid); const f = n.callFrame; const key = `${f.functionName || "(anonymous)"} ${f.url ? f.url.split("/").pop() : ""}`; self.set(key, (self.get(key) || 0) + dt[i]); total += dt[i]; });
const rows = [...self.entries()].sort((a, b) => b[1] - a[1]);
const idle = rows.filter(([k]) => k.startsWith("(idle)") || k.startsWith("(program)") || k.startsWith("(garbage")).reduce((s, [, v]) => s + v, 0);
console.log(`scenario ${scenario}: ${TIMES} runs, click time ${(measured / TIMES).toFixed(1)} ms each, sampled ${(total / 1000).toFixed(0)} ms (of which idle/program/gc ${(idle / 1000).toFixed(0)} ms)`);
for (const [k, v] of rows.slice(0, 30)) console.log(`${(v / 1000).toFixed(1).padStart(8)} ms  ${(100 * v / total).toFixed(1).padStart(5)}%  ${k}`);
done(0);

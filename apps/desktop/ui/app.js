// spacetrail desktop UI. Read-only: shows what fills the disk and what could be freed.
"use strict";

const tauri = window.__TAURI__;
const invoke = tauri ? tauri.core.invoke : mockInvoke;
const listen = tauri ? tauri.event.listen : () => Promise.resolve(() => {});

const $ = (id) => document.getElementById(id);
const el = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v);
  }
  for (const k of kids.flat()) if (k != null) e.append(k);
  return e;
};

// ───────────────────────── formatting ─────────────────────────
function bytes(n) {
  if (n == null) return "–";
  if (n < 1000) return `${n} B`;
  const u = ["kB", "MB", "GB", "TB"];
  let v = n, i = -1;
  do { v /= 1000; i++; } while (v >= 999.5 && i < u.length - 1);
  return `${v.toFixed(v < 10 ? 2 : v < 100 ? 1 : 0)} ${u[i]}`;
}
const pct = (a, b) => (b ? Math.round((a * 100) / b) : 0);
const tilde = (p) => p.replace(/^\/Users\/[^/]+/, "~");
const RISK = {
  safe: { icon: "✅", label: "Safe", meaning: "Regenerates automatically, no personal data" },
  review: { icon: "⚠️", label: "Review", meaning: "Regenerable but costly, or may hold something you want" },
  manual: { icon: "🔒", label: "Your call", meaning: "Your data or expensive assets: only you can decide" },
};
const PALETTE = ["#3b7dd8", "#e0782f", "#3e9d5b", "#a463b3", "#d4504f", "#2e9c9a", "#c9a227", "#8a6a55", "#d9709a", "#6b7280"];
const VOL_COLORS = { data: "#3b7dd8", vm: "#d4504f", system: "#6b7280", preboot: "#a463b3", recovery: "#c9a227", update: "#2e9c9a" };

function lightColor(hex) {
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  return 0.299 * r + 0.587 * g + 0.114 * b > 165;
}

// ───────────────────────── state ─────────────────────────
const state = {
  status: null,
  overview: null,
  report: null,
  stack: [], // treemap drill-down path (Folder objects)
  filter: "all",
  selected: new Set(),
};

function activity(text) {
  $("activity").hidden = !text;
  $("activity-text").textContent = text || "";
}

// ───────────────────────── hero ─────────────────────────
function totalFree() {
  const s = state.status;
  if (!s) return [0, 0];
  return s.container ? [s.container.total, s.container.free] : [s.volume.total, s.volume.free];
}

function level(free, total) {
  const p = (free * 100) / total;
  return p < 10 ? "crit" : p < 20 ? "warn" : "good";
}

function renderHero() {
  const [total, free] = totalFree();
  const used = total - free;
  const lvl = level(free, total);
  const color = { crit: "var(--crit)", warn: "var(--warn)", good: "var(--good)" }[lvl];
  const r = 50, c = 2 * Math.PI * r, frac = used / total;
  $("gauge").innerHTML =
    `<circle cx="60" cy="60" r="${r}" fill="none" stroke="var(--track)" stroke-width="14"/>` +
    `<circle cx="60" cy="60" r="${r}" fill="none" stroke="${color}" stroke-width="14" stroke-linecap="round" stroke-dasharray="${c * frac} ${c}"/>`;
  $("free-big").textContent = bytes(free);
  $("total").textContent = bytes(total);
  const pill = $("status-pill");
  pill.className = `pill ${lvl}`;
  pill.textContent = { crit: "● Almost full", warn: "● Getting full", good: "● Healthy" }[lvl] + ` · ${pct(used, total)}% used`;
  $("headline").textContent =
    lvl === "crit" ? `Only ${bytes(free)} left. Time to make room.` :
    lvl === "warn" ? `${bytes(free)} free. Worth a cleanup soon.` :
    `${bytes(free)} free. You're in good shape.`;
  renderSubline();

  const vols = (state.status.container?.volumes || []).filter((v) => v.consumed > 0);
  const bar = $("volumes"), legend = $("volumes-legend");
  bar.replaceChildren(); legend.replaceChildren();
  const label = (role) => typeof role === "string"
    ? { data: "Your data", vm: "Swap (memory)", system: "macOS", preboot: "Boot files", recovery: "Recovery", update: "Updates" }[role] || role
    : Object.values(role)[0];
  const explain = (role) => typeof role === "string" ? {
    data: "Your files, apps and caches. Everything below is about this part.",
    vm: "Memory paged out to disk. Grows when RAM is short; shrinks when you quit heavy apps or restart.",
    system: "macOS itself (sealed, read-only).",
    preboot: "Boot support for installed macOS versions.",
    recovery: "The Recovery system.",
    update: "Staged macOS updates.",
  }[role] || "" : "Other system volume.";
  const parts = [...vols.map((v) => ({ name: label(v.role), bytes: v.consumed, color: VOL_COLORS[v.role] || "#6b7280", tip: explain(v.role) })),
    { name: "Free", bytes: free, color: "transparent", tip: "Available space." }];
  for (const p of parts) {
    if (p.bytes / total < 0.002) continue;
    const seg = el("div", { style: `width:${(p.bytes * 100) / total}%;background:${p.color}` });
    tip(seg, () => `<b>${p.name}: ${bytes(p.bytes)}</b>${p.tip}`);
    bar.append(seg);
    legend.append(el("span", {}, el("i", { style: `background:${p.color === "transparent" ? "var(--track)" : p.color}` }), `${p.name} `, el("b", { text: bytes(p.bytes) })));
  }
}

function renderSubline() {
  const r = state.report;
  const [, free] = totalFree();
  $("subline").textContent = r
    ? `You can free ${bytes(r.totals.safe)} with no risk, and up to ${bytes(r.totals.safe + r.totals.review)} if you review a few things. That would leave ${bytes(free + r.totals.safe)}–${bytes(free + r.totals.safe + r.totals.review)} free.`
    : "Looking for things you can safely remove…";
}

// ───────────────────────── Full Disk Access ─────────────────────────
let fdaTimer = null;
function renderFda() {
  const granted = state.status?.full_disk_access?.status === "granted";
  $("fda-banner").hidden = granted;
}

async function openFda() {
  try { await invoke("open_full_disk_access_settings"); } catch (e) { console.error(e); }
  $("fda-title").textContent = "Switch on “spacetrail” in the list";
  $("fda-text").textContent = "In System Settings → Privacy & Security → Full Disk Access, turn on the switch next to spacetrail. This window will notice by itself.";
  const started = Date.now();
  clearInterval(fdaTimer);
  fdaTimer = setInterval(async () => {
    const s = await invoke("check_full_disk_access");
    if (s.status === "granted") {
      clearInterval(fdaTimer);
      const b = $("fda-banner");
      b.classList.add("ok");
      $("fda-title").textContent = "Full Disk Access granted";
      $("fda-text").textContent = "spacetrail can now see the whole disk. Rescanning…";
      $("fda-open").hidden = true; $("fda-restart").hidden = true;
      setTimeout(() => { b.hidden = true; b.classList.remove("ok"); loadAll(); }, 1800);
    } else if (Date.now() - started > 15000) {
      $("fda-restart").hidden = false;
      $("fda-text").textContent = "Already switched it on? macOS sometimes applies it only after a restart of the app.";
    }
  }, 2000);
}

// ───────────────────────── treemap ─────────────────────────
function squarify(nodes, x, y, w, h) {
  const total = nodes.reduce((s, n) => s + n.value, 0);
  if (!total || w <= 0 || h <= 0) return [];
  const scale = (w * h) / total;
  const items = nodes.map((n) => ({ n, area: n.value * scale }));
  const out = [];
  let row = [], rx = x, ry = y, rw = w, rh = h;
  const worst = (r, side) => {
    const s = r.reduce((a, b) => a + b.area, 0);
    let mx = 0, mn = Infinity;
    for (const it of r) { mx = Math.max(mx, it.area); mn = Math.min(mn, it.area); }
    return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
  };
  const lay = (r) => {
    const s = r.reduce((a, b) => a + b.area, 0);
    if (rw >= rh) {
      const cw = s / rh; let cy = ry;
      for (const it of r) { const hh = it.area / cw; out.push({ n: it.n, x: rx, y: cy, w: cw, h: hh }); cy += hh; }
      rx += cw; rw -= cw;
    } else {
      const ch = s / rw; let cx = rx;
      for (const it of r) { const ww = it.area / ch; out.push({ n: it.n, x: cx, y: ry, w: ww, h: ch }); cx += ww; }
      ry += ch; rh -= ch;
    }
  };
  for (const it of items) {
    const side = Math.min(rw, rh);
    if (!row.length || worst([...row, it], side) <= worst(row, side)) row.push(it);
    else { lay(row); row = [it]; }
  }
  if (row.length) lay(row);
  return out;
}

/// Bytes that safe + review suggestions could free inside `path`.
function reclaimable(path) {
  const r = state.report;
  if (!r) return 0;
  const prefix = path === "/" ? "/" : path + "/";
  let sum = 0;
  for (const s of r.suggestions) {
    if (s.risk === "manual") continue;
    for (const i of s.items) if (i.path === path || i.path.startsWith(prefix)) sum += i.bytes;
  }
  return sum;
}

/// Children to draw for a folder. A child holding more than half of the level
/// is replaced by its own children (e.g. /Users → /Users/me → ~/Library…), so
/// the first view shows meaningful blocks instead of one giant "Users".
function childrenOf(folder, expand = true) {
  // At the top level and after expanding, full paths avoid "Library" vs "~/Library" confusion.
  const pathLabels = folder.path === "/";
  let kids = folder.children.map((c) => ({ value: c.bytes, folder: c, label: pathLabels ? tilde(c.path) : c.name }));
  let other = folder.other;
  for (let round = 0; expand && round < 3; round++) {
    const big = kids.find((k) => k.folder && k.folder.children.length && k.value > folder.bytes * 0.5);
    if (!big) break;
    const f = big.folder;
    kids = kids.filter((k) => k !== big).concat(f.children.map((c) => ({ value: c.bytes, folder: c, label: tilde(c.path) })));
    other += f.other;
  }
  kids.sort((a, b) => b.value - a.value);
  kids.forEach((k, i) => (k.color = PALETTE[i % PALETTE.length]));
  if (other > 0) kids.push({ value: other, other: true, folder: null });
  return kids;
}

function renderTreemap() {
  const box = $("treemap");
  const ov = state.overview;
  if (!ov) return;
  const cur = state.stack[state.stack.length - 1];
  box.replaceChildren();
  const W = box.clientWidth, H = box.clientHeight;
  for (const r of squarify(childrenOf(cur), 0, 0, W, H)) {
    const f = r.n.folder;
    const cell = el("div", { class: "tm-cell", tabindex: f && f.children.length ? "0" : null });
    cell.style.cssText = `left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;background:${r.n.other ? "" : r.n.color}`;
    if (r.n.other) cell.classList.add("tm-other");
    else if (lightColor(r.n.color)) cell.classList.add("dark-text");
    if (f) {
      // A faint preview of what's inside.
      if (f.children.length && r.w > 60 && r.h > 50) {
        for (const s of squarify(childrenOf(f, false), 3, 3, r.w - 9, r.h - 9)) {
          if (s.w < 6 || s.h < 6) continue;
          cell.append(el("div", { class: "tm-sub", style: `left:${s.x}px;top:${s.y}px;width:${s.w}px;height:${s.h}px` }));
        }
      }
      const free = Math.min(reclaimable(f.path), f.bytes);
      if (free > f.bytes * 0.01) {
        cell.append(el("div", { class: "tm-hatch", style: `height:${Math.max(6, (free * 100) / f.bytes)}%` }));
      }
      if (f.children.length) {
        cell.classList.add("drill");
        const go = () => { state.stack.push(f); renderCrumbs(); renderTreemap(); hideTip(); };
        cell.addEventListener("click", go);
        cell.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); });
      }
      tip(cell, () => {
        const unread = f.state === "unreadable" ? "<br>⚠️ macOS hid this folder (size unknown)." : "";
        const fr = free > 0 ? `<br>Striped: about <b style="display:inline">${bytes(free)}</b> could be freed.` : "";
        return `<b>${tilde(f.path)}</b>${bytes(f.bytes)} · ${pct(f.bytes, state.overview.totals.alloc)}% of your data · ${f.files.toLocaleString()} files${fr}${unread}${f.children.length ? "<br>Click to look inside." : ""}`;
      });
    } else {
      tip(cell, () => `<b>Files and small folders</b>${bytes(r.n.value)} not shown individually.`);
    }
    if (r.w > 70 && r.h > 36) {
      cell.append(el("div", { class: "tm-name", text: f ? r.n.label : "Files & small folders" }), el("div", { class: "tm-size", text: bytes(r.n.value) }));
    }
    box.append(cell);
  }
}

function renderCrumbs() {
  const nav = $("crumbs");
  nav.replaceChildren();
  state.stack.forEach((f, i) => {
    if (i) nav.append(el("span", { text: "›" }));
    const last = i === state.stack.length - 1;
    nav.append(last
      ? el("span", { class: "here", text: i === 0 ? "Whole disk" : f.name })
      : el("button", { type: "button", text: i === 0 ? "Whole disk" : f.name, onclick: () => { state.stack.length = i + 1; renderCrumbs(); renderTreemap(); } }));
  });
}

function renderOverviewFoot() {
  const ov = state.overview;
  const foot = $("tm-foot");
  foot.replaceChildren(el("span", { text: `Scanned ${ov.totals.files.toLocaleString()} files in ${ov.elapsed_secs.toFixed(0)} s` }));
  if (ov.privacy_protected > 0) foot.append(el("span", { text: `🔐 ${ov.privacy_protected} folders hidden by macOS privacy (size unknown)` }));
  if (ov.reconciliation?.unattributed > 1e9) foot.append(el("span", { text: `${bytes(ov.reconciliation.unattributed)} of your data couldn't be measured` }));
  const list = $("files");
  list.replaceChildren(...ov.largest_files.slice(0, 15).map((f) => fileRow(f.path, f.bytes)));
  $("files-box").hidden = !ov.largest_files.length;
}

function fileRow(path, size, note) {
  return el("li", {},
    el("span", { class: "sz", text: bytes(size) }),
    el("span", { class: "path", title: path }, tilde(path), note ? el("span", { class: "note", text: note }) : null),
    el("button", { class: "btn tiny ghost", type: "button", text: "Show in Finder", onclick: () => invoke("reveal_in_finder", { path }).catch(alert) }));
}

// ───────────────────────── suggestions ─────────────────────────
function renderSuggestions() {
  const r = state.report;
  const box = $("suggestions");
  box.replaceChildren();
  const t = r.totals, all = t.safe + t.review + t.manual;

  const tabs = $("risk-tabs");
  tabs.replaceChildren();
  for (const [key, label, n] of [["all", "All", all], ["safe", "✅ Safe", t.safe], ["review", "⚠️ Review", t.review], ["manual", "🔒 Your call", t.manual]]) {
    tabs.append(el("button", { class: "tab", role: "tab", type: "button", "aria-selected": String(state.filter === key), text: `${label} · ${bytes(n)}`,
      onclick: () => { state.filter = key; renderSuggestions(); } }));
  }

  const rb = $("riskbar");
  rb.hidden = !all;
  rb.replaceChildren();
  for (const k of ["safe", "review", "manual"]) {
    if (!t[k]) continue;
    const seg = el("div", { class: `rb-${k}`, style: `width:${(t[k] * 100) / all}%`, text: `${RISK[k].icon} ${bytes(t[k])}` });
    tip(seg, () => `<b>${RISK[k].label}: ${bytes(t[k])}</b>${RISK[k].meaning}`);
    rb.append(seg);
  }

  for (const risk of ["safe", "review", "manual"]) {
    if (state.filter !== "all" && state.filter !== risk) continue;
    const group = r.suggestions.filter((s) => s.risk === risk);
    if (!group.length) continue;
    box.append(el("div", { class: "group-title" }, el("h3", { text: `${RISK[risk].icon} ${RISK[risk].label}` }), el("span", { class: "muted", text: RISK[risk].meaning })));
    for (const s of group) box.append(card(s));
  }
  if (!box.children.length) box.append(el("div", { class: "empty", text: "Nothing to show here." }));

  const adv = $("advice");
  adv.replaceChildren(...r.advice.map((a) => el("li", { text: a })));
  $("advice-card").hidden = !r.advice.length;
  updateSim();
}

function card(s) {
  const key = s.id;
  const check = el("input", { type: "checkbox", "aria-label": `Count ${s.name} in the total` });
  check.checked = state.selected.has(key);
  check.addEventListener("change", () => { check.checked ? state.selected.add(key) : state.selected.delete(key); updateSim(); });

  const body = el("div", { class: "sg-body" }, el("p", { class: "sg-desc", text: s.description }), el("div", { class: "how", text: `→ ${s.how.summary}` }));
  for (const c of s.how.commands || []) {
    const text = c.join(" ");
    const btn = el("button", { class: "btn tiny", type: "button", text: "Copy" });
    btn.addEventListener("click", async () => {
      try { await navigator.clipboard.writeText(text); btn.textContent = "Copied ✓"; setTimeout(() => (btn.textContent = "Copy"), 1500); } catch { /* clipboard unavailable */ }
    });
    body.append(el("div", { class: "cmd" }, el("code", { text }), btn));
  }
  if (s.how.instructions) body.append(el("p", { class: "regen", text: s.how.instructions }));
  if (s.regenerates) body.append(el("div", { class: "regen", text: `↺ ${s.regenerates}` }));
  if (s.notes?.length) body.append(el("ul", { class: "notes" }, s.notes.map((n) => el("li", { text: n }))));
  if (s.items.length) {
    const det = el("details", {}, el("summary", { text: s.items.length === 1 ? "Show location" : `Show ${s.items.length} items` }));
    det.append(el("ul", { class: "rows" }, s.items.slice(0, 60).map((i) => fileRow(i.path, i.bytes, i.note))));
    body.append(det);
  }

  const title = el("div", { class: "sg-title" }, el("h3", { text: s.name }), el("span", { class: `badge ${s.risk}`, text: `${RISK[s.risk].icon} ${RISK[s.risk].label}` }));
  for (const app of s.blocked_by || []) title.append(el("span", { class: "badge warn", text: `Close ${app} first` }));
  return el("article", { class: `sg ${s.risk}` },
    check,
    title,
    el("div", { class: "sg-size" }, bytes(s.bytes), s.exact ? null : el("span", { class: "approx", text: "up to" })),
    el("div", {}),
    body);
}

function updateSim() {
  const r = state.report;
  if (!r) return;
  const sel = r.suggestions.filter((s) => state.selected.has(s.id)).reduce((a, s) => a + s.bytes, 0);
  const [total, free] = totalFree();
  $("sim-selected").textContent = bytes(sel);
  $("sim-free").textContent = `${bytes(free + sel)} (${pct(free + sel, total)}%)`;
  $("simbar").hidden = sel === 0;
}

// ───────────────────────── tooltip ─────────────────────────
const tipEl = $("tooltip");
function tip(node, html) {
  node.addEventListener("mousemove", (e) => {
    tipEl.innerHTML = html();
    tipEl.hidden = false;
    const x = Math.min(e.clientX + 14, window.innerWidth - tipEl.offsetWidth - 8);
    const y = Math.min(e.clientY + 14, window.innerHeight - tipEl.offsetHeight - 8);
    tipEl.style.left = `${x}px`; tipEl.style.top = `${y}px`;
  });
  node.addEventListener("mouseleave", hideTip);
}
function hideTip() { tipEl.hidden = true; }

// ───────────────────────── loading ─────────────────────────
let loading = false;
async function loadAll() {
  if (loading) return;
  loading = true;
  $("rescan").disabled = true;
  try {
    state.status = await invoke("get_disk_status");
    renderHero(); renderFda();

    activity("Scanning your disk…");
    $("treemap").replaceChildren(el("div", { class: "loading" }, el("span", { class: "spinner" }), el("span", { id: "tm-progress", text: "Scanning your disk…" })));
    const overviewP = invoke("get_overview").then((ov) => {
      state.overview = ov; state.stack = [ov.tree];
      renderCrumbs(); renderTreemap(); renderOverviewFoot();
    });
    await overviewP;

    activity("Looking for things to free…");
    state.report = await invoke("get_suggestions");
    state.selected = new Set(state.report.suggestions.filter((s) => s.risk === "safe").map((s) => s.id));
    state.status = state.report.disk;
    renderHero(); renderSuggestions(); renderTreemap();
  } catch (e) {
    console.error(e);
    activity(`Something went wrong: ${e}`);
    return;
  } finally {
    loading = false;
    $("rescan").disabled = false;
  }
  activity("");
}

listen("scan-progress", (e) => {
  const p = $("tm-progress");
  if (p) p.textContent = `Scanning your disk… ${e.payload.files.toLocaleString()} files · ${bytes(e.payload.bytes)}`;
});
listen("suggest-progress", (e) => {
  activity(`${e.payload}…`);
  const p = $("sg-progress");
  if (p) p.textContent = `${e.payload}…`;
});

$("rescan").addEventListener("click", loadAll);
$("fda-open").addEventListener("click", openFda);
$("fda-restart").addEventListener("click", () => invoke("restart"));
let resizeT;
window.addEventListener("resize", () => { clearTimeout(resizeT); resizeT = setTimeout(renderTreemap, 120); });

loadAll();

// ───────────────────────── browser preview (no Tauri) ─────────────────────────
function mockInvoke(cmd) {
  const GB = 1e9;
  const f = (name, path, b, kids = [], state = "complete") =>
    ({ name, path, bytes: b, files: Math.round(b / 2e5), state, children: kids, other: Math.max(0, b - kids.reduce((s, k) => s + k.bytes, 0)) });
  const tree = f("Macintosh HD", "/", 458 * GB, [
    f("Users", "/Users", 375 * GB, [f("demo", "/Users/demo", 374 * GB, [
      f("Library", "/Users/demo/Library", 239 * GB, [
        f("Application Support", "/Users/demo/Library/Application Support", 147 * GB, [f("SomeGame", "/Users/demo/Library/Application Support/SomeGame", 73 * GB), f("Code", "/Users/demo/Library/Application Support/Code", 22 * GB, [f("logs", "/Users/demo/Library/Application Support/Code/logs", 20 * GB)])]),
        f("Containers", "/Users/demo/Library/Containers", 57 * GB, [f("com.docker.docker", "/Users/demo/Library/Containers/com.docker.docker", 57 * GB)]),
        f("Caches", "/Users/demo/Library/Caches", 22 * GB, [f("Yarn", "/Users/demo/Library/Caches/Yarn", 7.6 * GB)]),
      ]),
      f("Documents", "/Users/demo/Documents", 68 * GB, [f("dev", "/Users/demo/Documents/dev", 68 * GB)]),
      f(".nuget", "/Users/demo/.nuget", 7.4 * GB),
    ])]),
    f("Applications", "/Applications", 30 * GB), f("Library", "/Library", 15 * GB), f("System", "/System", 14 * GB), f("opt", "/opt", 11 * GB),
  ]);
  const sg = (id, name, risk, b, path, extra = {}) => ({ id, name, risk, bytes: b, exact: true, description: "Example description.", how: { summary: "Run the command below", commands: [["tool", "clean"]] }, items: [{ path, bytes: b }], notes: [], blocked_by: [], ...extra });
  const report = {
    disk: null, totals: { safe: 40.5 * GB, review: 88 * GB, manual: 168 * GB },
    suggestions: [
      sg("vscode-logs", "VS Code logs", "safe", 12.8 * GB, "/Users/demo/Library/Application Support/Code/logs", { regenerates: "New logs are written each session." }),
      sg("yarn", "Yarn cache", "safe", 7.6 * GB, "/Users/demo/Library/Caches/Yarn", { exact: false }),
      sg("wt", "Git worktrees", "review", 32.8 * GB, "/Users/demo/Documents/dev/repo/.claude/worktrees"),
      sg("docker", "Docker build cache", "review", 18.4 * GB, "/Users/demo/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw", { exact: false }),
      sg("game", "Large app data", "manual", 73 * GB, "/Users/demo/Library/Application Support/SomeGame"),
    ],
    advice: ["Swap uses 15 GB because memory is under pressure."], catalog_problems: [],
  };
  const status = { volume: { total: 494 * GB, free: 6.5 * GB }, full_disk_access: { status: "denied" },
    container: { total: 494 * GB, free: 6.5 * GB, volumes: [{ role: "data", consumed: 446 * GB }, { role: "vm", consumed: 15 * GB }, { role: "system", consumed: 12.6 * GB }, { role: "preboot", consumed: 10.8 * GB }, { role: "recovery", consumed: 2.6 * GB }] } };
  report.disk = status;
  const delay = (v, ms) => new Promise((r) => setTimeout(() => r(v), ms));
  switch (cmd) {
    case "get_disk_status": return delay(status, 50);
    case "get_overview": return delay({ elapsed_secs: 19, totals: { alloc: 458 * GB, files: 3561390 }, reconciliation: null, privacy_protected: 518, other_owner: 148, tree, largest_files: [{ path: "/Users/demo/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw", bytes: 56.9 * GB }] }, 300);
    case "get_suggestions": return delay(report, 300);
    case "check_full_disk_access": return delay({ status: "denied" }, 50);
    default: return delay(null, 10);
  }
}

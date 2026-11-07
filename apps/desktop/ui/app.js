// spacetrail desktop UI: what fills the disk, what could be freed, and safe removal to the Trash.
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
    else if (k === "html") e.innerHTML = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v);
  }
  for (const k of kids.flat()) if (k != null) e.append(k);
  return e;
};

// ───────────────────────── icons (inline SVG, currentColor) ─────────────────────────
const ICON = {
  sparkles: '<path d="M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6z"/><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"/><path d="M5 2l.6 1.4L7 4l-1.4.6L5 6l-.6-1.4L3 4l1.4-.6z"/>',
  finder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><circle cx="12" cy="13" r="2.6"/><path d="M14 15l2 2"/>',
  trash: '<path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="M6 7l1 13h10l1-13"/><path d="M10 11v6M14 11v6"/>',
  refresh: '<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h8"/>',
  check: '<path d="M5 12l5 5L20 7"/>',
  chevron: '<path d="M6 9l6 6 6-6"/>',
  close: '<path d="M6 6l12 12M18 6L6 18"/>',
};
/// Escapes text for the few places that build HTML strings (tooltips). File names come from disk.
const esc = (t) => String(t).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const svg = (name) => `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICON[name]}</svg>`;
const iconBtn = (name, tipText, onclick, cls = "") =>
  el("button", { class: `icon-btn ${cls}`, type: "button", "data-tip": tipText, "aria-label": tipText, html: svg(name), onclick });

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
const baseName = (p) => (p === "/" ? "Macintosh HD" : p.split("/").filter(Boolean).pop());
function ago(unix) {
  if (!unix) return "unknown date";
  const days = Math.floor((Date.now() / 1000 - unix) / 86400);
  if (days < 1) return "today";
  if (days < 60) return `${days} days ago`;
  if (days < 730) return `${Math.round(days / 30)} months ago`;
  return `${(days / 365).toFixed(1)} years ago`;
}
const RISK = {
  safe: { icon: "✅", label: "Safe", meaning: "regenerates automatically, no personal data" },
  review: { icon: "⚠️", label: "Review", meaning: "regenerable but costly, or may hold something you want" },
  manual: { icon: "🔒", label: "Your call", meaning: "your data or expensive assets" },
};
const PALETTE = ["#3b7dd8", "#e0782f", "#3e9d5b", "#a463b3", "#d4504f", "#2e9c9a", "#c9a227", "#8a6a55", "#d9709a", "#6b7280"];
const VOL_COLORS = { data: "#3b7dd8", vm: "#d4504f", system: "#6b7280", preboot: "#a463b3", recovery: "#c9a227", update: "#2e9c9a" };
const lightColor = (hex) => {
  const n = parseInt(hex.slice(1), 16);
  return 0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255) > 165;
};

// ───────────────────────── state ─────────────────────────
const state = { status: null, overview: null, report: null, stack: [], view: "map", filter: "all", selected: new Set(), open: new Set() };
const activity = (text) => { $("activity").hidden = !text; $("activity-text").textContent = text || ""; };

// ───────────────────────── tooltips ─────────────────────────
const tipEl = $("tooltip");
let richTip = null;
function placeTip(e) {
  const x = Math.min(e.clientX + 14, window.innerWidth - tipEl.offsetWidth - 8);
  const y = Math.min(e.clientY + 16, window.innerHeight - tipEl.offsetHeight - 8);
  tipEl.style.left = `${x}px`; tipEl.style.top = `${y}px`;
}
document.addEventListener("mouseover", (e) => {
  const t = e.target.closest("[data-tip]");
  if (!t || richTip) return;
  tipEl.textContent = t.getAttribute("data-tip");
  tipEl.hidden = false; placeTip(e);
});
document.addEventListener("mousemove", (e) => { if (!tipEl.hidden) placeTip(e); });
document.addEventListener("mouseout", (e) => { if (e.target.closest("[data-tip]") && !richTip) hideTip(); });
/// Rich HTML tooltip for treemap cells and bars.
function tip(node, html) {
  node.addEventListener("mousemove", (e) => {
    if (e.target.closest("[data-tip]")) return; // a button inside shows its own tooltip
    richTip = node; tipEl.innerHTML = html(); tipEl.hidden = false; placeTip(e);
  });
  node.addEventListener("mouseleave", () => { richTip = null; hideTip(); });
}
function hideTip() { tipEl.hidden = true; }

// ───────────────────────── views ─────────────────────────
function showView(v) {
  state.view = v;
  for (const b of document.querySelectorAll(".vtab")) b.setAttribute("aria-selected", String(b.dataset.view === v));
  for (const id of ["map", "free", "files", "tips"]) $(`view-${id}`).hidden = id !== v;
  if (v === "map") renderTreemap();
  updateSim();
}
for (const b of document.querySelectorAll(".vtab")) b.addEventListener("click", () => showView(b.dataset.view));

// ───────────────────────── summary ─────────────────────────
function totalFree() {
  const s = state.status;
  if (!s) return [0, 0];
  return s.container ? [s.container.total, s.container.free] : [s.volume.total, s.volume.free];
}
const level = (free, total) => { const p = (free * 100) / total; return p < 10 ? "crit" : p < 20 ? "warn" : "good"; };

function renderSummary() {
  const [total, free] = totalFree();
  const used = total - free, lvl = level(free, total);
  const color = { crit: "var(--crit)", warn: "var(--warn)", good: "var(--good)" }[lvl];
  const r = 50, c = 2 * Math.PI * r;
  $("gauge").innerHTML = `<circle cx="60" cy="60" r="${r}" fill="none" stroke="var(--track)" stroke-width="14"/><circle cx="60" cy="60" r="${r}" fill="none" stroke="${color}" stroke-width="14" stroke-linecap="round" stroke-dasharray="${(c * used) / total} ${c}"/>`;
  $("free-big").textContent = bytes(free);
  $("total").textContent = bytes(total);
  const pill = $("status-pill");
  pill.className = `pill ${lvl}`;
  pill.textContent = `${{ crit: "● Almost full", warn: "● Getting full", good: "● Healthy" }[lvl]} · ${pct(used, total)}% used`;
  $("headline").textContent = lvl === "crit" ? `Only ${bytes(free)} left` : lvl === "warn" ? `${bytes(free)} free, worth a cleanup` : `${bytes(free)} free, all good`;
  const rep = state.report;
  $("subline").textContent = rep
    ? `Free ${bytes(rep.totals.safe)} with no risk, up to ${bytes(rep.totals.safe + rep.totals.review)} if you review a few things.`
    : "Looking for things you can safely remove…";

  const vols = (state.status.container?.volumes || []).filter((v) => v.consumed > 0);
  const label = (role) => typeof role === "string" ? ({ data: "Your data", vm: "Swap", system: "macOS", preboot: "Boot", recovery: "Recovery", update: "Updates" }[role] || role) : Object.values(role)[0];
  const explain = (role) => typeof role === "string" ? ({
    data: "Your files, apps and caches: everything in the map below.",
    vm: "Memory paged out to disk. Grows when RAM is short; shrinks when you quit heavy apps or restart.",
    system: "macOS itself (sealed, read-only).", preboot: "Boot support for installed macOS versions.",
    recovery: "The Recovery system.", update: "Staged macOS updates.",
  }[role] || "") : "Other system volume.";
  const parts = [...vols.map((v) => ({ name: label(v.role), bytes: v.consumed, color: VOL_COLORS[v.role] || "#6b7280", tip: explain(v.role) })),
    { name: "Free", bytes: free, color: "transparent", tip: "Available space." }];
  const bar = $("volumes"), legend = $("volumes-legend");
  bar.replaceChildren(); legend.replaceChildren();
  for (const p of parts) {
    if (p.bytes / total < 0.002) continue;
    const seg = el("div", { style: `width:${(p.bytes * 100) / total}%;background:${p.color}` });
    tip(seg, () => `<b>${p.name}: ${bytes(p.bytes)}</b>${p.tip}`);
    bar.append(seg);
    legend.append(el("span", {}, el("i", { style: `background:${p.color === "transparent" ? "var(--track)" : p.color}` }), `${p.name} `, el("b", { text: bytes(p.bytes) })));
  }
}

// ───────────────────────── Full Disk Access ─────────────────────────
let fdaTimer = null;
const renderFda = () => { $("fda-banner").hidden = state.status?.full_disk_access?.status === "granted"; };
async function openFda() {
  try { await invoke("open_full_disk_access_settings"); } catch (e) { console.error(e); }
  $("fda-title").textContent = "Switch on “spacetrail” in the list";
  $("fda-text").textContent = "System Settings → Privacy & Security → Full Disk Access: turn on spacetrail. This window notices by itself.";
  const started = Date.now();
  clearInterval(fdaTimer);
  fdaTimer = setInterval(async () => {
    const s = await invoke("check_full_disk_access");
    if (s.status === "granted") {
      clearInterval(fdaTimer);
      $("fda-banner").classList.add("ok");
      $("fda-title").textContent = "Full Disk Access granted";
      $("fda-text").textContent = "spacetrail can now see the whole disk. Rescanning…";
      $("fda-open").hidden = true; $("fda-restart").hidden = true;
      setTimeout(() => { $("fda-banner").hidden = true; loadAll(); }, 1600);
    } else if (Date.now() - started > 15000) {
      $("fda-restart").hidden = false;
      $("fda-text").textContent = "Already switched it on? macOS sometimes applies it only after restarting the app.";
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
    if (rw >= rh) { const cw = s / rh; let cy = ry; for (const it of r) { const hh = it.area / cw; out.push({ n: it.n, x: rx, y: cy, w: cw, h: hh }); cy += hh; } rx += cw; rw -= cw; }
    else { const ch = s / rw; let cx = rx; for (const it of r) { const ww = it.area / ch; out.push({ n: it.n, x: cx, y: ry, w: ww, h: ch }); cx += ww; } ry += ch; rh -= ch; }
  };
  for (const it of items) {
    const side = Math.min(rw, rh);
    if (!row.length || worst([...row, it], side) <= worst(row, side)) row.push(it);
    else { lay(row); row = [it]; }
  }
  if (row.length) lay(row);
  return out;
}

function reclaimable(path) {
  const r = state.report;
  if (!r) return 0;
  const prefix = path === "/" ? "/" : `${path}/`;
  let sum = 0;
  for (const s of r.suggestions) {
    if (s.risk === "manual") continue;
    for (const i of s.items) if (i.path === path || i.path.startsWith(prefix)) sum += i.bytes;
  }
  return sum;
}

/// Children to draw. A child with more than half of the level is replaced by
/// its own children, so the first view shows ~/Library, ~/Documents… not "Users".
function childrenOf(folder, expand = true) {
  const pathLabels = folder.path === "/";
  let kids = folder.children.map((c) => ({ value: c.bytes, folder: c, label: pathLabels ? tilde(c.path) : c.name }));
  let other = folder.other;
  for (let round = 0; expand && round < 3; round++) {
    const big = kids.find((k) => k.folder && k.folder.children.length && k.value > folder.bytes * 0.5);
    if (!big) break;
    kids = kids.filter((k) => k !== big).concat(big.folder.children.map((c) => ({ value: c.bytes, folder: c, label: tilde(c.path) })));
    other += big.folder.other;
  }
  kids.sort((a, b) => b.value - a.value);
  kids.forEach((k, i) => (k.color = PALETTE[i % PALETTE.length]));
  if (other > 0) kids.push({ value: other, other: true, folder: null });
  return kids;
}

function renderTreemap() {
  const box = $("treemap"), ov = state.overview;
  if (!ov || state.view !== "map") return;
  const cur = state.stack[state.stack.length - 1];
  box.replaceChildren();
  for (const r of squarify(childrenOf(cur), 0, 0, box.clientWidth, box.clientHeight)) {
    const f = r.n.folder;
    const cell = el("div", { class: "tm-cell", tabindex: f && f.children.length ? "0" : null });
    cell.style.cssText = `left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;background:${r.n.other ? "" : r.n.color}`;
    if (r.n.other) cell.classList.add("tm-other");
    else if (lightColor(r.n.color)) cell.classList.add("dark-text");
    if (f) {
      if (f.children.length && r.w > 60 && r.h > 50) {
        for (const s of squarify(childrenOf(f, false), 3, 3, r.w - 9, r.h - 9)) {
          if (s.w >= 6 && s.h >= 6) cell.append(el("div", { class: "tm-sub", style: `left:${s.x}px;top:${s.y}px;width:${s.w}px;height:${s.h}px` }));
        }
      }
      const free = Math.min(reclaimable(f.path), f.bytes);
      if (free > f.bytes * 0.01) cell.append(el("div", { class: "tm-hatch", style: `height:${Math.max(6, (free * 100) / f.bytes)}%` }));
      if (f.children.length) {
        cell.classList.add("drill");
        const go = () => { state.stack.push(f); renderCrumbs(); renderTreemap(); hideTip(); };
        cell.addEventListener("click", go);
        cell.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); });
      }
      if (r.w > 64 && r.h > 40) {
        const ask = el("button", { class: "tm-ask", type: "button", "aria-label": "Explain with AI", "data-tip": "Explain with AI", html: svg("sparkles") });
        ask.addEventListener("click", (e) => { e.stopPropagation(); hideTip(); explainItem(f.path, { bytes: f.bytes }); });
        cell.append(ask);
      }
      tip(cell, () => {
        const fr = free > 0 ? `<br>Striped: about ${bytes(free)} could be freed.` : "";
        const hidden = f.state === "unreadable" ? "<br>🔐 macOS hid this folder (size unknown)." : "";
        return `<b>${esc(tilde(f.path))}</b>${bytes(f.bytes)} · ${pct(f.bytes, ov.totals.alloc)}% of your data · ${f.files.toLocaleString()} files${fr}${hidden}${f.children.length ? "<br>Click to look inside" : ""}`;
      });
    } else {
      tip(cell, () => `<b>Files and small folders</b>${bytes(r.n.value)} not shown individually.`);
    }
    if (r.w > 70 && r.h > 36) cell.append(el("div", { class: "tm-name", text: f ? r.n.label : "Files & small folders" }), el("div", { class: "tm-size", text: bytes(r.n.value) }));
    box.append(cell);
  }
}

function renderCrumbs() {
  const nav = $("crumbs");
  nav.replaceChildren();
  state.stack.forEach((f, i) => {
    if (i) nav.append(el("span", { text: "›" }));
    const name = i === 0 ? "Whole disk" : f.name;
    nav.append(i === state.stack.length - 1
      ? el("span", { class: "here", text: name })
      : el("button", { type: "button", text: name, onclick: () => { state.stack.length = i + 1; renderCrumbs(); renderTreemap(); } }));
  });
}

function renderMapFoot() {
  const ov = state.overview, foot = $("tm-foot");
  foot.replaceChildren(el("span", { text: `Scanned ${ov.totals.files.toLocaleString()} files in ${ov.elapsed_secs.toFixed(0)} s` }));
  if (ov.privacy_protected > 0) foot.append(el("span", { text: `🔐 ${ov.privacy_protected} folders hidden by macOS privacy` }));
  if (ov.reconciliation?.unattributed > 1e9) foot.append(el("span", { text: `${bytes(ov.reconciliation.unattributed)} couldn't be measured` }));
}

// ───────────────────────── rows (files & items) ─────────────────────────
function itemRow({ path, size, note, hint, canDelete, source }) {
  const actions = el("div", { class: "actions" },
    iconBtn("sparkles", "Explain with AI: what is it, is it safe to delete?", () => explainItem(path, { bytes: size, spacetrail_note: hint || null }), "ai"),
    iconBtn("finder", "Show in Finder", () => invoke("reveal_in_finder", { path }).catch((e) => toast(String(e)))));
  if (canDelete) actions.append(iconBtn("trash", "Move to Trash…", () => confirmDelete({ path, size, source }), "del"));
  return el("li", { "data-path": path },
    el("span", { class: "sz", text: bytes(size) }),
    el("span", { class: "path", title: path }, tilde(path), note ? el("span", { class: "note", text: note }) : null),
    actions);
}

const SYSTEM_PREFIXES = ["/System/", "/Library/", "/private/", "/usr/", "/bin/", "/sbin/", "/cores/", "/opt/homebrew/Cellar/"];
const isSystem = (p) => SYSTEM_PREFIXES.some((x) => p.startsWith(x)) || /\.(app|framework|bundle|kext)\//.test(p);
let filesTab = "largest";
function renderFiles() {
  const ov = state.overview;
  if (!ov) return;
  const hide = $("hide-system").checked;
  try { localStorage.setItem("hideSystem", hide ? "1" : "0"); } catch { /* storage unavailable */ }
  const src = filesTab === "largest" ? ov.largest_files : ov.oldest_files;
  const rows = src.filter((f) => !hide || !isSystem(f.path)).slice(0, 25);
  $("tab-largest").setAttribute("aria-checked", String(filesTab === "largest"));
  $("tab-oldest").setAttribute("aria-checked", String(filesTab === "oldest"));
  $("files-hint").textContent = filesTab === "oldest"
    ? "Files of 5 MB or more that haven't changed in the longest time: good candidates to review."
    : "The biggest single files on your disk.";
  $("files").replaceChildren(...rows.map((f) => itemRow({ path: f.path, size: f.bytes, canDelete: true, source: `${filesTab} files`, note: `modified ${ago(f.modified)}` })));
  if (!rows.length) $("files").append(el("li", { class: "empty", text: "Nothing to show with the current filter." }));
}
$("tab-largest").addEventListener("click", () => { filesTab = "largest"; renderFiles(); });
$("tab-oldest").addEventListener("click", () => { filesTab = "oldest"; renderFiles(); });
$("hide-system").addEventListener("change", renderFiles);
try { $("hide-system").checked = localStorage.getItem("hideSystem") !== "0"; } catch { /* storage unavailable */ }

// ───────────────────────── suggestions ─────────────────────────
function renderSuggestions() {
  const r = state.report, box = $("suggestions");
  box.replaceChildren();
  const t = r.totals, all = t.safe + t.review + t.manual;
  $("count-free").textContent = bytes(t.safe + t.review);

  $("risk-tabs").replaceChildren(...[["all", "All", all], ["safe", "✅ Safe", t.safe], ["review", "⚠️ Review", t.review], ["manual", "🔒 Your call", t.manual]].map(([key, label, n]) =>
    el("button", { class: "chip", role: "tab", type: "button", "aria-selected": String(state.filter === key), text: `${label} · ${bytes(n)}`, onclick: () => { state.filter = key; renderSuggestions(); } })));
  const rb = $("riskbar");
  rb.hidden = !all;
  rb.replaceChildren(...["safe", "review", "manual"].filter((k) => t[k]).map((k) => {
    const seg = el("div", { class: `rb-${k}`, style: `width:${(t[k] * 100) / all}%` });
    tip(seg, () => `<b>${RISK[k].icon} ${RISK[k].label}: ${bytes(t[k])}</b>${RISK[k].meaning}`);
    return seg;
  }));

  for (const risk of ["safe", "review", "manual"]) {
    if (state.filter !== "all" && state.filter !== risk) continue;
    const group = r.suggestions.filter((s) => s.risk === risk);
    if (!group.length) continue;
    box.append(el("div", { class: "group-title" }, el("h3", { text: `${RISK[risk].icon} ${RISK[risk].label}` }), el("span", { class: "muted", text: RISK[risk].meaning }), el("span", { class: "gsum", text: bytes(t[risk]) })));
    for (const s of group) box.append(suggestionRow(s));
  }
  if (!box.children.length) box.append(el("div", { class: "loading", text: "Nothing to show here." }));
  $("advice").replaceChildren(...r.advice.map((a) => el("li", { text: a })));
  $("count-tips").textContent = r.advice.length || "";
  updateSim();
}

function suggestionRow(s) {
  const open = state.open.has(s.id);
  const check = el("input", { type: "checkbox", "aria-label": `Count ${s.name} in the total`, "data-tip": "Include in the “you'd have” total" });
  check.checked = state.selected.has(s.id);
  check.addEventListener("change", () => { if (check.checked) state.selected.add(s.id); else state.selected.delete(s.id); updateSim(); });
  const head = el("button", { class: "sg-head", type: "button", "aria-expanded": String(open), "data-tip": open ? "Hide details" : "Show details and how to free it" },
    el("span", { class: "sg-name", text: s.name }),
    el("span", { class: "sg-meta", text: s.items.length > 1 ? `${s.items.length} items` : "" }),
    ...(s.blocked_by || []).map((a) => el("span", { class: "badge warn", text: `close ${a}` })));
  const chev = el("button", { class: "icon-btn chev", type: "button", "aria-label": "Details", html: svg("chevron") });
  const art = el("article", { class: `sg ${s.risk}${open ? " open" : ""}` },
    el("div", { class: "sg-row" }, check, head,
      el("span", { class: "sg-size" }, s.exact ? null : el("span", { class: "approx", text: "up to" }), bytes(s.bytes)), chev));
  const toggle = () => {
    hideTip();
    if (state.open.has(s.id)) state.open.delete(s.id); else state.open.add(s.id);
    art.replaceWith(suggestionRow(s));
  };
  head.addEventListener("click", toggle);
  chev.addEventListener("click", toggle);
  if (open) art.append(suggestionDetail(s));
  return art;
}

function suggestionDetail(s) {
  const d = el("div", { class: "sg-detail" }, el("p", { class: "muted", text: s.description }), el("div", { class: "how", text: `→ ${s.how.summary}` }));
  for (const c of s.how.commands || []) {
    const text = c.join(" ");
    const btn = iconBtn("copy", "Copy command", async () => {
      try { await navigator.clipboard.writeText(text); btn.innerHTML = svg("check"); setTimeout(() => (btn.innerHTML = svg("copy")), 1400); } catch { /* clipboard unavailable */ }
    });
    d.append(el("div", { class: "cmd" }, el("code", { text }), btn));
  }
  if (s.how.instructions) d.append(el("p", { class: "regen", text: s.how.instructions }));
  if (s.regenerates) d.append(el("div", { class: "regen", text: `↺ ${s.regenerates}` }));
  if (s.notes?.length) d.append(el("ul", { class: "notes" }, s.notes.map((n) => el("li", { text: n }))));
  if (s.items.length) d.append(el("ul", { class: "rows compact" }, s.items.slice(0, 60).map((i) => itemRow({ path: i.path, size: i.bytes, note: i.note, hint: `${s.name} (spacetrail risk: ${s.risk})` }))));
  return d;
}

function updateSim() {
  const r = state.report;
  if (!r) { $("simbar").hidden = true; return; }
  const sel = r.suggestions.filter((s) => state.selected.has(s.id)).reduce((a, s) => a + s.bytes, 0);
  const [total, free] = totalFree();
  $("sim-selected").textContent = bytes(sel);
  $("sim-free").textContent = `${bytes(free + sel)} (${pct(free + sel, total)}%)`;
  $("simbar").hidden = sel === 0 || state.view !== "free";
  const runnable = r.suggestions.filter((s) => state.selected.has(s.id) && (stepPlan(s).run || stepPlan(s).manual)).length;
  $("clean-btn").disabled = runnable === 0;
  $("clean-btn").textContent = runnable ? `Clean selected (${runnable})…` : "Nothing runnable selected";
}

// ───────────────────────── clean selected ─────────────────────────
/// "Your call" items the user ticked in the plan, by suggestion id.
const picks = new Map();
const pickedOf = (s) => picks.get(s.id) || new Set();
const tool = (argv) => argv[0].split("/").pop();

/// How spacetrail would carry out a suggestion (decided by the backend), or why not.
function stepPlan(s) {
  const n = s.items.length;
  if (s.exec?.kind === "none" || !s.exec) return { run: false, icon: "✋", how: "Not run: spacetrail has no automatic way; see the instructions" };
  if (s.risk === "manual") {
    const k = pickedOf(s).size;
    if (s.exec.kind === "command") return { run: k > 0, icon: "🔒", how: k ? "Run " : "Your call: tick to confirm, then it runs ", code: s.exec.argv.map((a, i) => (i ? a : tool(s.exec.argv))).join(" "), manual: true };
    return { run: k > 0, icon: "🔒", how: k ? `${k} picked: removed with the tool's uninstaller or moved to the Trash (never permanent)` : "Your call: pick what to remove below", manual: true };
  }
  const pre = s.blocked_by?.length ? `Close ${s.blocked_by.join(", ")} first (checked again when cleaning) · ` : "";
  switch (s.exec.kind) {
    case "remove_items": return { run: true, icon: s.blocked_by?.length ? "⏸" : "🗑", how: `${pre}${$("cl-permanent").checked ? "Delete permanently" : "Move to the Trash"}: ${n} item${n === 1 ? "" : "s"}` };
    case "command": return { run: true, icon: s.blocked_by?.length ? "⏸" : "▶", how: `${pre}Run `, code: s.exec.argv.map((a, i) => (i ? a : tool(s.exec.argv))).join(" ") };
    case "remove_worktrees": return { run: true, icon: "⎇", how: `Remove ${n} git worktree${n === 1 ? "" : "s"} (git refuses if there are uncommitted changes; branches are kept)` };
    default: return { run: false, icon: "✋", how: "Not run" };
  }
}

let cleanupResults = null;
const chosenSuggestions = () => (state.report ? state.report.suggestions.filter((s) => state.selected.has(s.id)) : []);
function openCleanup() {
  cleanupResults = null;
  renderCleanupPlan();
  $("cleanup").showModal();
}

function manualPicker(s) {
  // Commands (e.g. docker volume prune) are confirmed with one tick; items one by one.
  const list = s.exec.kind === "command" ? [{ path: `cmd:${s.id}`, bytes: s.bytes, note: "Confirm running this command" }] : s.items;
  const sel = pickedOf(s);
  return el("ul", { class: "picks" }, list.map((i) => {
    const box = el("input", { type: "checkbox", disabled: i.protected ? "" : null });
    box.checked = sel.has(i.path);
    box.addEventListener("change", () => {
      const set = new Set(pickedOf(s));
      if (box.checked) set.add(i.path); else set.delete(i.path);
      picks.set(s.id, set);
      renderCleanupPlan();
    });
    const how = i.protected ? `kept: ${i.protected}` : i.argv ? `${tool(i.argv)} ${i.argv.slice(1, 3).join(" ")}` : s.exec.kind === "command" ? "" : "Trash";
    return el("li", { class: i.protected ? "skip" : "" },
      el("label", {}, box, el("span", { class: "pk-name", text: i.note || tilde(i.path), title: i.path }), el("span", { class: "pk-how", text: how })),
      el("span", { class: "pk-size", text: bytes(i.bytes) }));
  }));
}

function renderCleanupPlan() {
  const chosen = chosenSuggestions();
  const runnable = chosen.filter((s) => stepPlan(s).run);
  const sizeOf = (s) => (s.risk === "manual" && s.exec?.kind !== "command" ? s.items.filter((i) => pickedOf(s).has(i.path)).reduce((a, i) => a + i.bytes, 0) : s.bytes);
  const bytesRun = runnable.reduce((a, s) => a + sizeOf(s), 0);
  $("cl-title").textContent = "Clean selected";
  $("cl-sub").textContent = `${runnable.length} of ${chosen.length} selected will run · about ${bytes(bytesRun)}`;
  $("cl-steps").replaceChildren(...chosen.map((s) => {
    const p = stepPlan(s);
    const body = el("div", {}, el("div", { class: "st-name", text: s.name }), el("div", { class: "st-how" }, p.how, p.code ? el("code", { text: p.code }) : null));
    if (p.manual) body.append(manualPicker(s));
    return el("li", { class: p.run || p.manual ? "" : "skip", "data-id": s.id },
      el("span", { class: "st-icon", text: p.icon }), body,
      el("span", { class: "st-size", text: `${s.exact ? "" : "≤ "}${bytes(p.manual ? sizeOf(s) || s.bytes : s.bytes)}` }));
  }));
  $("cl-permanent-row").hidden = !runnable.some((s) => s.exec?.kind === "remove_items" && s.risk !== "manual");
  $("cl-note").textContent = ($("cl-permanent").checked
    ? "Safe/Review items are deleted for good. "
    : "Items go to the Trash: the space is freed when you empty it, and you can undo. ") +
    "Tool commands free space right away. “Your call” items run only if you tick them, and never permanently.";
  $("cl-summary").textContent = "";
  $("cl-go").hidden = false;
  $("cl-go").disabled = runnable.length === 0;
  $("cl-go").textContent = `Clean ${bytes(bytesRun)}`;
  $("cl-cancel").textContent = "Cancel";
  $("cl-cancel").disabled = false;
  $("cl-permanent").disabled = false;
  $("cl-undo").hidden = true;
}

/// Item picks to send for a set of suggestions (manual ones only).
function pickMap(list) {
  const m = {};
  for (const s of list) if (s.risk === "manual") m[s.id] = s.exec?.kind === "command" ? s.items.map((i) => i.path) : [...pickedOf(s)];
  return m;
}

async function execute(list, items) {
  for (const s of list) {
    const icon = document.querySelector(`#cl-steps li[data-id="${CSS.escape(s.id)}"] .st-icon`);
    if (icon) icon.replaceChildren(el("span", { class: "spinner" }));
  }
  return invoke("run_cleanup", { ids: list.map((s) => s.id), permanent: $("cl-permanent").checked, items });
}

function showResult(r) {
  const li = document.querySelector(`#cl-steps li[data-id="${CSS.escape(r.id)}"]`);
  if (!li) return;
  li.classList.remove("skip");
  li.querySelector(".st-icon").textContent = r.ok ? "✅" : r.done ? "⚠️" : r.method === "skipped" && !r.failed ? "➖" : "❌";
  const body = li.children[1];
  for (const x of body.querySelectorAll(".st-msg, .picks, .retry")) x.remove();
  body.querySelector(".st-how").textContent = r.done
    ? `${r.done} done${r.failed ? `, ${r.failed} failed` : ""}${r.bytes ? ` · ${bytes(r.bytes)}` : ""}`
    : r.method === "skipped" ? "Skipped" : "Failed";
  if (r.messages.length) body.append(el("div", { class: `st-msg ${r.ok ? "" : "err"}`, text: r.messages.join("\n") }));
  if (!r.ok && (r.failed || r.retry_items.length)) {
    const btn = el("button", { class: "btn retry", type: "button", text: "Retry" });
    btn.addEventListener("click", () => retryStep(r, btn));
    body.append(btn);
  }
}

async function retryStep(prev, btn) {
  const s = state.report.suggestions.find((x) => x.id === prev.id);
  if (!s) return;
  btn.disabled = true;
  const items = prev.retry_items.length ? { [s.id]: prev.retry_items } : pickMap([s]);
  try {
    const [r] = await execute([s], items);
    cleanupResults = cleanupResults.map((x) => (x.id === r.id ? mergeResult(x, r) : x));
    showResult(r);
    afterRun();
  } catch (e) {
    btn.disabled = false;
    toast(String(e));
  }
}
/// A retry only re-runs what failed: keep the earlier successes.
const mergeResult = (a, b) => ({ ...b, done: a.done + b.done, bytes: a.bytes + b.bytes, undo_ids: [...a.undo_ids, ...b.undo_ids], ok: b.failed === 0 && a.done + b.done > 0 });

async function runCleanup() {
  const list = chosenSuggestions().filter((s) => stepPlan(s).run);
  if (!list.length) return;
  $("cl-go").disabled = true;
  $("cl-cancel").disabled = true;
  $("cl-permanent").disabled = true;
  try {
    cleanupResults = await execute(list, pickMap(list));
  } catch (e) {
    $("cl-summary").textContent = String(e);
    $("cl-go").disabled = false; $("cl-cancel").disabled = false; $("cl-permanent").disabled = false;
    return;
  }
  cleanupResults.forEach(showResult);
  $("cl-go").hidden = true;
  $("cl-cancel").disabled = false;
  $("cl-cancel").textContent = "Close & rescan";
  $("cl-permanent").disabled = false;
  afterRun();
}

function afterRun() {
  let freed = 0, trashed = 0, failed = 0;
  for (const r of cleanupResults) {
    if (r.method === "trash") trashed += r.bytes; else freed += r.bytes;
    failed += r.ok ? 0 : r.failed;
  }
  const parts = [];
  if (freed) parts.push(`Freed about ${bytes(freed)}`);
  if (trashed) parts.push(`moved ${bytes(trashed)} to the Trash (empty it to free the space)`);
  if (failed) parts.push(`${failed} failed`);
  $("cl-summary").textContent = parts.length ? `${parts.join(" · ")}.` : "Nothing was cleaned.";
  $("cl-undo").hidden = cleanupResults.flatMap((r) => r.undo_ids).length === 0;
  const doneIds = new Set(cleanupResults.filter((r) => r.ok).map((r) => r.id));
  state.report.suggestions = state.report.suggestions.filter((s) => !doneIds.has(s.id));
  for (const id of doneIds) { state.selected.delete(id); picks.delete(id); }
  recomputeTotals();
  renderSuggestions();
  invoke("get_disk_status").then((st) => { state.status = st; renderSummary(); updateSim(); });
}
function recomputeTotals() {
  const t = { safe: 0, review: 0, manual: 0 };
  for (const s of state.report.suggestions) t[s.risk] += s.bytes;
  state.report.totals = t;
}
async function undoCleanup() {
  const ids = (cleanupResults || []).flatMap((r) => r.undo_ids);
  let ok = 0, bad = 0;
  for (const id of ids) { try { await invoke("undo_trash", { id }); ok++; } catch { bad++; } }
  for (const r of cleanupResults) r.undo_ids = [];
  $("cl-undo").hidden = true;
  $("cl-summary").textContent = `Put back ${ok} item${ok === 1 ? "" : "s"}${bad ? `, ${bad} could not be restored` : ""}.`;
}
$("clean-btn").addEventListener("click", openCleanup);
$("cl-go").addEventListener("click", runCleanup);
$("cl-undo").addEventListener("click", undoCleanup);
$("cl-permanent").addEventListener("change", () => { if (!cleanupResults) renderCleanupPlan(); });
const closeCleanup = () => { $("cleanup").close(); if (cleanupResults) loadAll(); };
$("cl-cancel").addEventListener("click", closeCleanup);
$("cl-close").addEventListener("click", closeCleanup);
listen("cleanup-progress", (e) => { $("cl-sub").textContent = `Cleaning ${e.payload.index + 1} of ${e.payload.total}: ${e.payload.name}…`; });

// ───────────────────────── toast ─────────────────────────
let toastTimer = null;
function toast(text, action) {
  const t = $("toast");
  t.replaceChildren(el("span", { text }));
  if (action) t.append(el("button", { type: "button", text: action.label, onclick: () => { t.hidden = true; action.run(); } }));
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), action ? 12000 : 4000);
}

// ───────────────────────── delete (to Trash, via the executor) ─────────────────────────
/// Files that look harmless in a size list but hold a whole app's data.
const CRITICAL = [
  [/\/Docker\.raw$|\/com\.docker\.docker\//, "This is Docker's virtual disk: deleting it erases ALL your Docker images, containers and volumes (databases included). Free space with Docker's own cleanup instead (see “Free up space”)."],
  [/\.(vmdk|vdi|qcow2|hdd|utm)$|\.(vmwarevm|pvm|utm)\//i, "This is a virtual machine disk: deleting it erases that VM and everything inside it."],
  [/\.photoslibrary(\/|$)/i, "This is a Photos library: deleting it removes your photos stored on this Mac."],
  [/\/Library\/Mail\/|\/Library\/Messages\//, "These are your local emails/messages."],
  [/\/MobileSync\/Backup\//, "This is an iPhone/iPad backup: it may be the only copy of that device's data."],
  [/\/\.git\//, "This is inside a git repository's internal database: deleting it can corrupt the repository."],
];
const criticalWarning = (p) => (CRITICAL.find(([re]) => re.test(p)) || [])[1];

let pending = null;
function confirmDelete(item) {
  pending = item;
  $("cf-path").textContent = tilde(item.path);
  $("cf-facts").replaceChildren(el("span", {}, "Size ", el("b", { text: bytes(item.size) })), el("span", {}, "Name ", el("b", { text: baseName(item.path) })));
  const warn = criticalWarning(item.path) || (isSystem(item.path) ? "This looks like a system or app-internal file; spacetrail will most likely refuse to remove it." : "");
  $("cf-error").hidden = !warn;
  $("cf-error").textContent = warn ? `⚠️ ${warn}` : "";
  $("cf-ai").replaceChildren();
  if (explained.has(item.path)) $("cf-ai").append(explanationView(explained.get(item.path), true));
  $("cf-go").disabled = false;
  $("cf-ask").disabled = false;
  $("confirm").showModal();
}
async function askInConfirm() {
  if (!pending) return;
  const box = $("cf-ai");
  box.replaceChildren(el("div", { class: "loading" }, el("span", { class: "spinner" }), el("span", { text: "Asking the AI…" })));
  $("cf-ask").disabled = true;
  try {
    box.replaceChildren(explanationView(await getExplanation(pending.path, { bytes: pending.size }), true));
  } catch (e) {
    box.replaceChildren(errorWithSettings(String(e), () => $("confirm").close()));
  } finally {
    $("cf-ask").disabled = false;
  }
}
async function doDelete() {
  if (!pending) return;
  const item = pending;
  $("cf-go").disabled = true;
  try {
    const rec = await invoke("trash_item", { path: item.path, bytes: item.size, source: item.source || "files" });
    $("confirm").close();
    removeFromLists(item.path);
    toast(`Moved “${baseName(item.path)}” to the Trash. Empty the Trash to free ${bytes(item.size)}.`, {
      label: "Undo",
      run: async () => {
        try { await invoke("undo_trash", { id: rec.id }); restoreToLists(item); toast(`Restored “${baseName(item.path)}”`); }
        catch (e) { toast(String(e)); }
      },
    });
  } catch (e) {
    $("cf-error").hidden = false;
    $("cf-error").textContent = String(e);
    $("cf-go").disabled = false;
  }
}
const removed = new Map();
function removeFromLists(path) {
  const ov = state.overview;
  for (const key of ["largest_files", "oldest_files"]) {
    const idx = ov[key].findIndex((f) => f.path === path);
    if (idx >= 0) { removed.set(`${key}:${path}`, [idx, ov[key][idx]]); ov[key].splice(idx, 1); }
  }
  renderFiles();
}
function restoreToLists(item) {
  const ov = state.overview;
  for (const key of ["largest_files", "oldest_files"]) {
    const r = removed.get(`${key}:${item.path}`);
    if (r) { ov[key].splice(r[0], 0, r[1]); removed.delete(`${key}:${item.path}`); }
  }
  renderFiles();
}
$("cf-ask").addEventListener("click", askInConfirm);
$("cf-go").addEventListener("click", doDelete);
$("cf-cancel").addEventListener("click", () => $("confirm").close());
$("cf-close").addEventListener("click", () => $("confirm").close());

// ───────────────────────── settings ─────────────────────────
const PROVIDERS = [
  { id: "anthropic", label: "Anthropic", keyHint: "sk-ant-…", needsKey: true },
  { id: "openai", label: "OpenAI", keyHint: "sk-…", needsKey: true },
  { id: "compatible", label: "OpenAI-compatible", keyHint: "optional (e.g. OpenRouter key)", needsKey: false },
];
let settingsView = null;
async function openSettings() { settingsView = await invoke("get_settings"); renderSettings(); $("settings").showModal(); }
const hasKey = (id) => (settingsView.has_key || []).some(([p, has]) => p === id && has);
async function saveSettings(patch) {
  settingsView = await invoke("save_settings", { settings: { ...settingsView.settings, ...patch } });
  $("settings-status").textContent = "Saved ✓";
  setTimeout(() => ($("settings-status").textContent = ""), 1500);
}
function renderSettings() {
  const st = settingsView.settings;
  if (settingsView.keys_file) $("keys-file").textContent = settingsView.keys_file;
  $("provider-seg").replaceChildren(...PROVIDERS.map((p) => el("button", { type: "button", role: "radio", "aria-checked": String(st.active === p.id), text: p.label,
    onclick: async () => { await saveSettings({ active: p.id }); renderSettings(); } })));
  const panels = $("provider-panels");
  panels.replaceChildren();
  const p = PROVIDERS.find((x) => x.id === st.active);
  if (!p) { panels.append(el("p", { class: "muted small", text: "Pick a provider above to set it up." })); }
  else {
    const keyInput = el("input", { type: "password", placeholder: hasKey(p.id) ? "•••••••• saved (type to replace)" : p.keyHint, autocomplete: "off", spellcheck: "false" });
    const keyMsg = el("span", { class: hasKey(p.id) ? "ok-text" : "muted small", text: hasKey(p.id) ? "Key saved ✓" : p.needsKey ? "No key saved yet" : "" });
    const saveKey = el("button", { class: "btn", type: "button", text: "Save key" });
    saveKey.addEventListener("click", async () => {
      try { settingsView = await invoke("set_api_key", { provider: p.id, key: keyInput.value }); keyInput.value = ""; renderSettings(); }
      catch (e) { keyMsg.className = "err-text"; keyMsg.textContent = String(e); }
    });
    const removeKey = iconBtn("trash", "Remove saved key", async () => { settingsView = await invoke("set_api_key", { provider: p.id, key: "" }); renderSettings(); }, "del");
    removeKey.hidden = !hasKey(p.id);
    const modelKey = `${p.id}_model`;
    const modelSel = el("select", { id: "model-select" }, el("option", { value: st[modelKey] || "", text: st[modelKey] || "— load the model list —" }));
    modelSel.addEventListener("change", () => saveSettings({ [modelKey]: modelSel.value }));
    const panel = el("div", { class: "panel" },
      el("div", { class: "field" }, el("span", { class: "label", text: `${p.label} API key` }), el("div", { class: "row" }, keyInput, saveKey, removeKey), keyMsg));
    if (p.id === "compatible") {
      const url = el("input", { type: "text", value: st.compatible_base_url, placeholder: "http://localhost:11434/v1" });
      url.addEventListener("change", () => saveSettings({ compatible_base_url: url.value.trim() }));
      panel.append(el("div", { class: "field" }, el("span", { class: "label", text: "Base URL (OpenRouter, Ollama, LM Studio…)" }), url));
    }
    panel.append(el("div", { class: "field" }, el("span", { class: "label", text: "Model" }),
      el("div", { class: "row" }, modelSel, iconBtn("refresh", "Reload the model list", loadModels)), el("span", { class: "muted small", id: "model-msg" })));
    panels.append(panel);
    if (hasKey(p.id) || !p.needsKey) loadModels();
  }
  $("language").value = st.language;
  $("share-listing").checked = st.share_folder_listing;
}
async function loadModels() {
  const st = settingsView.settings, sel = $("model-select"), msg = $("model-msg");
  if (!sel) return;
  msg.className = "muted small"; msg.textContent = "Loading models…";
  try {
    const models = await invoke("list_models", { provider: st.active });
    const key = `${st.active}_model`;
    let current = st[key];
    if (!current && models.length) {
      current = (st.active === "anthropic" ? models.find((m) => m.id === "claude-opus-5-5") || models[0] : models.find((m) => /mini/.test(m.id)) || models[0]).id;
      await saveSettings({ [key]: current });
    }
    sel.replaceChildren(...models.map((m) => el("option", { value: m.id, text: m.name === m.id ? m.id : `${m.name} (${m.id})` })));
    if (current && !models.some((m) => m.id === current)) sel.prepend(el("option", { value: current, text: current }));
    sel.value = current || "";
    msg.textContent = `${models.length} models available`;
  } catch (e) { msg.className = "err-text"; msg.textContent = String(e); }
}
$("open-settings").addEventListener("click", openSettings);
$("language").addEventListener("change", () => saveSettings({ language: $("language").value }));
$("share-listing").addEventListener("change", () => saveSettings({ share_folder_listing: $("share-listing").checked }));

// ───────────────────────── explain ─────────────────────────
const explained = new Map();
const VERDICT = { safe: { icon: "✅", text: "Looks safe to delete" }, caution: { icon: "⚠️", text: "Delete with care" }, keep: { icon: "🛑", text: "Better keep it" } };

async function getExplanation(path, hint) {
  if (explained.has(path)) return explained.get(path);
  const ex = await invoke("explain", { path, hint });
  explained.set(path, ex);
  return ex;
}
function errorWithSettings(msg, before) {
  const box = el("div", {}, el("p", { class: "err-text", text: msg }));
  if (/Settings|API key|model|provider/i.test(msg)) box.append(el("button", { class: "btn primary", type: "button", text: "Open Settings", onclick: () => { before?.(); openSettings(); } }));
  return box;
}
function explanationView(ex, compact = false) {
  const a = ex.answer, v = VERDICT[a.verdict] || VERDICT.caution;
  const sec = (title, text) => (text ? el("div", { class: "ex-sec" }, el("h4", { text: title }), el("p", { text })) : null);
  return el("div", {},
    el("div", { class: `verdict ${VERDICT[a.verdict] ? a.verdict : "caution"}` }, el("span", { class: "v-icon", text: v.icon }), el("span", { text: `${v.text} · confidence: ${a.confidence}` })),
    sec("What it is", a.what), sec("Where it comes from", a.origin), sec("If you delete it", a.impact),
    compact ? null : sec("How to remove it", a.how_to_remove),
    el("div", { class: "ex-foot muted small" },
      el("div", { text: `Answered by ${ex.provider} · ${ex.model}. AI answers can be wrong; when in doubt keep it, or use the Trash so you can undo.` }),
      compact ? null : el("details", {}, el("summary", { text: "What was sent" }), el("pre", { text: JSON.stringify(ex.sent, null, 2) }))));
}
async function explainItem(path, hint = {}) {
  $("ex-title").textContent = baseName(path);
  $("ex-path").textContent = tilde(path);
  const body = $("ex-body"), dlg = $("explain");
  if (!dlg.open) dlg.showModal();
  if (!explained.has(path)) body.replaceChildren(el("div", { class: "loading" }, el("span", { class: "spinner" }), el("span", { text: "Asking the AI where this comes from…" })));
  try {
    const ex = await getExplanation(path, hint);
    if ($("ex-path").textContent === tilde(path)) body.replaceChildren(explanationView(ex));
  } catch (e) {
    body.replaceChildren(errorWithSettings(String(e), () => dlg.close()));
  }
}
$("ex-close").addEventListener("click", () => $("explain").close());

// ───────────────────────── loading ─────────────────────────
let loading = false;
async function loadAll() {
  if (loading) return;
  loading = true;
  $("rescan").disabled = true;
  try {
    state.status = await invoke("get_disk_status");
    renderSummary(); renderFda();
    activity("Scanning your disk…");
    $("treemap").replaceChildren(el("div", { class: "loading" }, el("span", { class: "spinner" }), el("span", { id: "tm-progress", text: "Scanning your disk…" })));
    const ov = await invoke("get_overview");
    state.overview = ov; state.stack = [ov.tree];
    renderCrumbs(); renderTreemap(); renderMapFoot(); renderFiles();
    activity("Looking for things to free…");
    state.report = await invoke("get_suggestions");
    state.selected = new Set(state.report.suggestions.filter((s) => s.risk === "safe").map((s) => s.id));
    state.status = state.report.disk;
    renderSummary(); renderSuggestions(); renderTreemap();
    activity("");
  } catch (e) {
    console.error(e);
    activity(`Something went wrong: ${e}`);
  } finally {
    loading = false;
    $("rescan").disabled = false;
  }
}

listen("scan-progress", (e) => { const p = $("tm-progress"); if (p) p.textContent = `Scanning your disk… ${e.payload.files.toLocaleString()} files · ${bytes(e.payload.bytes)}`; });
listen("suggest-progress", (e) => { activity(`${e.payload}…`); const p = $("sg-progress"); if (p) p.textContent = `${e.payload}…`; });

$("rescan").innerHTML = svg("refresh");
$("open-settings").innerHTML = svg("gear");
for (const b of document.querySelectorAll(".close-x")) b.innerHTML = svg("close");
$("rescan").addEventListener("click", loadAll);
$("fda-open").addEventListener("click", openFda);
$("fda-restart").addEventListener("click", () => invoke("restart"));
let resizeT;
window.addEventListener("resize", () => { clearTimeout(resizeT); resizeT = setTimeout(renderTreemap, 120); });

loadAll();

// ───────────────────────── browser preview (no Tauri) ─────────────────────────
function mockInvoke(cmd, args) {
  const GB = 1e9;
  const f = (name, path, b, kids = [], st = "complete") => ({ name, path, bytes: b, files: Math.round(b / 2e5), state: st, children: kids, other: Math.max(0, b - kids.reduce((s, k) => s + k.bytes, 0)) });
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
  const sg = (id, name, risk, b, path, extra = {}) => ({ id, name, risk, bytes: b, exact: true, description: "Example description of what this is.", how: { summary: "Run the command below", commands: [["tool", "clean"]] }, items: [{ path, bytes: b }], notes: [], blocked_by: [], exec: { kind: risk === "manual" ? "none" : "remove_items" }, ...extra });
  const status = { volume: { total: 494 * GB, free: 6.5 * GB }, full_disk_access: { status: "granted" },
    container: { total: 494 * GB, free: 6.5 * GB, volumes: [{ role: "data", consumed: 446 * GB }, { role: "vm", consumed: 15 * GB }, { role: "system", consumed: 12.6 * GB }, { role: "preboot", consumed: 10.8 * GB }, { role: "recovery", consumed: 2.6 * GB }] } };
  const report = { disk: status, totals: { safe: 40.5 * GB, review: 88 * GB, manual: 168 * GB },
    suggestions: [
      sg("vscode-logs", "VS Code logs", "safe", 12.8 * GB, "/Users/demo/Library/Application Support/Code/logs", { regenerates: "New logs are written each session." }),
      sg("yarn", "Yarn cache", "safe", 7.6 * GB, "/Users/demo/Library/Caches/Yarn", { exact: false, exec: { kind: "command", argv: ["/opt/homebrew/bin/yarn", "cache", "clean"] } }),
      sg("wt", "Git worktrees", "review", 32.8 * GB, "/Users/demo/Documents/dev/repo/.claude/worktrees", { exec: { kind: "remove_worktrees" }, items: [{ path: "/Users/demo/Documents/dev/repo/.claude/worktrees/a", bytes: 3e9, note: "branch a · last change 9 days ago" }, { path: "/Users/demo/Documents/dev/repo/.claude/worktrees/b", bytes: 2e9 }] }),
      sg("docker", "Docker build cache", "review", 18.4 * GB, "/Users/demo/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw", { exact: false, blocked_by: ["Docker"], exec: { kind: "command", argv: ["/usr/local/bin/docker", "builder", "prune", "--force"] } }),
      sg("game", "Large app data", "manual", 73 * GB, "/Users/demo/Library/Application Support/SomeGame", { exec: { kind: "remove_items" } }),
      sg("ollama-models", "Ollama models", "manual", 9 * GB, "/x", { exec: { kind: "remove_items" }, items: [{ path: "/Users/demo/.ollama/models/manifests/registry.ollama.ai/library/llama3/8b", bytes: 4.7e9, note: "model llama3:8b", argv: ["/opt/homebrew/bin/ollama", "rm", "llama3:8b"] }, { path: "/Users/demo/.ollama/models/manifests/registry.ollama.ai/library/qwen/7b", bytes: 4.3e9, note: "model qwen:7b", argv: ["/opt/homebrew/bin/ollama", "rm", "qwen:7b"] }] }),
      sg("nvm-node-versions", "Node.js versions (nvm)", "manual", 0.6e9, "/x", { exec: { kind: "remove_items" }, items: [{ path: "/Users/demo/.nvm/versions/node/v16.20.2", bytes: 2e8 }, { path: "/Users/demo/.nvm/versions/node/v22.22.2", bytes: 4e8, protected: "your default Node version" }] }),
    ],
    advice: ["Swap uses 15 GB because memory is under pressure.", "Coding agents create a git worktree per task."], catalog_problems: [] };
  const delay = (v, ms) => new Promise((r) => setTimeout(() => r(v), ms));
  switch (cmd) {
    case "get_disk_status": return delay(status, 50);
    case "get_overview": return delay({ elapsed_secs: 19, totals: { alloc: 458 * GB, files: 3561390 }, reconciliation: null, privacy_protected: 518, other_owner: 148, tree,
      largest_files: [{ path: "/Users/demo/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw", bytes: 56.9 * GB, modified: 1790000000 }, { path: "/Users/demo/Downloads/movie.mkv", bytes: 4.2 * GB, modified: 1760000000 }, { path: "/System/Library/AssetsV2/x.dmg", bytes: 8.8 * GB, modified: 1700000000 }],
      oldest_files: [{ path: "/Users/demo/Downloads/old-installer.dmg", bytes: 0.9 * GB, modified: 1600000000 }, { path: "/Applications/Xcode.app/Contents/x", bytes: 0.2 * GB, modified: 1500000000 }] }, 300);
    case "get_suggestions": return delay(report, 300);
    case "check_full_disk_access": return delay({ status: "granted" }, 50);
    case "get_settings": case "save_settings": case "set_api_key":
      return delay({ settings: { active: "anthropic", anthropic_model: "claude-opus-5-5", openai_model: "", compatible_base_url: "http://localhost:11434/v1", compatible_model: "", language: "Spanish", share_folder_listing: true }, has_key: [["anthropic", true], ["openai", false], ["compatible", false]], keys_file: "~/.config/spacetrail/api-keys.json" }, 50);
    case "list_models": return delay([{ id: "claude-opus-5-5", name: "Claude Opus 5.5" }, { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5" }], 200);
    case "explain": return delay({ provider: "Anthropic", model: "claude-opus-5-5", answer: { what: "A disk image of an old app installer.", origin: "Downloaded with Safari; the app it installs is already in /Applications.", verdict: "safe", impact: "Nothing breaks; you can download it again if needed.", how_to_remove: "Move it to the Trash.", confidence: "high" }, sent: { path: tilde(args.path), kind: "file" } }, 600);
    case "trash_item": return delay({ id: "1-1", original: args.path, trashed_to: "/Users/demo/.Trash/x" }, 300);
    case "undo_trash": return delay({ id: args.id, restored: true }, 200);
    case "run_cleanup": return delay(args.ids.map((id) => {
      const fail = id === "wt" && !(args.items?.wt);
      return { id, name: id, method: id === "yarn" ? "command" : "trash", ok: !fail, done: 1, failed: fail ? 1 : 0, bytes: 5e9, messages: fail ? ["/Users/demo/…/b: git refused: contains modified or untracked files"] : [], undo_ids: id === "vscode-logs" ? ["1"] : [], retry_items: fail ? ["/Users/demo/Documents/dev/repo/.claude/worktrees/b"] : [] };
    }), 900);
    default: return delay(null, 10);
  }
}

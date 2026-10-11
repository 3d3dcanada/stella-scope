/* Stella Scope UI: live view and rewind. No frameworks, no CDN: it must work with no internet. */
const $ = (id) => document.getElementById(id);
const fmt = (v, d = 2) => (v === null || v === undefined || Number.isNaN(v)) ? "—" : Number(v).toFixed(d);
const esc = (s) => String(s ?? "").replace(/[<>&"]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;" }[c]));
const clock = (ts) => new Date(ts * 1000).toLocaleTimeString([], { hour12: false });
const stamp = (ts) => new Date(ts * 1000).toLocaleString([], { hour12: false });

const S = {
  live: true, meta: null, sample: null, at: null,      // at = replay timestamp
  series: [], events: [], qgl: [], selection: { kind: "toolhead" }, windowSec: 900,
};

async function api(path) {
  const r = await fetch(path);
  if (!r.ok) throw new Error(path + " -> " + r.status);
  return r.json();
}

/* ---------- map ---------- */
const BED = 350;
function drawMap() {
  const svg = $("map"), d = S.sample?.data || {};
  const mr = d.motion_report || {}, th = d.toolhead || {};
  const pos = mr.live_position || th.position || [null, null, null, 0];
  const homed = (th.homed_axes || "") === "xyz";
  const px = (v) => 5 + (v / BED) * 350;            // bed x -> svg x
  const py = (v) => 355 - (v / BED) * 350;          // bed y -> svg y (front at bottom)
  let out = `<rect x="5" y="5" width="350" height="350" rx="8" fill="#0b0f15" stroke="#2a323f"/>`;
  for (let g = 50; g < BED; g += 50) {
    out += `<line x1="${px(g)}" y1="5" x2="${px(g)}" y2="355" stroke="#1b222c"/>`;
    out += `<line x1="5" y1="${py(g)}" x2="355" y2="${py(g)}" stroke="#1b222c"/>`;
  }
  (S.meta?.corners || []).forEach((c) => {
    const tm = d["tmc2209 " + c.id] || {}, drv = tm.drv_status || {};
    const moving = drv.stst ? 0 : 1;
    const sel = S.selection.kind === "motor" && S.selection.id === c.id;
    const cx = 5 + c.x * 350, cy = 5 + c.y * 350;
    const colour = drv.ot ? "#ff5f56" : drv.otpw ? "#ffbe4d" : moving ? "#4da3ff" : "#3ddc97";
    out += `<g class="clickable" data-motor="${c.id}">
      <circle cx="${cx}" cy="${cy}" r="${sel ? 26 : 22}" fill="#161b23" stroke="${colour}" stroke-width="${sel ? 3 : 2}"/>
      <text x="${cx}" y="${cy - 3}" text-anchor="middle" fill="#e8ecf2" font-size="12" font-weight="600">${c.id.replace("stepper_", "")}</text>
      <text x="${cx}" y="${cy + 12}" text-anchor="middle" fill="#93a0b4" font-size="10">${fmt(tm.run_current, 2)}A</text>
    </g>`;
  });
  if (pos[0] !== null && pos[0] !== undefined) {
    const tx = px(pos[0]), ty = py(pos[1]);
    const sel = S.selection.kind === "toolhead";
    out += `<g class="clickable" data-toolhead="1">
      <line x1="${tx}" y1="5" x2="${tx}" y2="355" stroke="#4da3ff" stroke-dasharray="3 5" opacity=".45"/>
      <line x1="5" y1="${ty}" x2="355" y2="${ty}" stroke="#4da3ff" stroke-dasharray="3 5" opacity=".45"/>
      <circle cx="${tx}" cy="${ty}" r="${sel ? 11 : 8}" fill="#4da3ff" opacity=".85"/>
      <circle cx="${tx - (32.95 / BED) * 350}" cy="${ty + (10.718 / BED) * 350}" r="5" fill="none" stroke="#c792ea" stroke-width="2"/>
    </g>`;
  }
  out += `<text x="180" y="373" text-anchor="middle" fill="#93a0b4" font-size="11">front${homed ? "" : " — not homed"}</text>`;
  svg.innerHTML = out;
  $("map-sub").textContent = pos[0] == null ? "" :
    `X ${fmt(pos[0], 1)}  Y ${fmt(pos[1], 1)}  Z ${fmt(pos[2], 3)}`;
  svg.querySelectorAll("[data-motor]").forEach((el) =>
    el.onclick = () => { S.selection = { kind: "motor", id: el.dataset.motor }; render(); });
  svg.querySelectorAll("[data-toolhead]").forEach((el) =>
    el.onclick = () => { S.selection = { kind: "toolhead" }; render(); });
}

/* ---------- charts (plain canvas: no libraries to break offline) ---------- */
function chart(canvas, series, opts = {}) {
  const dpr = window.devicePixelRatio || 1, w = canvas.clientWidth, h = canvas.clientHeight;
  canvas.width = w * dpr; canvas.height = h * dpr;
  const c = canvas.getContext("2d"); c.setTransform(dpr, 0, 0, dpr, 0, 0); c.clearRect(0, 0, w, h);
  const all = series.flatMap((s) => s.data.map((p) => p[1])).filter((v) => typeof v === "number");
  if (!all.length) {
    c.fillStyle = "#93a0b4"; c.font = "13px system-ui";
    c.fillText("no data in this window", 12, h / 2); return;
  }
  let lo = Math.min(...all), hi = Math.max(...all);
  if (opts.zeroBased) lo = Math.min(0, lo);
  if (hi - lo < 1e-6) { hi += 1; lo -= 1; }
  const pad = (hi - lo) * 0.1; lo -= pad; hi += pad;
  const xs = series.flatMap((s) => s.data.map((p) => p[0]));
  const t0 = Math.min(...xs), t1 = Math.max(...xs);
  const X = (t) => 38 + ((t - t0) / Math.max(1e-6, t1 - t0)) * (w - 46);
  const Y = (v) => h - 18 - ((v - lo) / (hi - lo)) * (h - 28);
  c.strokeStyle = "#2a323f"; c.lineWidth = 1; c.fillStyle = "#93a0b4"; c.font = "11px system-ui";
  for (let i = 0; i <= 3; i++) {
    const v = lo + (i / 3) * (hi - lo), y = Y(v);
    c.beginPath(); c.moveTo(38, y); c.lineTo(w - 8, y); c.stroke();
    c.fillText(v.toFixed(opts.decimals ?? 0), 4, y + 3);
  }
  series.forEach((s) => {
    c.strokeStyle = s.colour; c.lineWidth = s.width || 2; c.setLineDash(s.dash || []);
    c.beginPath();
    let started = false;
    s.data.forEach(([t, v]) => {
      if (typeof v !== "number") { started = false; return; }
      const x = X(t), y = Y(v);
      started ? c.lineTo(x, y) : c.moveTo(x, y);
      started = true;
    });
    c.stroke(); c.setLineDash([]);
  });
  if (S.at) {  // where the rewind cursor sits
    const x = X(S.at);
    c.strokeStyle = "#ffbe4d"; c.setLineDash([4, 4]); c.beginPath();
    c.moveTo(x, 4); c.lineTo(x, h - 16); c.stroke(); c.setLineDash([]);
  }
  c.fillStyle = "#93a0b4";
  c.fillText(clock(t0), 38, h - 4); c.fillText(clock(t1), w - 56, h - 4);
}

function drawCharts() {
  const pts = S.series;
  const pick = (k) => pts.map((p) => [p.ts, p[k]]);
  chart($("chart-temp"), [
    { data: pick("nozzle"), colour: "#ff8f5e" },
    { data: pick("nozzle_t"), colour: "#93a0b4", width: 1, dash: [4, 4] },
    { data: pick("bed"), colour: "#4da3ff" },
    { data: pick("bed_t"), colour: "#93a0b4", width: 1, dash: [4, 4] },
  ], { decimals: 0 });
  chart($("chart-motion"), [
    { data: pick("z"), colour: "#c792ea" },
    { data: pick("v"), colour: "#3ddc97", width: 1 },
  ], { decimals: 1, zeroBased: true });
}

/* ---------- panels ---------- */
function drawMotors() {
  const d = S.sample?.data || {}, box = $("motors");
  box.innerHTML = (S.meta?.steppers || []).map((s) => {
    const tm = d["tmc2209 " + s] || {}, drv = tm.drv_status || {};
    const enabled = (d.stepper_enable?.steppers || {})[s];
    const load = drv.cs_actual != null ? Math.min(1, drv.cs_actual / 31) : 0;
    const flags = Object.keys(drv).filter((k) => !["cs_actual", "stst", "stealth"].includes(k));
    const cls = drv.ot ? "bad" : (drv.otpw || flags.length) ? "warn" : "ok";
    return `<div class="motor" data-motor="${s}">
      <div class="name">${s.replace("stepper_", "")} <span class="badge ${cls}">${
        drv.ot ? "too hot" : drv.otpw ? "warm" : flags.length ? flags.join(" ") : (drv.stst ? "idle" : "moving")}</span></div>
      <div class="sub">${fmt(tm.run_current, 2)} A · ${enabled === false ? "disabled" : "on"}</div>
      <div class="bar"><span style="width:${(load * 100).toFixed(0)}%"></span></div>
    </div>`;
  }).join("");
  box.querySelectorAll("[data-motor]").forEach((el) =>
    el.onclick = () => { S.selection = { kind: "motor", id: el.dataset.motor }; render(); });
}

function drawDetail() {
  const d = S.sample?.data || {}, sel = S.selection;
  const row = (k, v) => `<dt>${k}</dt><dd>${v}</dd>`;
  if (sel.kind === "motor") {
    const tm = d["tmc2209 " + sel.id] || {}, drv = tm.drv_status || {};
    const corner = (S.meta?.corners || []).find((c) => c.id === sel.id);
    $("detail-title").textContent = corner ? corner.label : sel.id.replace("stepper_", "motor ");
    $("detail-body").innerHTML = `<dl>
      ${row("driver current", fmt(tm.run_current, 2) + " A")}
      ${row("holding current", fmt(tm.hold_current, 2) + " A")}
      ${row("load now", drv.cs_actual != null ? drv.cs_actual + " / 31" : "—")}
      ${row("state", drv.stst ? "standing still" : "moving")}
      ${row("warnings", Object.keys(drv).filter((k) => !["cs_actual", "stst", "stealth"].includes(k)).join(", ") || "none")}
      ${row("enabled", (d.stepper_enable?.steppers || {})[sel.id] === false ? "no" : "yes")}
    </dl><p class="hint">Load is how hard the driver is pushing, 0 to 31. A corner that reads high while the
    others are low is working harder than it should.</p>`;
    return;
  }
  const mr = d.motion_report || {}, th = d.toolhead || {}, gm = d.gcode_move || {};
  const pos = mr.live_position || th.position || [];
  $("detail-title").textContent = "Print head";
  $("detail-body").innerHTML = `<dl>
    ${row("position", `X ${fmt(pos[0], 2)}  Y ${fmt(pos[1], 2)}  Z ${fmt(pos[2], 3)}`)}
    ${row("speed", fmt(mr.live_velocity, 1) + " mm/s")}
    ${row("homed", th.homed_axes || "nothing")}
    ${row("live Z offset", fmt((gm.homing_origin || [])[2], 3) + " mm")}
    ${row("nozzle", fmt(d.extruder?.temperature, 1) + " / " + fmt(d.extruder?.target, 0) + " °C")}
    ${row("bed", fmt(d.heater_bed?.temperature, 1) + " / " + fmt(d.heater_bed?.target, 0) + " °C")}
    ${row("part fan", fmt((d.fan?.speed ?? 0) * 100, 0) + " %")}
    ${row("hotend fan", fmt((d["heater_fan hotend_fan"]?.speed ?? 0) * 100, 0) + " %")}
    ${row("probe", d.probe?.last_query ? "TRIGGERED" : "open")}
    ${row("gantry level", d.quad_gantry_level?.applied ? "applied" : "not applied")}
    ${row("mesh", esc(d.bed_mesh?.profile_name) || "none")}
    ${row("job", esc(d.print_stats?.filename) || "idle")}
  </dl>`;
}

function drawEvents() {
  $("events").innerHTML = S.events.length ? S.events.map((e) =>
    `<div class="${/!!|error|shutdown/i.test(e.message) ? "err" : ""}"><time>${clock(e.ts)}</time>${
      e.message.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" }[c]))}</div>`).join("")
    : "nothing recorded in this window";
}

function drawQgl() {
  if (!S.qgl.length) { $("qgl").textContent = "no levelling runs recorded yet"; return; }
  const runs = {};
  S.qgl.forEach((r) => { (runs[r.run_id] ||= []).push(r); });
  const ids = Object.keys(runs).sort((a, b) => b - a).slice(0, 6);
  $("qgl").innerHTML = ids.map((id) => {
    const rows = runs[id].sort((a, b) => a.round - b.round);
    const last = rows[rows.length - 1];
    const ok = last.range_mm <= 0.02;
    return `<table><thead><tr><th>${stamp(rows[0].ts)}</th><th>range mm</th>
      <th>z0</th><th>z1</th><th>z2</th><th>z3</th></tr></thead><tbody>${
      rows.map((r) => `<tr class="${r === last ? (ok ? "pass" : "fail") : ""}">
        <td>round ${r.round}</td><td class="range">${fmt(r.range_mm, 4)}</td>
        <td>${fmt(r.z0, 3)}</td><td>${fmt(r.z1, 3)}</td><td>${fmt(r.z2, 3)}</td><td>${fmt(r.z3, 3)}</td></tr>`).join("")
      }</tbody></table>`;
  }).join("<div style='height:10px'></div>");
}

function render() { drawMap(); drawCharts(); drawMotors(); drawDetail(); drawEvents(); drawQgl(); }

/* ---------- data flow ---------- */
async function tickLive() {
  try {
    const r = await api("/api/live");
    S.sample = r.sample; S.at = null;
    const c = r.collector || {};
    $("conn-dot").className = "dot" + (c.connected ? " ok" : "");
    const d = S.sample?.data || {};
    $("state-line").textContent = c.connected
      ? `${d.print_stats?.state || "?"} · nozzle ${fmt(d.extruder?.temperature, 1)}° bed ${fmt(d.heater_bed?.temperature, 1)}° · recording ${c.polls} samples`
      : `printer not answering (${c.error || "offline"}) · ${c.polls} samples recorded before that`;
  } catch (e) { $("state-line").textContent = "scope server unreachable"; }
}

async function tickWindow() {
  const end = S.at || (Date.now() / 1000);
  const start = end - S.windowSec;
  try {
    const [s, e, q] = await Promise.all([
      api(`/api/series?from=${start}&to=${end}&points=700`),
      api(`/api/events?from=${start}&to=${end}`),
      api("/api/qgl"),
    ]);
    S.series = s.points; S.events = e.events; S.qgl = q.rows;
  } catch (err) { /* keep whatever we had */ }
}

async function setReplay(ts) {
  S.at = ts;
  const r = await api("/api/at?ts=" + ts);
  if (r.sample) S.sample = r.sample;
  await tickWindow();
  $("tl-now").textContent = stamp(ts);
  render();
}

async function refreshMeta() {
  S.meta = await api("/api/meta");
  const a = S.meta.first_ts, b = S.meta.last_ts || S.meta.server_time;
  if (a) {
    $("tl-start").textContent = stamp(a);
    $("tl-end").textContent = stamp(b);
    $("scrub").dataset.a = a; $("scrub").dataset.b = b;
  }
}

function setMode(live) {
  S.live = live;
  $("btn-live").classList.toggle("on", live);
  $("btn-replay").classList.toggle("on", !live);
  $("timeline").hidden = live;
  if (live) { S.at = null; }
  else { refreshMeta().then(() => setReplay(Number($("scrub").dataset.b || Date.now() / 1000))); }
}

$("btn-live").onclick = () => setMode(true);
$("btn-replay").onclick = () => setMode(false);
$("btn-now").onclick = () => { $("scrub").value = 1000; setReplay(Number($("scrub").dataset.b)); };
$("scrub").oninput = (e) => {
  const a = Number(e.target.dataset.a), b = Number(e.target.dataset.b);
  if (a && b) setReplay(a + (b - a) * (e.target.value / 1000));
};
document.querySelectorAll("[data-back]").forEach((b) => b.onclick = () => {
  const end = Number($("scrub").dataset.b || Date.now() / 1000);
  setMode(false);
  setTimeout(() => setReplay(end - Number(b.dataset.back)), 50);
});
window.addEventListener("resize", () => drawCharts());

(async function start() {
  await refreshMeta();
  await tickLive(); await tickWindow(); render();
  setInterval(async () => { if (S.live) { await tickLive(); render(); } }, 500);
  setInterval(async () => { if (S.live) { await tickWindow(); await refreshMeta(); } }, 5000);
})();

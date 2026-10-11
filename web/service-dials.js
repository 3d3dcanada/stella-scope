// Stella Scope instrument dials — canvas gauges for the restyled cockpit.
// Vanilla canvas 2D, no dependencies. Colors resolve from the CSS theme so the
// design system stays single-sourced; hex fallbacks keep the gauges legible if
// the stylesheet fails to load. No DOM access at module scope (import must be
// side-effect free for the node parse check the build runs).

const SWEEP = 1.5 * Math.PI; // 270° arc dial
const RING_SWEEP = (300 / 360) * 2 * Math.PI;
let fontsHooked = false;
const registry = new Set();
try {
  if (typeof window !== 'undefined') window.__stellaDials = registry;
} catch {}

function theme(name, fallback) {
  try {
    const v = getComputedStyle(document.body).getPropertyValue(name).trim();
    return v || fallback;
  } catch {
    return fallback;
  }
}

// One eased number per instrument: a single rAF loop runs while the displayed
// value is still catching up to the reported one, then parks. set() during a
// loop only moves the target — no stacked animators.
class Eased {
  constructor(value) {
    this.shown = value;
    this.actual = value;
    this.raf = 0;
  }
  set(v) {
    this.actual = Number.isFinite(v) ? v : this.shown;
    if (!this.raf) this.raf = requestAnimationFrame(this.step);
  }
  step = () => {
    const d = this.actual - this.shown;
    if (Math.abs(d) < 0.0005) {
      this.shown = this.actual;
      this.raf = 0;
    } else {
      this.shown += d * 0.16;
      this.raf = requestAnimationFrame(this.step);
    }
    this.onFrame && this.onFrame();
  };
  destroy() {
    if (this.raf) cancelAnimationFrame(this.raf);
    this.raf = 0;
  }
}

function fitCanvas(canvas) {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const w = canvas.clientWidth || 0;
  const h = canvas.clientHeight || 0;
  if (!w || !h) return null;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
  }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { ctx, w, h };
}

function hexToRgba(hex, a) {
  const m = /^#?([0-9a-f]{6})$/i.exec(String(hex).trim());
  if (!m) return `rgba(4,217,196,${a})`;
  const n = parseInt(m[1], 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}

function spaced(text) {
  return String(text ?? '').toUpperCase().split('').join('\u200a');
}

function hookFonts() {
  if (fontsHooked) return;
  fontsHooked = true;
  if (typeof document !== 'undefined' && document.fonts?.ready) {
    document.fonts.ready.then(() => {
      for (const d of registry) d.draw();
    }).catch(() => {});
  }
}

function labelFont(ctx, px) {
  ctx.font = `500 ${Math.max(11, px)}px "Geist Mono", ui-monospace, monospace`;
  try { ctx.letterSpacing = '0.16em'; } catch {}
}
function numFont(ctx, px, weight = 600) {
  ctx.font = `${weight} ${Math.max(11, px)}px "Geist Mono", ui-monospace, monospace`;
  try { ctx.letterSpacing = '0em'; } catch {}
}

function drawArcGauge(d) {
  const f = fitCanvas(d.canvas);
  if (!f) return;
  const { ctx, w, h } = f;
  const cx = w / 2;
  const cy = h / 2 - 6;
  // Reserve a bottom label zone so nothing clips: arc radius leaves ~26px below.
  const r = Math.max(1, Math.min(w, h) / 2 - 26);
  const start = 0.75 * Math.PI;
  const accent = theme(d.opts.accent || '--teal', '#04D9C4');
  const targetColor = theme(d.opts.targetColor || '--orange', '#FF6A00');
  const span = Math.max(1e-9, (d.opts.max ?? 1) - (d.opts.min ?? 0));
  const frac = !d.known ? 0 : Math.max(0, Math.min(1, (d.shown - (d.opts.min ?? 0)) / span));
  const over = d.actual > (d.opts.max ?? 1);
  ctx.clearRect(0, 0, w, h);
  // Track.
  ctx.lineCap = 'round';
  ctx.lineWidth = 8;
  ctx.strokeStyle = theme('--track', 'rgba(255,255,255,.07)');
  ctx.beginPath();
  ctx.arc(cx, cy, r, start, start + SWEEP);
  ctx.stroke();
  // Ticks under the arc: ten quiet majors inside the track.
  ctx.lineWidth = 1;
  ctx.strokeStyle = theme('--line', 'rgba(255,255,255,.14)');
  for (let i = 0; i <= 10; i++) {
    const a = start + (SWEEP * i) / 10;
    const c = Math.cos(a), s = Math.sin(a);
    ctx.beginPath();
    ctx.moveTo(cx + c * (r - 11), cy + s * (r - 11));
    ctx.lineTo(cx + c * (r - 17), cy + s * (r - 17));
    ctx.stroke();
  }
  // Value arc with a controlled neon glow.
  if (frac > 0.004) {
    ctx.strokeStyle = over ? theme('--magenta', '#FF2D92') : accent;
    ctx.shadowColor = hexToRgba(over ? theme('--magenta', '#FF2D92') : accent, 0.55);
    ctx.shadowBlur = 14;
    ctx.beginPath();
    ctx.arc(cx, cy, r, start, start + SWEEP * frac);
    ctx.stroke();
    ctx.shadowBlur = 0;
    ctx.shadowColor = 'transparent';
  }
  // Target marker rides on top of the arc.
  if (Number.isFinite(d.target)) {
    const t = Math.max(0, Math.min(1, (d.target - (d.opts.min ?? 0)) / span));
    const a = start + SWEEP * t;
    const c = Math.cos(a), s = Math.sin(a);
    ctx.lineWidth = 2;
    ctx.strokeStyle = targetColor;
    ctx.shadowColor = hexToRgba(targetColor, 0.6);
    ctx.shadowBlur = 8;
    ctx.beginPath();
    ctx.moveTo(cx + c * (r - 7), cy + s * (r - 7));
    ctx.lineTo(cx + c * (r + 7), cy + s * (r + 7));
    ctx.stroke();
    ctx.shadowBlur = 0;
    ctx.shadowColor = 'transparent';
  }
  // Center readout, stacked inside the arc; label parks in the reserved bottom zone.
  const valueColor = over ? theme('--magenta', '#FF2D92') : theme('--text', '#f2f5f3');
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  numFont(ctx, Math.max(17, r * 0.48));
  ctx.fillStyle = valueColor;
  ctx.fillText(
    d.known ? d.shown.toFixed(d.opts.digits ?? 1) : '—',
    cx, cy - r * 0.22
  );
  if (d.opts.unit) {
    numFont(ctx, 9, 500);
    ctx.fillStyle = theme('--muted', '#879396');
    ctx.fillText(String(d.opts.unit), cx, cy + r * 0.24);
  }
  if (d.status) {
    labelFont(ctx, 8);
    ctx.fillStyle = theme(d.status.color || '--muted', '#879396');
    ctx.fillText(spaced(d.status.text), cx, cy + r * 0.62);
  }
  if (d.opts.label) {
    labelFont(ctx, 8);
    ctx.fillStyle = theme('--muted', '#879396');
    ctx.fillText(spaced(d.opts.label), cx, h - 9);
  }
}

class Dial {
  constructor(canvas, opts) {
    this.canvas = canvas;
    this.opts = opts || {};
    this.target = Number.isFinite(opts?.target) ? opts.target : null;
    this.status = null;
    this.known = Number.isFinite(opts?.value);
    this.eased = new Eased(opts?.value ?? opts?.min ?? 0);
    this.eased.onFrame = () => this.draw();
    this.ro = new ResizeObserver(() => this.draw());
    this.ro.observe(canvas);
    registry.add(this);
    hookFonts();
    this.draw();
  }
  get shown() { return this.eased.shown; }
  get actual() { return this.eased.actual; }
  set(value, target, status) {
    this.known = Number.isFinite(value);
    this.canvas.setAttribute('aria-label', `${this.opts.label}: ${this.known ? value : 'Unknown'}`);
    if (target !== undefined && target !== null) this.target = target;
    else if (target === null) this.target = null;
    if (status !== undefined) this.status = status;
    this.eased.set(value);
    this.draw();
  }
  draw() { drawArcGauge(this); }
  destroy() {
    this.eased.destroy();
    this.ro.disconnect();
    registry.delete(this);
  }
}

class Bar {
  constructor(canvas, opts) {
    this.canvas = canvas;
    this.opts = opts || {};
    this.known = Number.isFinite(opts?.value);
    this.eased = new Eased(opts?.value ?? opts?.min ?? 0);
    this.eased.onFrame = () => this.draw();
    this.ro = new ResizeObserver(() => this.draw());
    this.ro.observe(canvas);
    registry.add(this);
    hookFonts();
    this.draw();
  }
  get shown() { return this.eased.shown; }
  set(value) { this.known = Number.isFinite(value); this.canvas.setAttribute('aria-label', `${this.opts.label}: ${this.known ? value : 'Unknown'}`); this.eased.set(value); this.draw(); }
  draw() {
    const f = fitCanvas(this.canvas);
    if (!f) return;
    const { ctx, w, h } = f;
    const accent = theme(this.opts.accent || '--teal', '#04D9C4');
    const span = Math.max(1e-9, (this.opts.max ?? 1) - (this.opts.min ?? 0));
    const frac = !this.known ? 0 : Math.max(0, Math.min(1, (this.shown - (this.opts.min ?? 0)) / span));
    const over = this.eased.actual > (this.opts.max ?? 1);
    const fill = over ? theme('--magenta', '#FF2D92') : accent;
    ctx.clearRect(0, 0, w, h);
    const labelY = 12;
    if (this.opts.label) {
      labelFont(ctx, 9);
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = theme('--muted', '#879396');
      ctx.fillText(spaced(this.opts.label), 2, labelY, w-4);
    }
    const trackY = labelY + 14;
    const barW = Math.round(w * 0.6);
    // Tick marks under the fill: ten quiet majors, instrument feel.
    ctx.lineWidth = 1;
    ctx.strokeStyle = theme('--line', 'rgba(255,255,255,.13)');
    for (let i = 0; i <= 10; i++) {
      const tx = 4 + barW * i / 10;
      ctx.beginPath();
      ctx.moveTo(tx, trackY + 7);
      ctx.lineTo(tx, trackY + 12);
      ctx.stroke();
    }
    ctx.lineCap = 'round';
    ctx.lineWidth = 8;
    ctx.strokeStyle = theme('--track', 'rgba(255,255,255,.07)');
    ctx.beginPath();
    ctx.moveTo(4, trackY);
    ctx.lineTo(4 + barW, trackY);
    ctx.stroke();
    if (frac > 0.004) {
      ctx.strokeStyle = fill;
      ctx.shadowColor = hexToRgba(fill, 0.55);
      ctx.shadowBlur = 12;
      ctx.beginPath();
      ctx.moveTo(4, trackY);
      ctx.lineTo(4 + barW * frac, trackY);
      ctx.stroke();
      ctx.shadowBlur = 0;
      ctx.shadowColor = 'transparent';
    }
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    numFont(ctx, 22);
    ctx.fillStyle = over ? theme('--magenta', '#FF2D92') : theme('--text', '#f2f5f3');
    ctx.fillText(this.known ? this.shown.toFixed(this.opts.digits ?? 1) : '—', barW + 16, trackY - 2);
    if (this.opts.unit) {
      numFont(ctx, 10, 500);
      ctx.fillStyle = theme('--muted', '#879396');
      ctx.fillText(String(this.opts.unit), barW + 16, trackY + 16);
    }
  }
  destroy() {
    this.eased.destroy();
    this.ro.disconnect();
    registry.delete(this);
  }
}

class Ring {
  constructor(canvas, opts) {
    this.canvas = canvas;
    this.opts = opts || {};
    this.known = Number.isFinite(opts?.value);
    this.eased = new Eased(0);
    this.eased.onFrame = () => this.draw();
    this.ro = new ResizeObserver(() => this.draw());
    this.ro.observe(canvas);
    registry.add(this);
    hookFonts();
    this.draw();
  }
  get shown() { return this.eased.shown; }
  set(fraction) { this.known = Number.isFinite(fraction); this.canvas.setAttribute('aria-label', `${this.opts.label}: ${this.known ? Math.round(fraction*100)+'%' : 'Unknown'}`); if(this.known)this.eased.set(Math.max(0, Math.min(1, fraction))); this.draw(); }
  draw() {
    const f = fitCanvas(this.canvas);
    if (!f) return;
    const { ctx, w, h } = f;
    const cx = w / 2, cy = h / 2 - 6;
    const r = Math.max(1, Math.min(w, h) / 2 - 22);
    const start = 0.72 * Math.PI;
    const accent = theme(this.opts.accent || '--teal', '#04D9C4');
    ctx.clearRect(0, 0, w, h);
    ctx.lineCap = 'round';
    ctx.lineWidth = 6;
    ctx.strokeStyle = theme('--track', 'rgba(255,255,255,.07)');
    ctx.beginPath();
    ctx.arc(cx, cy, r, start, start + RING_SWEEP);
    ctx.stroke();
    const frac = this.known ? Math.max(0, Math.min(1, this.shown)) : 0;
    if (frac > 0.004) {
      ctx.strokeStyle = accent;
      ctx.shadowColor = hexToRgba(accent, 0.55);
      ctx.shadowBlur = 12;
      ctx.beginPath();
      ctx.arc(cx, cy, r, start, start + RING_SWEEP * frac);
      ctx.stroke();
      ctx.shadowBlur = 0;
      ctx.shadowColor = 'transparent';
    }
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    numFont(ctx, Math.max(15, r * 0.55));
    ctx.fillStyle = theme('--text', '#f2f5f3');
    ctx.fillText(this.known ? `${Math.round(frac * 100)}%` : '—', cx, cy);
    if (this.opts.label) {
      labelFont(ctx, 8);
      ctx.fillStyle = theme('--muted', '#879396');
      ctx.fillText(spaced(this.opts.label), cx, h - 9);
    }
  }
  destroy() {
    this.eased.destroy();
    this.ro.disconnect();
    registry.delete(this);
  }
}

export function createDial(canvas, opts) { return new Dial(canvas, opts); }
export function createBar(canvas, opts) { return new Bar(canvas, opts); }
export function createRing(canvas, opts) { return new Ring(canvas, opts); }

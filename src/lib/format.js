// Pure formatting + display helpers shared by the editor UI. Kept out of the
// page component so they can be unit-tested in isolation (the component itself
// has no test harness).

/**
 * Format a time in seconds as MM:SS.cc (centiseconds). Negative / non-finite
 * inputs clamp to zero so a timeline readout never shows NaN.
 * @param {number} t
 * @returns {string}
 */
export function fmt(t) {
  if (!isFinite(t) || t < 0) t = 0;
  const m = Math.floor(t / 60);
  const s = Math.floor(t % 60);
  const cs = Math.floor((t % 1) * 100);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(cs).padStart(2, "0")}`;
}

/**
 * Format a byte count as MB, switching to GB just under 1 GB so ~1 GB clips
 * never read as "1000+ MB". Falsy / zero sizes render as an empty string.
 * @param {number|null|undefined} b
 * @returns {string}
 */
export function fmtSize(b) {
  if (!b) return "";
  const mb = b / (1024 * 1024);
  return mb >= 1000 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`;
}

/**
 * Strip a path down to its bare filename (handles both `/` and `\`). Used for
 * the editor title and the result toasts. A null/empty path yields "".
 * @param {string|null|undefined} p
 * @returns {string}
 */
export function baseName(p) {
  return (p || "").split(/[\\/]/).pop() || "";
}

/**
 * Build the waveform as a single SVG path string (one DOM node) instead of a
 * <rect> per bucket. Each bucket is a centred bar: x=i+0.15, width 0.7, height
 * max(1, p*92) at y=50-p*46, matching the 0..100 viewBox the markup uses. A
 * null/empty waveform yields an empty path. Pure.
 * @param {number[]|null} waveform
 * @returns {string}
 */
export function waveformPath(waveform) {
  if (!waveform) return "";
  let d = "";
  for (let i = 0; i < waveform.length; i++) {
    const p = waveform[i];
    const h = Math.max(1, p * 92);
    d += `M${i + 0.15} ${50 - p * 46}h0.7v${h}h-0.7z`;
  }
  return d;
}

/**
 * Live preview of the output file name for a naming scheme. Mirrors the Rust
 * `apply_naming_scheme` resolver (display only — the backend stays the source
 * of truth): substitutes {name}/{action}, strips illegal filename chars, and
 * falls back to `{name}_{action}` when the scheme collapses to nothing. Pure.
 * @param {string|null|undefined} scheme
 * @param {string} name
 * @param {string} action
 * @param {string} ext
 * @returns {string}
 */
export function previewName(scheme, name, action, ext) {
  const tmpl = (scheme || "").trim() || "{name}_{action}";
  const built = tmpl.replace(/\{name\}/g, name).replace(/\{action\}/g, action);
  const cleaned = built.replace(/[<>:"/\\|?*]/g, "").trim();
  return `${cleaned || `${name}_${action}`}.${ext}`;
}

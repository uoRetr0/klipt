<script>
  // The borderless-window chrome: brand mark, draggable region, and the
  // minimize / maximize / close controls. Self-contained — it talks to the
  // Tauri window directly, so it carries no props. First child carved out of
  // the +page.svelte monolith (the maximize path stays on the backend
  // `toggle_maximize` command, which applies Klipt's custom DWM chrome).
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { invoke } from "@tauri-apps/api/core";

  const appWindow = getCurrentWindow();
</script>

<div class="titlebar" data-tauri-drag-region>
  <div class="tb-brand" data-tauri-drag-region>
    <span class="mark">K</span><span class="word">klipt</span>
  </div>
  <div class="tb-drag" data-tauri-drag-region></div>
  <div class="tb-controls">
    <button class="tb-btn" onclick={() => appWindow.minimize()} aria-label="Minimize">
      <svg width="11" height="11" viewBox="0 0 11 11"><rect x="1" y="5" width="9" height="1" fill="currentColor"/></svg>
    </button>
    <button class="tb-btn" onclick={() => invoke("toggle_maximize")} aria-label="Maximize">
      <svg width="11" height="11" viewBox="0 0 11 11"><rect x="1.5" y="1.5" width="8" height="8" fill="none" stroke="currentColor" stroke-width="1"/></svg>
    </button>
    <button class="tb-btn danger" onclick={() => appWindow.close()} aria-label="Close">
      <svg width="11" height="11" viewBox="0 0 11 11"><path d="M1 1 L10 10 M10 1 L1 10" stroke="currentColor" stroke-width="1.2"/></svg>
    </button>
  </div>
</div>

<style>
  .titlebar { height: 28px; flex: 0 0 28px; display: flex; align-items: center; padding-left: 12px; background: var(--bg); border-bottom: 1px solid var(--border); }
  .tb-brand { display: flex; align-items: center; gap: 7px; }
  .mark { width: 15px; height: 15px; display: grid; place-items: center; background: linear-gradient(180deg, #161618, #0a0a0c); color: var(--accent); border: 1px solid rgba(255,255,255,0.09); border-radius: var(--r-xs); font-weight: 800; font-size: 10px; font-family: var(--display); }
  .word { font-family: var(--display); font-weight: 600; letter-spacing: 0.01em; font-size: 12px; }
  .tb-drag { flex: 1; height: 100%; }
  .tb-controls { display: flex; height: 100%; }
  .tb-btn { width: 40px; height: 28px; display: grid; place-items: center; background: transparent; border: 0; color: var(--muted); cursor: pointer; transition: background 0.12s, color 0.12s; }
  .tb-btn:hover { background: var(--panel-2); color: var(--text); }
  .tb-btn.danger:hover { background: #b4232a; color: #fff; }
</style>

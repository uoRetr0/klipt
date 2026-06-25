<script module>
  // Per-instance counter so option ids are unique across multiple dropdowns —
  // aria-activedescendant has to point at a globally-unique id.
  let uid = 0;
</script>

<script>
  // A compact, on-brand monochrome dropdown. Keyboard + click-out aware.
  // options: [{ value, label, count? }]
  // custom (optional): { min, max, step?, unit? } — adds a numeric "Custom" row
  //   inside the popup so a value off the preset list can still be entered. The
  //   trigger then shows e.g. "37 MB" for that custom value.
  let {
    options = [],
    value = $bindable(),
    label = "",
    ariaLabel = "",
    custom = /** @type {{min:number,max:number,step?:number,unit?:string}|null} */ (null),
  } = $props();

  let open = $state(false);
  let active = $state(-1);
  // Open upward when the popup would otherwise be clipped below the viewport
  // (these pickers live in a bar near the bottom of the window).
  let dropUp = $state(false);
  let rootEl = /** @type {HTMLDivElement | null} */ ($state(null));

  const baseId = `dd-${(uid += 1)}`;
  const menuId = `${baseId}-menu`;
  /** @param {number} i */
  const optId = (i) => `${baseId}-opt-${i}`;

  /** @param {any} v */
  const fmtCustom = (v) => `${v}${custom?.unit ? ` ${custom.unit}` : ""}`;
  // True when the live value isn't one of the presets (so it's a custom entry).
  const isCustom = $derived(custom != null && !options.some((o) => o.value === value));

  const selected = $derived(
    options.find((o) => o.value === value) ??
      (custom != null ? { value, label: fmtCustom(value) } : options[0]),
  );

  /** Choose open direction so the popup stays on screen: drop up when there
   *  isn't room below and there's more room above. */
  function decideDirection() {
    const rect = rootEl?.getBoundingClientRect();
    if (!rect) return;
    // Estimate the menu height: ~32px per row (+ the custom row), capped at the
    // menu's max-height (it scrolls past that).
    const rows = options.length + (custom != null ? 1 : 0);
    const needed = Math.min(286, rows * 32 + 12);
    const below = window.innerHeight - rect.bottom;
    const above = rect.top;
    dropUp = below < needed + 8 && above > below;
  }
  function toggle() {
    open = !open;
    if (open) {
      decideDirection();
      active = Math.max(0, options.findIndex((o) => o.value === value));
    }
  }
  /** @param {any} v */
  function choose(v) {
    value = v;
    open = false;
    /** @type {HTMLElement | null | undefined} */ (rootEl?.querySelector(".dd-trigger"))?.focus();
  }
  /** Clamp + apply a typed custom value without closing the popup. */
  function onCustomInput(/** @type {Event} */ e) {
    if (!custom) return;
    const raw = /** @type {HTMLInputElement} */ (e.target).value;
    if (raw === "") return; // let the field be cleared mid-edit
    const n = Math.min(custom.max, Math.max(custom.min, Number(raw)));
    if (!Number.isNaN(n)) value = n;
  }
  /** Keep listbox arrow-nav from hijacking typing in the custom field. */
  function onCustomKey(/** @type {KeyboardEvent} */ e) {
    if (e.key === "Enter") {
      e.preventDefault();
      open = false;
      /** @type {HTMLElement | null | undefined} */ (rootEl?.querySelector(".dd-trigger"))?.focus();
    } else if (e.key !== "Escape") {
      e.stopPropagation();
    }
  }
  /** @param {KeyboardEvent} e */
  function onKey(e) {
    if (!open) {
      if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        toggle();
      }
      return;
    }
    if (e.key === "Escape") {
      open = false;
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      active = (active + 1) % options.length;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = (active - 1 + options.length) % options.length;
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      if (options[active]) choose(options[active].value);
    } else if (e.key === "Tab") {
      open = false;
    }
  }
  /** @param {PointerEvent} e */
  function onWindowDown(e) {
    if (open && rootEl && !rootEl.contains(/** @type {Node} */ (e.target))) open = false;
  }
</script>

<svelte:window onpointerdown={onWindowDown} />

<div class="dd" bind:this={rootEl} onkeydown={onKey}>
  <button
    class="dd-trigger"
    class:open
    onclick={toggle}
    role="combobox"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={open ? menuId : undefined}
    aria-activedescendant={open && active >= 0 ? optId(active) : undefined}
    aria-label={ariaLabel || label}
  >
    {#if label}<span class="dd-lbl">{label}</span>{/if}
    <span class="dd-val">{selected?.label ?? ""}</span>
    <svg class="dd-caret" width="10" height="10" viewBox="0 0 10 10" fill="none"
      ><path d="M2.5 4 L5 6.5 L7.5 4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" /></svg
    >
  </button>

  {#if open}
    <ul class="dd-menu" class:up={dropUp} id={menuId} role="listbox" tabindex="-1" aria-label={ariaLabel || label}>
      {#each options as o, i (o.value)}
        <li>
          <button
            class="dd-opt"
            id={optId(i)}
            class:on={o.value === value}
            class:active={i === active}
            role="option"
            aria-selected={o.value === value}
            onclick={() => choose(o.value)}
            onpointerenter={() => (active = i)}
          >
            <span class="dd-opt-label">{o.label}</span>
            {#if o.count != null}<span class="dd-count">{o.count}</span>{/if}
            <!-- Always render the check slot so the count keeps its position
                 whether or not the row is selected. -->
            <span class="dd-check" aria-hidden="true">
              {#if o.value === value}
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none"
                  ><path d="M2.5 6.2 L5 8.5 L9.5 3.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg
                >
              {/if}
            </span>
          </button>
        </li>
      {/each}
      {#if custom}
        <li class="dd-customrow" class:on={isCustom}>
          <span class="dd-opt-label">Custom</span>
          <input
            class="dd-custominput mono"
            type="number"
            min={custom.min}
            max={custom.max}
            step={custom.step ?? 1}
            value={isCustom ? value : ""}
            placeholder="—"
            oninput={onCustomInput}
            onkeydown={onCustomKey}
            aria-label={`Custom ${label || ariaLabel}`}
          />
          {#if custom.unit}<span class="dd-customunit">{custom.unit}</span>{/if}
          <span class="dd-check" aria-hidden="true">
            {#if isCustom}
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none"
                ><path d="M2.5 6.2 L5 8.5 L9.5 3.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg
              >
            {/if}
          </span>
        </li>
      {/if}
    </ul>
  {/if}
</div>

<style>
  .dd { position: relative; display: inline-block; }

  .dd-trigger {
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--border-2);
    border-radius: var(--r-sm);
    padding: 7px 11px;
    min-width: 132px;
    transition: background 0.14s, border-color 0.14s;
  }
  .dd-trigger:hover { background: var(--panel-2); border-color: #41414a; }
  .dd-trigger.open { background: var(--panel-2); border-color: #41414a; }
  .dd-lbl {
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--faint);
    flex: 0 0 auto;
  }
  .dd-val {
    flex: 1;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dd-caret { color: var(--muted); flex: 0 0 auto; transition: transform 0.16s; }
  .dd-trigger.open .dd-caret { transform: rotate(180deg); }

  .dd-menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 30;
    margin: 0;
    padding: 5px;
    list-style: none;
    min-width: 100%;
    max-height: 280px;
    overflow-y: auto;
    background: var(--panel-2);
    border: 1px solid var(--border-2);
    border-radius: var(--r-md);
    box-shadow: 0 18px 44px -16px rgba(0, 0, 0, 0.85), inset 0 1px 0 rgba(255, 255, 255, 0.04);
    animation: dd-rise 0.14s cubic-bezier(0.16, 1, 0.3, 1);
  }
  /* Flip above the trigger when there isn't room below (bottom-of-window bars). */
  .dd-menu.up {
    top: auto;
    bottom: calc(100% + 6px);
    box-shadow: 0 -18px 44px -16px rgba(0, 0, 0, 0.85), inset 0 1px 0 rgba(255, 255, 255, 0.04);
    animation: dd-drop 0.14s cubic-bezier(0.16, 1, 0.3, 1);
  }
  .dd-menu li { margin: 0; }
  .dd-opt {
    width: 100%;
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: left;
    color: var(--muted);
    background: transparent;
    border: 0;
    border-radius: var(--r-sm);
    padding: 7px 9px;
    white-space: nowrap;
  }
  .dd-opt.active { background: var(--panel-3); color: var(--text); }
  .dd-opt.on { color: var(--text); }
  .dd-opt-label { flex: 1; overflow: hidden; text-overflow: ellipsis; }
  .dd-count {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--faint);
    flex: 0 0 auto;
  }
  /* Fixed-width slot, present on every row, so selecting a row doesn't nudge
     the count left to make room for the check. */
  .dd-check {
    width: 12px;
    height: 12px;
    display: grid;
    place-items: center;
    color: var(--text);
    flex: 0 0 auto;
  }

  /* Custom numeric entry row — mirrors .dd-opt layout so it lines up with the
     preset rows above it. */
  .dd-customrow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 9px;
    margin-top: 3px;
    border-top: 1px solid var(--border-2);
    color: var(--muted);
    font-size: 12.5px;
  }
  .dd-customrow.on { color: var(--text); }
  .dd-customrow .dd-opt-label { flex: 0 0 auto; }
  .dd-custominput {
    flex: 1 1 auto;
    /* Keep the field comfortably wide so typed values aren't clipped into a
       thin vertical sliver; the menu grows to fit it. */
    min-width: 96px;
    font: inherit;
    font-size: 12.5px;
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--border-2);
    border-radius: var(--r-sm);
    padding: 5px 9px;
    text-align: right;
    -moz-appearance: textfield;
    appearance: textfield;
  }
  /* Drop the spin buttons — they ate horizontal space and clipped the value. */
  .dd-custominput::-webkit-inner-spin-button,
  .dd-custominput::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }
  .dd-custominput:focus { outline: none; border-color: #52525c; }
  .dd-customunit { font-size: 10.5px; color: var(--faint); flex: 0 0 auto; }

  @keyframes dd-rise {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
  @keyframes dd-drop {
    from { opacity: 0; transform: translateY(4px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>

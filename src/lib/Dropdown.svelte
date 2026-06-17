<script>
  // A compact, on-brand monochrome dropdown. Keyboard + click-out aware.
  // options: [{ value, label, count? }]
  let { options = [], value = $bindable(), label = "", ariaLabel = "" } = $props();

  let open = $state(false);
  let active = $state(-1);
  let rootEl = $state(null);

  const selected = $derived(options.find((o) => o.value === value) ?? options[0]);

  function toggle() {
    open = !open;
    if (open) active = Math.max(0, options.findIndex((o) => o.value === value));
  }
  function choose(v) {
    value = v;
    open = false;
    rootEl?.querySelector(".dd-trigger")?.focus();
  }
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
  function onWindowDown(e) {
    if (open && rootEl && !rootEl.contains(e.target)) open = false;
  }
</script>

<svelte:window onpointerdown={onWindowDown} />

<div class="dd" bind:this={rootEl} onkeydown={onKey}>
  <button
    class="dd-trigger"
    class:open
    onclick={toggle}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label={ariaLabel || label}
  >
    {#if label}<span class="dd-lbl">{label}</span>{/if}
    <span class="dd-val">{selected?.label ?? ""}</span>
    <svg class="dd-caret" width="10" height="10" viewBox="0 0 10 10" fill="none"
      ><path d="M2.5 4 L5 6.5 L7.5 4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" /></svg
    >
  </button>

  {#if open}
    <ul class="dd-menu" role="listbox" tabindex="-1" aria-label={ariaLabel || label}>
      {#each options as o, i (o.value)}
        <li>
          <button
            class="dd-opt"
            class:on={o.value === value}
            class:active={i === active}
            role="option"
            aria-selected={o.value === value}
            onclick={() => choose(o.value)}
            onpointerenter={() => (active = i)}
          >
            <span class="dd-opt-label">{o.label}</span>
            {#if o.count != null}<span class="dd-count">{o.count}</span>{/if}
            {#if o.value === value}
              <svg class="dd-check" width="12" height="12" viewBox="0 0 12 12" fill="none"
                ><path d="M2.5 6.2 L5 8.5 L9.5 3.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" /></svg
              >
            {/if}
          </button>
        </li>
      {/each}
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
    border-radius: 9px;
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
    border-radius: 11px;
    box-shadow: 0 18px 44px -16px rgba(0, 0, 0, 0.85), inset 0 1px 0 rgba(255, 255, 255, 0.04);
    animation: dd-rise 0.14s cubic-bezier(0.16, 1, 0.3, 1);
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
    border-radius: 7px;
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
  .dd-check { color: var(--text); flex: 0 0 auto; }

  @keyframes dd-rise {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>

<script>
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import Dropdown from "$lib/Dropdown.svelte";

  const appWindow = getCurrentWindow();

  // --- state ------------------------------------------------------------
  let watchedFolder = $state(null);
  let recentClips = $state([]);
  let gameFilter = $state("all");
  let dateFilter = $state("all");

  let clip = $state(null);
  let videoSrc = $state(null);
  let videoEl = $state(null);
  let timelineEl = $state(null);

  let duration = $state(0);
  let currentTime = $state(0);
  let inPoint = $state(0);
  let outPoint = $state(0);
  let playing = $state(false);

  // playback-preview volume (does NOT affect exported audio)
  let volume = $state(1);
  let muted = $state(false);

  // lazily-rendered poster thumbnails, keyed by clip path
  let thumbs = $state({});
  const thumbReq = new Set();
  const thumbQueue = [];
  let thumbActive = 0;
  // requestAnimationFrame handle for the precise out-point stop while playing.
  let playRaf = 0;
  const THUMB_CONCURRENCY = 3;

  // Monotonic token so only the most recent loadClip() may commit its result;
  // a newer load (or closeClip) supersedes any still-in-flight probe.
  let loadGen = 0;

  let activeHandle = $state(null);
  let dragOver = $state(false);
  let busy = $state(false);
  let busyLabel = $state("");
  let toast = $state(null);

  // export options
  let mode = $state("lossless"); // 'lossless' | 'compress'
  let compressBy = $state("size"); // 'size' | 'quality'
  let targetMb = $state(25);
  let quality = $state("medium");
  let outputName = $state("");

  const SIZE_PRESETS = [10, 25, 50];
  const selLength = $derived(Math.max(0, outPoint - inPoint));
  const baseStem = $derived(clip ? clip.name.replace(/\.[^.]+$/, "") : "");
  const defaultStem = $derived(mode === "compress" ? `${baseStem}_small` : `${baseStem}_trim`);
  const outExt = $derived(mode === "compress" ? "mp4" : clip ? clip.name.split(".").pop() : "mp4");

  const games = $derived.by(() => {
    const m = new Map();
    for (const c of recentClips) {
      const g = c.game || "Other";
      m.set(g, (m.get(g) || 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  });
  const filteredClips = $derived.by(() => {
    const now = Date.now() / 1000;
    const spans = { today: 86400, "7d": 7 * 86400, "30d": 30 * 86400 };
    return recentClips.filter((c) => {
      if (gameFilter !== "all" && (c.game || "Other") !== gameFilter) return false;
      if (dateFilter !== "all" && now - c.modified > spans[dateFilter]) return false;
      return true;
    });
  });

  const gameOptions = $derived([
    { value: "all", label: "All games", count: recentClips.length },
    ...games.map(([g, n]) => ({ value: g, label: g, count: n })),
  ]);
  const DATE_OPTIONS = [
    { value: "all", label: "All time" },
    { value: "today", label: "Today" },
    { value: "7d", label: "Last 7 days" },
    { value: "30d", label: "Last 30 days" },
  ];

  // Reset a filter if its selected value disappears (e.g. after switching folders).
  $effect(() => {
    if (gameFilter !== "all" && !games.some(([g]) => g === gameFilter)) gameFilter = "all";
  });

  // --- thumbnails (lazy, concurrency-capped) ---------------------------
  // Request a clip's thumbnail only once its card scrolls into view. The
  // rootMargin prefetches a little below the fold so thumbs are usually ready
  // by the time the card is fully visible. Once enqueued we stop observing —
  // enqueueThumb dedups via thumbReq, so re-observing would be a no-op anyway.
  /** @param {Element} node @param {string} path */
  function thumbOnVisible(node, path) {
    let current = path;
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            enqueueThumb(current);
            io.unobserve(node);
          }
        }
      },
      { rootMargin: "200px" },
    );
    io.observe(node);
    return {
      /** @param {string} next */
      update(next) {
        current = next;
      },
      destroy() {
        io.disconnect();
      },
    };
  }
  function enqueueThumb(path) {
    if (thumbReq.has(path)) return;
    thumbReq.add(path);
    thumbQueue.push(path);
    pumpThumbs();
  }
  function pumpThumbs() {
    while (thumbActive < THUMB_CONCURRENCY && thumbQueue.length) {
      const path = thumbQueue.shift();
      thumbActive++;
      invoke("clip_thumbnail", { path })
        .then((p) => (thumbs[path] = convertFileSrc(p)))
        .catch(() => {})
        .finally(() => {
          thumbActive--;
          pumpThumbs();
        });
    }
  }

  // --- volume (preview only) -------------------------------------------
  $effect(() => {
    if (videoEl) {
      videoEl.volume = volume;
      videoEl.muted = muted || volume === 0;
    }
  });
  function toggleMute() {
    muted = !(muted || volume === 0);
    if (!muted && volume === 0) volume = 0.5;
  }
  function onVolInput() {
    if (volume > 0) muted = false;
    try { localStorage.setItem("klipt:volume", String(volume)); } catch {}
  }

  // --- helpers ----------------------------------------------------------
  function fmt(t) {
    if (!isFinite(t) || t < 0) t = 0;
    const m = Math.floor(t / 60);
    const s = Math.floor(t % 60);
    const cs = Math.floor((t % 1) * 100);
    return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(cs).padStart(2, "0")}`;
  }
  function fmtSize(b) {
    if (!b) return "";
    const mb = b / (1024 * 1024);
    return mb >= 1024 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`;
  }
  function pct(t) {
    return duration > 0 ? (t / duration) * 100 : 0;
  }

  // --- folder / recent clips -------------------------------------------
  async function loadSettings() {
    try {
      const s = await invoke("get_settings");
      watchedFolder = s.watched_folder ?? null;
      await refreshClips();
    } catch (e) {
      console.error(e);
    }
  }
  async function refreshClips() {
    if (!watchedFolder) return;
    try {
      recentClips = await invoke("list_recent_clips", { folder: watchedFolder });
    } catch {
      recentClips = [];
    }
  }
  async function chooseFolder() {
    const picked = await open({ directory: true, multiple: false, title: "Choose your clips folder" });
    if (typeof picked === "string") {
      watchedFolder = picked;
      gameFilter = "all";
      await invoke("set_settings", { settings: { watched_folder: picked } });
      await refreshClips();
    }
  }
  async function openFileDialog() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "avi", "webm", "m4v"] }],
    });
    if (typeof picked === "string") await loadClip(picked);
  }

  // --- load a clip ------------------------------------------------------
  async function loadClip(path) {
    const gen = ++loadGen;
    busy = true;
    busyLabel = "Loading";
    toast = null;
    try {
      const info = await invoke("probe_clip", { path });
      if (gen !== loadGen) return; // superseded by a newer load
      const name = path.split(/[\\/]/).pop();
      clip = { path, name, ...info };
      videoSrc = convertFileSrc(path);
      duration = info.duration || 0;
      inPoint = 0;
      outPoint = duration;
      currentTime = 0;
      outputName = "";
      mode = "lossless";
    } catch (e) {
      if (gen !== loadGen) return; // a newer load is in charge; swallow this error
      toast = { kind: "err", msg: String(e) };
    } finally {
      if (gen === loadGen) {
        busy = false;
        busyLabel = "";
      }
    }
  }
  function closeClip() {
    loadGen++; // cancel any in-flight loadClip
    if (videoEl) videoEl.pause();
    clip = null;
    videoSrc = null;
    playing = false;
    refreshClips();
  }

  function onMeta() {
    if (videoEl && isFinite(videoEl.duration)) {
      duration = videoEl.duration;
      if (outPoint === 0 || outPoint > duration) outPoint = duration;
    }
  }
  function onTimeUpdate() {
    // No-op: watchPlayback() owns currentTime while playing;
    // onTrackDown / moveHandle update it directly for paused scrubs.
  }
  function watchPlayback() {
    if (!videoEl) return;
    currentTime = videoEl.currentTime;
    if (currentTime >= outPoint) {
      videoEl.pause();
      videoEl.currentTime = outPoint;
      playing = false;
      stopWatch();
      return;
    }
    playRaf = requestAnimationFrame(watchPlayback);
  }
  function startWatch() {
    stopWatch();
    playRaf = requestAnimationFrame(watchPlayback);
  }
  function stopWatch() {
    if (playRaf !== 0) {
      cancelAnimationFrame(playRaf);
      playRaf = 0;
    }
  }

  // --- transport --------------------------------------------------------
  function togglePlay() {
    if (!videoEl) return;
    if (videoEl.paused) {
      if (currentTime < inPoint || currentTime >= outPoint) videoEl.currentTime = inPoint;
      videoEl.play();
      playing = true;
    } else {
      videoEl.pause();
      playing = false;
    }
  }
  function playSelection() {
    if (!videoEl) return;
    videoEl.currentTime = inPoint;
    videoEl.play();
    playing = true;
  }

  // --- timeline interaction --------------------------------------------
  function timeFromX(clientX) {
    const r = timelineEl.getBoundingClientRect();
    let f = (clientX - r.left) / r.width;
    f = Math.max(0, Math.min(1, f));
    return f * duration;
  }
  function onTrackDown(e) {
    if (activeHandle) return;
    const t = timeFromX(e.clientX);
    if (videoEl) videoEl.currentTime = t;
    currentTime = t;
  }
  function startHandle(which, e) {
    e.stopPropagation();
    activeHandle = which;
    e.currentTarget.setPointerCapture(e.pointerId);
  }
  function moveHandle(e) {
    if (!activeHandle) return;
    const t = timeFromX(e.clientX);
    const guard = Math.min(0.05, duration / 1000);
    if (activeHandle === "in") inPoint = Math.min(t, outPoint - guard);
    else outPoint = Math.max(t, inPoint + guard);
    if (videoEl) {
      videoEl.currentTime = activeHandle === "in" ? inPoint : outPoint;
      currentTime = videoEl.currentTime;
    }
  }
  function endHandle(e) {
    if (!activeHandle) return;
    try { e.currentTarget.releasePointerCapture(e.pointerId); } catch {}
    activeHandle = null;
  }
  function setInHere() { inPoint = Math.min(currentTime, outPoint - 0.05); }
  function setOutHere() { outPoint = Math.max(currentTime, inPoint + 0.05); }

  function setMode(m) {
    mode = m;
  }

  // --- export -----------------------------------------------------------
  async function exportClip() {
    if (!clip || selLength <= 0) return;
    busy = true;
    busyLabel = mode === "compress" ? "Compressing" : "Trimming";
    toast = null;
    try {
      const name = outputName.trim() || null;
      let res;
      if (mode === "compress") {
        res = await invoke("compress_clip", {
          path: clip.path,
          start: inPoint,
          end: outPoint,
          outputName: name,
          mode: compressBy,
          targetMb: compressBy === "size" ? Number(targetMb) : null,
          quality: compressBy === "quality" ? quality : null,
        });
      } else {
        res = await invoke("trim_clip", { path: clip.path, start: inPoint, end: outPoint, outputName: name });
      }
      toast = { kind: "ok", ...res };
    } catch (e) {
      toast = { kind: "err", msg: String(e) };
    } finally {
      busy = false;
      busyLabel = "";
    }
  }
  async function revealOutput() {
    if (toast?.path) {
      try { await revealItemInDir(toast.path); } catch (e) { console.error(e); }
    }
  }

  // --- keyboard ---------------------------------------------------------
  function onKey(e) {
    if (!clip || e.target.tagName === "INPUT") return;
    if (e.code === "Space") { e.preventDefault(); togglePlay(); }
    else if (e.key === "i" || e.key === "I") setInHere();
    else if (e.key === "o" || e.key === "O") setOutHere();
  }

  onMount(() => {
    loadSettings();
    try {
      const v = parseFloat(localStorage.getItem("klipt:volume"));
      if (isFinite(v)) volume = Math.max(0, Math.min(1, v));
    } catch {}
    let un;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "over" || p.type === "enter") dragOver = true;
        else if (p.type === "leave") dragOver = false;
        else if (p.type === "drop") {
          dragOver = false;
          const f = (p.paths || []).find((x) => /\.(mp4|mov|mkv|avi|webm|m4v)$/i.test(x));
          if (f) loadClip(f);
        }
      })
      .then((f) => (un = f));
    return () => un && un();
  });
</script>

<svelte:window on:keydown={onKey} />

<div class="app">
  <!-- ============ TITLEBAR ============ -->
  <div class="titlebar" data-tauri-drag-region>
    <div class="tb-brand" data-tauri-drag-region>
      <span class="mark">K</span><span class="word">klipt</span>
    </div>
    <div class="tb-drag" data-tauri-drag-region></div>
    <div class="tb-controls">
      <button class="tb-btn" onclick={() => appWindow.minimize()} aria-label="Minimize">
        <svg width="11" height="11" viewBox="0 0 11 11"><rect x="1" y="5" width="9" height="1" fill="currentColor"/></svg>
      </button>
      <button class="tb-btn" onclick={() => appWindow.toggleMaximize()} aria-label="Maximize">
        <svg width="11" height="11" viewBox="0 0 11 11"><rect x="1.5" y="1.5" width="8" height="8" fill="none" stroke="currentColor" stroke-width="1"/></svg>
      </button>
      <button class="tb-btn danger" onclick={() => appWindow.close()} aria-label="Close">
        <svg width="11" height="11" viewBox="0 0 11 11"><path d="M1 1 L10 10 M10 1 L1 10" stroke="currentColor" stroke-width="1.2"/></svg>
      </button>
    </div>
  </div>

  <div class="body">
    {#if !clip}
      <!-- ============ LANDING ============ -->
      <section class="landing">
        <div class="ltop">
          {#if watchedFolder}
            <button class="srcbtn" onclick={chooseFolder} title="Change clips folder">
              <svg width="15" height="15" viewBox="0 0 16 16" fill="none"><path d="M1.5 4.5 A1 1 0 0 1 2.5 3.5 H6 L7.5 5 H13.5 A1 1 0 0 1 14.5 6 V12 A1 1 0 0 1 13.5 13 H2.5 A1 1 0 0 1 1.5 12 Z" stroke="currentColor" stroke-width="1.1"/></svg>
              <span class="srcpath mono">{watchedFolder}</span>
            </button>
            <button class="iconlink" onclick={refreshClips} aria-label="Refresh" title="Refresh">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M13 8 A5 5 0 1 1 11.4 4.4 M11.4 4.4 H8.7 M11.4 4.4 V1.7" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </button>
          {:else}
            <button class="srcbtn empty" onclick={chooseFolder}>
              <svg width="15" height="15" viewBox="0 0 16 16" fill="none"><path d="M1.5 4.5 A1 1 0 0 1 2.5 3.5 H6 L7.5 5 H13.5 A1 1 0 0 1 14.5 6 V12 A1 1 0 0 1 13.5 13 H2.5 A1 1 0 0 1 1.5 12 Z" stroke="currentColor" stroke-width="1.1"/></svg>
              Choose clips folder
            </button>
          {/if}
          <div class="lspacer"></div>
          <button class="btn ghost sm" onclick={openFileDialog}>Open file</button>
        </div>

        {#if recentClips.length > 0}
          <div class="filters">
            <div class="fgroup">
              <Dropdown bind:value={gameFilter} options={gameOptions} label="Game" ariaLabel="Filter by game" />
              <Dropdown bind:value={dateFilter} options={DATE_OPTIONS} label="Date" ariaLabel="Filter by date" />
            </div>
            <span class="fcount mono">{filteredClips.length} of {recentClips.length}</span>
          </div>
        {/if}

        {#if busy && busyLabel === "Loading"}
          <div class="grid">
            {#each Array(8) as _, i}<div class="card skeleton" style="--i:{i}"></div>{/each}
          </div>
        {:else if filteredClips.length === 0}
          <div class="empty">
            <div class="empty-art">
              <svg width="40" height="40" viewBox="0 0 24 24" fill="none"><rect x="2" y="5" width="20" height="14" rx="2" stroke="currentColor" stroke-width="1.2"/><path d="M10 9.5 L15 12 L10 14.5 Z" fill="currentColor"/></svg>
            </div>
            <p>{recentClips.length === 0 ? "Nothing here yet" : "No clips match"}</p>
            <p class="muted">
              {recentClips.length === 0
                ? "Drop a video anywhere in this window, or point Klipt at your ShadowPlay folder."
                : "Try a different game or date filter."}
            </p>
          </div>
        {:else}
          <div class="grid">
            {#each filteredClips as c, i (c.path)}
              <button class="card" style="--i:{i}" use:thumbOnVisible={c.path} onclick={() => loadClip(c.path)} title={c.path}>
                <div class="thumb" class:loaded={thumbs[c.path]}>
                  {#if thumbs[c.path]}
                    <img src={thumbs[c.path]} alt="" loading="lazy" draggable="false" />
                  {:else}
                    <svg width="22" height="22" viewBox="0 0 24 24" fill="none"><path d="M8 6 L18 12 L8 18 Z" fill="currentColor"/></svg>
                  {/if}
                  <span class="playbadge">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none"><path d="M8 6 L18 12 L8 18 Z" fill="currentColor"/></svg>
                  </span>
                </div>
                <div class="cardbody">
                  <div class="cardname">{c.name}</div>
                  <div class="cardmeta"><span class="gtag">{c.game || "Other"}</span><span class="mono">{fmtSize(c.size_bytes)}</span></div>
                </div>
              </button>
            {/each}
          </div>
        {/if}
      </section>
    {:else}
      <!-- ============ EDITOR ============ -->
      <section class="editor">
        <div class="stage">
          <!-- svelte-ignore a11y_media_has_caption -->
          <video
            bind:this={videoEl}
            src={videoSrc}
            onloadedmetadata={onMeta}
            ontimeupdate={onTimeUpdate}
            onplay={() => { playing = true; startWatch(); }}
            onpause={() => { playing = false; stopWatch(); }}
            onclick={togglePlay}
          ></video>
        </div>

        <!-- top overlay -->
        <header class="ehead">
          <button class="btn ghost sm glass" onclick={closeClip}>
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M8.5 3 L4.5 7 L8.5 11" stroke="currentColor" stroke-width="1.3"/></svg>
            Back
          </button>
          <div class="ename">{clip.name}</div>
          <div class="emeta mono">{clip.width}×{clip.height} · {fmtSize(clip.size_bytes)}</div>
        </header>

        <!-- bottom overlay dock -->
        <div class="dock">
          <div
            class="timeline"
            bind:this={timelineEl}
            onpointerdown={onTrackDown}
            role="slider" tabindex="0" aria-label="Trim timeline" aria-valuenow={currentTime}
          >
            <div class="track"></div>
            <div class="region" style="left:{pct(inPoint)}%; width:{pct(selLength)}%"></div>
            <div class="playhead" style="left:{pct(currentTime)}%"></div>
            <div class="handle in" class:active={activeHandle === "in"} style="left:{pct(inPoint)}%"
              onpointerdown={(e) => startHandle("in", e)} onpointermove={moveHandle} onpointerup={endHandle}
              role="slider" tabindex="0" aria-label="In point" aria-valuenow={inPoint}></div>
            <div class="handle out" class:active={activeHandle === "out"} style="left:{pct(outPoint)}%"
              onpointerdown={(e) => startHandle("out", e)} onpointermove={moveHandle} onpointerup={endHandle}
              role="slider" tabindex="0" aria-label="Out point" aria-valuenow={outPoint}></div>
          </div>

          <!-- options bar: output mode, inline compress controls, output name -->
          <div class="optbar">
            <div class="obleft">
              <div class="seg">
                <button class="seg-btn" class:on={mode === "lossless"} onclick={() => setMode("lossless")}>Lossless</button>
                <button class="seg-btn" class:on={mode === "compress"} onclick={() => setMode("compress")}>Compress</button>
              </div>

              {#if mode === "compress"}
                <div class="seg sub">
                  <button class="seg-btn" class:on={compressBy === "size"} onclick={() => (compressBy = "size")}>Size</button>
                  <button class="seg-btn" class:on={compressBy === "quality"} onclick={() => (compressBy = "quality")}>Quality</button>
                </div>
                {#if compressBy === "size"}
                  <div class="pills">
                    {#each SIZE_PRESETS as mb}
                      <button class="pill" class:on={Number(targetMb) === mb} onclick={() => (targetMb = mb)}>{mb} MB</button>
                    {/each}
                    <label class="pill custom" class:on={!SIZE_PRESETS.includes(Number(targetMb))}>
                      <input class="mono" type="number" min="1" max="500" bind:value={targetMb} aria-label="Custom size in MB" /><span>MB</span>
                    </label>
                  </div>
                {:else}
                  <div class="pills">
                    {#each [["low", "Low"], ["medium", "Medium"], ["high", "High"]] as [v, label]}
                      <button class="pill" class:on={quality === v} onclick={() => (quality = v)}>{label}</button>
                    {/each}
                  </div>
                {/if}
              {:else}
                <span class="obhint">Instant · no quality loss · cuts snap to the nearest keyframe</span>
              {/if}
            </div>

            <label class="obname">
              <span class="oblabel">Save as</span>
              <div class="nameinput">
                <input bind:value={outputName} placeholder={defaultStem} spellcheck="false" />
                <span class="ext mono">.{outExt}</span>
              </div>
            </label>
          </div>

          <div class="dockrow">
            <div class="left">
              <button class="round" onclick={togglePlay} aria-label="Play/pause">
                {#if playing}
                  <svg width="13" height="13" viewBox="0 0 13 13"><rect x="2" y="1.5" width="3" height="10" fill="currentColor"/><rect x="8" y="1.5" width="3" height="10" fill="currentColor"/></svg>
                {:else}
                  <svg width="13" height="13" viewBox="0 0 13 13"><path d="M3 1.5 L11 6.5 L3 11.5 Z" fill="currentColor"/></svg>
                {/if}
              </button>
              <button class="btn ghost sm glass" onclick={playSelection}>Selection</button>

              <div class="vol">
                <button class="volbtn" onclick={toggleMute} aria-label={muted || volume === 0 ? "Unmute" : "Mute"}>
                  {#if muted || volume === 0}
                    <svg width="16" height="16" viewBox="0 0 16 16" fill="none"><path d="M2.5 6 H4.5 L8 3 V13 L4.5 10 H2.5 Z" fill="currentColor"/><path d="M11 6 L14 10 M14 6 L11 10" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>
                  {:else if volume < 0.5}
                    <svg width="16" height="16" viewBox="0 0 16 16" fill="none"><path d="M2.5 6 H4.5 L8 3 V13 L4.5 10 H2.5 Z" fill="currentColor"/><path d="M10.5 6.2 A2.4 2.4 0 0 1 10.5 9.8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>
                  {:else}
                    <svg width="16" height="16" viewBox="0 0 16 16" fill="none"><path d="M2.5 6 H4.5 L8 3 V13 L4.5 10 H2.5 Z" fill="currentColor"/><path d="M10.5 5.5 A3.2 3.2 0 0 1 10.5 10.5 M12.4 4 A5.6 5.6 0 0 1 12.4 12" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>
                  {/if}
                </button>
                <input
                  class="volslider"
                  type="range" min="0" max="1" step="0.02"
                  bind:value={volume} oninput={onVolInput}
                  style="--vfill:{(muted ? 0 : volume) * 100}%"
                  aria-label="Preview volume"
                />
              </div>

              <div class="readout mono">
                <span class="time">{fmt(currentTime)}<span class="muted"> / {fmt(duration)}</span></span>
                <span class="sep">·</span>
                <span class="lbl">IN</span> {fmt(inPoint)}
                <span class="lbl">OUT</span> {fmt(outPoint)}
                <span class="lbl">LEN</span> <span class="strong">{fmt(selLength)}</span>
              </div>
            </div>

            <div class="right">
              <button class="btn primary export" onclick={exportClip} disabled={busy || selLength <= 0}>
                {#if busy}<span class="spin"></span>{busyLabel}…{:else}{mode === "compress" ? "Compress" : "Trim"}<span class="blen mono">{fmt(selLength)}</span>{/if}
              </button>
            </div>
          </div>
        </div>
      </section>
    {/if}
  </div>

  {#if dragOver}
    <div class="dropmask"><div class="dropcard">Drop to load</div></div>
  {/if}

  {#if toast}
    <div class="toast {toast.kind}">
      {#if toast.kind === "ok"}
        <span class="tdot ok"></span>
        <span>Saved <strong>{toast.path?.split(/[\\/]/).pop()}</strong>
          <span class="mono muted"> · {fmtSize(toast.size_bytes)}{toast.encoder ? ` · ${toast.encoder}` : ""}</span></span>
        <button class="link" onclick={revealOutput}>Show in folder</button>
      {:else}
        <span class="tdot err"></span><span class="errmsg">{toast.msg}</span>
      {/if}
      <button class="link" onclick={() => (toast = null)}>Dismiss</button>
    </div>
  {/if}
</div>

<style>
  :global(:root) {
    --bg: #0a0a0b;
    --panel: #121214;
    --panel-2: #17171a;
    --panel-3: #1d1d21;
    --border: #26262b;
    --border-2: #34343a;
    --text: #f4f4f5;
    --muted: #86868d;
    --faint: #5b5b62;
    --accent: #fafafa;
    --ui: "Geist", system-ui, sans-serif;
    --display: "Space Grotesk", "Geist", system-ui, sans-serif;
    --mono: "Geist Mono", ui-monospace, monospace;
    color-scheme: dark;
  }
  :global(body) { margin: 0; font-family: var(--ui); -webkit-font-smoothing: antialiased; }
  :global(*) { box-sizing: border-box; }

  .app { height: 100vh; display: flex; flex-direction: column; background: var(--bg); color: var(--text); overflow: hidden; user-select: none; }

  /* ---------- titlebar ---------- */
  .titlebar { height: 38px; flex: 0 0 38px; display: flex; align-items: center; padding-left: 14px; background: var(--bg); border-bottom: 1px solid var(--border); }
  .tb-brand { display: flex; align-items: center; gap: 8px; }
  .mark { width: 19px; height: 19px; display: grid; place-items: center; background: linear-gradient(180deg, #161618, #0a0a0c); color: var(--accent); border: 1px solid rgba(255,255,255,0.09); border-radius: 5px; font-weight: 800; font-size: 12px; font-family: var(--display); }
  .word { font-family: var(--display); font-weight: 600; letter-spacing: 0.01em; font-size: 13.5px; }
  .tb-drag { flex: 1; height: 100%; }
  .tb-controls { display: flex; height: 100%; }
  .tb-btn { width: 44px; height: 38px; display: grid; place-items: center; background: transparent; border: 0; color: var(--muted); cursor: pointer; transition: background 0.12s, color 0.12s; }
  .tb-btn:hover { background: var(--panel-2); color: var(--text); }
  .tb-btn.danger:hover { background: #b4232a; color: #fff; }

  .body { flex: 1; min-height: 0; display: flex; flex-direction: column; }

  /* ---------- buttons ---------- */
  .btn { font: inherit; cursor: pointer; display: inline-flex; align-items: center; gap: 7px; border-radius: 9px; border: 1px solid var(--border-2); background: var(--panel-2); color: var(--text); padding: 8px 14px; font-size: 13px; transition: background 0.15s, border-color 0.15s, transform 0.05s; }
  .btn:hover { background: var(--panel-3); border-color: #41414a; }
  .btn:active { transform: translateY(1px); }
  .btn.ghost { background: transparent; }
  .btn.ghost:hover { background: var(--panel-2); }
  .btn.sm { padding: 6px 11px; font-size: 12.5px; }
  .btn.glass { background: rgba(20,20,22,0.55); backdrop-filter: blur(10px); border-color: rgba(255,255,255,0.12); }
  .btn.glass:hover, .btn.glass.on { background: rgba(40,40,44,0.7); }
  .btn.primary { background: var(--accent); color: #0a0a0b; border-color: var(--accent); font-weight: 600; padding: 11px 20px; font-size: 14px; box-shadow: 0 8px 24px -10px rgba(255,255,255,0.4); }
  .btn.primary:hover { background: #fff; }
  .btn.primary:disabled { opacity: 0.4; cursor: default; box-shadow: none; }
  .link { background: none; border: 0; color: var(--muted); cursor: pointer; font: inherit; font-size: 12.5px; text-decoration: underline; text-underline-offset: 2px; padding: 2px 4px; }
  .link:hover { color: var(--text); }
  .muted { color: var(--muted); }
  .mono { font-family: var(--mono); font-feature-settings: "tnum"; }

  /* ---------- landing ---------- */
  .landing { flex: 1; overflow-y: auto; padding: 16px 28px 30px; width: 100%; }
  .ltop { display: flex; align-items: center; gap: 8px; margin-bottom: 14px; }
  .lspacer { flex: 1; }
  .srcbtn { display: inline-flex; align-items: center; gap: 8px; min-width: 0; max-width: 62%; font: inherit; font-size: 12.5px; color: var(--text); background: var(--panel); border: 1px solid var(--border); border-radius: 8px; padding: 6px 11px; cursor: pointer; transition: background 0.14s, border-color 0.14s; }
  .srcbtn:hover { background: var(--panel-2); border-color: var(--border-2); }
  .srcbtn svg { flex: 0 0 auto; color: var(--faint); }
  .srcbtn:hover svg { color: var(--muted); }
  .srcpath { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11.5px; color: var(--muted); }
  .srcbtn:hover .srcpath { color: var(--text); }
  .iconlink { display: grid; place-items: center; width: 30px; height: 30px; flex: 0 0 auto; border: 0; border-radius: 8px; background: transparent; color: var(--muted); cursor: pointer; transition: background 0.14s, color 0.14s; }
  .iconlink:hover { background: var(--panel-2); color: var(--text); }

  /* ---------- filters ---------- */
  .filters { display: flex; flex-wrap: wrap; gap: 14px; justify-content: space-between; align-items: center; padding-bottom: 14px; margin-bottom: 18px; border-bottom: 1px solid var(--border); }
  .fgroup { display: flex; flex-wrap: wrap; gap: 9px; }
  .fcount { font-size: 11.5px; color: var(--faint); flex: 0 0 auto; }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 15px; }
  .card { background: var(--panel); border: 1px solid var(--border); border-radius: 14px; padding: 0; overflow: hidden; text-align: left; color: var(--text); cursor: pointer; transition: transform 0.18s cubic-bezier(0.16,1,0.3,1), border-color 0.18s, background 0.18s; box-shadow: inset 0 1px 0 rgba(255,255,255,0.02); animation: rise 0.4s cubic-bezier(0.16,1,0.3,1) backwards; animation-delay: calc(var(--i) * 26ms); }
  .card:hover { transform: translateY(-3px); border-color: var(--border-2); background: var(--panel-2); }
  .card:active { transform: translateY(-1px) scale(0.99); }
  .thumb { position: relative; height: 104px; display: grid; place-items: center; color: var(--faint); background: linear-gradient(150deg, #161619, #1e1e23); border-bottom: 1px solid var(--border); overflow: hidden; }
  .card:hover .thumb { color: var(--muted); }
  .thumb img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; display: block; animation: fade 0.3s ease; }
  .thumb.loaded { background: #0d0d0f; }
  .playbadge { position: absolute; right: 8px; bottom: 8px; width: 26px; height: 26px; display: grid; place-items: center; border-radius: 50%; color: #fff; background: rgba(10,10,11,0.55); backdrop-filter: blur(6px); border: 1px solid rgba(255,255,255,0.18); opacity: 0; transform: scale(0.85); transition: opacity 0.18s, transform 0.18s; }
  .thumb.loaded .playbadge { opacity: 0; }
  .card:hover .playbadge { opacity: 1; transform: scale(1); }
  .cardbody { padding: 10px 12px 11px; }
  .cardname { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-bottom: 7px; }
  .cardmeta { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 11px; color: var(--muted); }
  .gtag { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 60%; color: var(--faint); }
  .skeleton { height: 160px; background: var(--panel); animation: pulse 1.3s ease-in-out infinite; animation-delay: calc(var(--i) * 60ms); }

  .empty { margin: 12vh auto 0; max-width: 360px; text-align: center; }
  .empty-art { color: var(--faint); margin-bottom: 14px; display: grid; place-items: center; }
  .empty p { margin: 4px 0; }
  .empty p:first-of-type { font-size: 15px; font-weight: 600; }
  .empty .muted { font-size: 13px; line-height: 1.6; }

  /* ---------- editor (overlay) ---------- */
  .editor { position: relative; flex: 1; min-height: 0; background: #060607; overflow: hidden; }
  .stage { position: absolute; inset: 0; display: grid; place-items: center; padding: 8px; }
  video { max-width: 100%; max-height: 100%; border-radius: 8px; background: #000; }

  .ehead { position: absolute; top: 0; left: 0; right: 0; display: flex; align-items: center; gap: 14px; padding: 12px 16px 30px; background: linear-gradient(to bottom, rgba(0,0,0,0.6), transparent); pointer-events: none; }
  .ehead > * { pointer-events: auto; }
  .ename { font-weight: 600; font-size: 13.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-shadow: 0 1px 3px rgba(0,0,0,0.6); }
  .emeta { font-size: 12px; color: var(--muted); margin-left: auto; flex: 0 0 auto; text-shadow: 0 1px 3px rgba(0,0,0,0.6); }

  .dock { position: absolute; left: 0; right: 0; bottom: 0; padding: 46px 18px 16px; background: linear-gradient(to top, rgba(8,8,9,0.92) 55%, rgba(8,8,9,0.5) 80%, transparent); }
  .timeline { position: relative; height: 40px; margin-bottom: 8px; cursor: pointer; touch-action: none; }
  .track { position: absolute; top: 50%; left: 0; right: 0; height: 8px; transform: translateY(-50%); background: rgba(255,255,255,0.1); border: 1px solid rgba(255,255,255,0.12); border-radius: 5px; }
  .region { position: absolute; top: 50%; height: 16px; transform: translateY(-50%); background: rgba(255,255,255,0.2); border-top: 1px solid rgba(255,255,255,0.6); border-bottom: 1px solid rgba(255,255,255,0.6); }
  .playhead { position: absolute; top: 2px; bottom: 2px; width: 2px; background: var(--text); transform: translateX(-1px); pointer-events: none; border-radius: 2px; box-shadow: 0 0 6px rgba(0,0,0,0.6); }
  .handle { position: absolute; top: 50%; width: 12px; height: 30px; transform: translate(-50%, -50%); background: var(--accent); border-radius: 5px; cursor: ew-resize; box-shadow: 0 0 0 1px #000, 0 4px 12px -4px rgba(0,0,0,0.8); touch-action: none; transition: box-shadow 0.15s; }
  .handle::after { content: ""; position: absolute; left: 50%; top: 50%; width: 2px; height: 12px; background: #0a0a0b40; transform: translate(-50%,-50%); border-radius: 2px; }
  .handle:hover, .handle.active { box-shadow: 0 0 0 1px #000, 0 0 0 4px rgba(255,255,255,0.18); }

  .dockrow { display: flex; align-items: center; justify-content: space-between; gap: 14px; flex-wrap: wrap; }
  .left { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .right { display: flex; align-items: center; gap: 9px; }
  .round { width: 36px; height: 36px; border-radius: 50%; border: 0; background: var(--accent); color: #0a0a0b; cursor: pointer; display: grid; place-items: center; transition: transform 0.05s, background 0.15s; flex: 0 0 auto; }
  .round:hover { background: #fff; }
  .round:active { transform: scale(0.94); }
  .readout { display: flex; align-items: baseline; gap: 7px; font-size: 12px; white-space: nowrap; overflow: hidden; }
  .readout .time { font-size: 12.5px; }
  .readout .sep { color: var(--faint); }
  .readout .lbl { font-size: 9.5px; letter-spacing: 0.1em; color: var(--faint); }
  .readout .strong { color: var(--text); font-weight: 600; }

  /* segmented + pills */
  .seg { display: inline-flex; padding: 3px; background: rgba(10,10,11,0.6); backdrop-filter: blur(10px); border: 1px solid var(--border); border-radius: 10px; gap: 3px; }
  .seg.sub { background: var(--panel); backdrop-filter: none; }
  .seg-btn { border: 0; background: transparent; color: var(--muted); cursor: pointer; font: inherit; font-size: 12.5px; padding: 6px 14px; border-radius: 7px; transition: background 0.15s, color 0.15s; }
  .seg-btn:hover { color: var(--text); }
  .seg-btn.on { background: var(--panel-3); color: var(--text); box-shadow: inset 0 1px 0 rgba(255,255,255,0.05); }

  /* options bar (between timeline and transport) */
  .optbar { display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; margin-bottom: 12px; }
  .obleft { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; min-width: 0; }
  .obhint { font-size: 12px; color: var(--muted); }
  .obname { display: flex; align-items: center; gap: 9px; flex: 0 1 auto; min-width: 0; }
  .oblabel { font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); flex: 0 0 auto; }
  .pills { display: flex; gap: 7px; flex-wrap: wrap; }
  .pill { border: 1px solid var(--border-2); background: var(--panel); color: var(--muted); cursor: pointer; font: inherit; font-size: 12.5px; padding: 6px 12px; border-radius: 9px; transition: background 0.15s, color 0.15s, border-color 0.15s; }
  .pill:hover { color: var(--text); border-color: #41414a; }
  .pill.on { background: var(--accent); color: #0a0a0b; border-color: var(--accent); font-weight: 600; }
  .pill.custom { display: inline-flex; align-items: center; gap: 5px; padding: 3px 10px 3px 4px; }
  .pill.custom input { width: 50px; background: var(--bg); border: 1px solid var(--border); color: var(--text); border-radius: 6px; padding: 5px 7px; font-size: 12.5px; outline: none; text-align: right; }
  .pill.custom.on { background: var(--panel); border-color: var(--accent); color: var(--text); }

  .nameinput { display: flex; align-items: center; width: 230px; max-width: 100%; background: rgba(10,10,11,0.55); backdrop-filter: blur(10px); border: 1px solid var(--border-2); border-radius: 9px; padding-right: 10px; overflow: hidden; }
  .nameinput:focus-within { border-color: #4a4a52; }
  .nameinput input { flex: 1; min-width: 0; background: transparent; border: 0; color: var(--text); font: inherit; font-size: 13px; padding: 8px 11px; outline: none; }
  .ext { color: var(--faint); font-size: 12.5px; }

  /* volume (preview only) */
  .vol { display: flex; align-items: center; gap: 6px; flex: 0 0 auto; }
  .volbtn { width: 30px; height: 30px; display: grid; place-items: center; border: 0; border-radius: 8px; background: transparent; color: var(--muted); cursor: pointer; transition: background 0.14s, color 0.14s; }
  .volbtn:hover { background: rgba(255,255,255,0.08); color: var(--text); }
  .volslider { -webkit-appearance: none; appearance: none; width: 74px; height: 4px; border-radius: 3px; cursor: pointer; background: linear-gradient(to right, var(--text) var(--vfill), rgba(255,255,255,0.16) var(--vfill)); outline: none; }
  .volslider::-webkit-slider-thumb { -webkit-appearance: none; appearance: none; width: 12px; height: 12px; border-radius: 50%; background: var(--text); border: 0; box-shadow: 0 1px 4px rgba(0,0,0,0.6); cursor: pointer; }
  .volslider:focus-visible { box-shadow: 0 0 0 3px rgba(255,255,255,0.18); }

  .export { flex: 0 0 auto; position: relative; z-index: 21; }
  .blen { font-size: 12px; opacity: 0.55; }
  .spin { width: 13px; height: 13px; border-radius: 50%; border: 2px solid rgba(10,10,11,0.25); border-top-color: #0a0a0b; animation: spin 0.7s linear infinite; display: inline-block; }

  /* ---------- overlays ---------- */
  .dropmask { position: fixed; inset: 0; display: grid; place-items: center; background: rgba(5,5,6,0.7); backdrop-filter: blur(2px); z-index: 50; }
  .dropcard { padding: 30px 56px; border: 1.5px dashed rgba(255,255,255,0.35); border-radius: 16px; font-size: 18px; font-weight: 600; background: rgba(20,20,22,0.6); }
  .toast { position: fixed; bottom: 20px; left: 50%; display: flex; align-items: center; gap: 13px; padding: 11px 15px; border-radius: 11px; border: 1px solid var(--border-2); background: var(--panel-3); font-size: 13px; z-index: 60; box-shadow: 0 18px 50px -16px rgba(0,0,0,0.8); max-width: 70vw; animation: toastin 0.25s cubic-bezier(0.16,1,0.3,1); transform: translateX(-50%); }
  .tdot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
  .tdot.ok { background: #4ade80; }
  .tdot.err { background: #f87171; }
  .errmsg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 46vw; }

  @keyframes rise { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: translateY(0); } }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
  @keyframes pulse { 0%, 100% { opacity: 0.5; } 50% { opacity: 0.85; } }
  @keyframes spin { to { transform: rotate(360deg); } }
  @keyframes toastin { from { opacity: 0; transform: translateX(-50%) translateY(8px); } to { opacity: 1; transform: translateX(-50%) translateY(0); } }
</style>

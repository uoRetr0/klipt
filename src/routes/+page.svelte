<script>
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import Dropdown from "$lib/Dropdown.svelte";
  import Titlebar from "$lib/Titlebar.svelte";
  import { resolve as resolveKey } from "$lib/keymap.js";
  import { slideRegion } from "$lib/region.js";
  import { frameOf, timeOf } from "$lib/frames.js";
  import { loopDecision } from "$lib/loop.js";
  import { hoverTime, frameIndexAt } from "$lib/filmstrip.js";
  import { gridColumns, rowWindow } from "$lib/grid.js";
  import { fmt, fmtSize, waveformPath, previewName, baseName } from "$lib/format.js";
  import { matchesDateFilter } from "$lib/datefilter.js";
  import { sortClips } from "$lib/sort.js";
  import { VIDEO_EXTS, isVideoFile } from "$lib/video.js";
  import { screenToSource, normalizeCrop, cropToPercent, moveCrop, resizeCrop, hitTestCrop } from "$lib/crop.js";
  import {
    trashedToast,
    deletedToast,
    restoringToast,
    restoredToast,
    restoreFailedToast,
    undoAvailable,
  } from "$lib/toast.js";

  /**
   * @typedef {{ x: number, idx: number, time: number }} HoverFrame
   * @typedef {{ path: string, idx: number }} CardHover
   * @typedef {{ x: number, y: number, clip: import('$lib/types').ClipEntry }} CardMenu
   * @typedef {{ path: string, name: string }} Renaming
   * @typedef {{ startX: number, startIn: number, startOut: number, moved: boolean, scrub: boolean }} RegionDrag
   * @typedef {Record<string, any>} Toast  Union of toast-builder outputs and inline {kind,...} objects.
   */

  // --- state ------------------------------------------------------------
  let watchedFolder = /** @type {string | null} */ ($state(null));
  let recentClips = /** @type {import('$lib/types').ClipEntry[]} */ ($state([]));
  let gameFilter = $state("all");
  let dateFilter = $state("all");
  // Library ordering. Default date+desc reproduces the backend's newest-first
  // scan; persisted (view pref) via localStorage like the timeline toggles.
  let sortKey = $state("date"); // 'date' | 'name' | 'size'
  let sortDir = $state("desc"); // 'asc' | 'desc'
  // Free-text filter over clip name + game. Client-side over the full list, so a
  // big library searches instantly without re-scanning the disk.
  let query = $state("");

  // --- windowed (virtualized) library grid ------------------------------
  // The grid renders only the cards in/near the viewport, so the DOM node count
  // stays constant no matter how large the library is. These measure the live
  // layout; the pure math lives in grid.js.
  const GRID_MIN = 190; // matches the CSS minmax() floor
  const GRID_GAP = 15; // matches the CSS grid gap
  /** @type {HTMLElement|null} */
  let landingEl = /** @type {HTMLElement | null} */ ($state(null)); // the scroll container
  /** @type {HTMLElement|null} */
  let gridWrapEl = /** @type {HTMLElement | null} */ ($state(null)); // full-height spacer that owns the scrollbar
  let scrollTop = $state(0);
  let viewportH = $state(0); // landing's visible height
  let gridW = $state(0); // grid content-box width (drives the column count)
  let gridTop = $state(0); // grid's offset within the scroll content
  let cardH = $state(168); // measured card height (estimate until first measure)
  let scrollRaf = 0;

  let clip = /** @type {(import('$lib/types').ClipInfo & { path: string, name: string }) | null} */ ($state(null));
  let videoSrc = /** @type {string | null} */ ($state(null));
  let videoEl = /** @type {HTMLVideoElement | null} */ ($state(null));
  let timelineEl = /** @type {HTMLElement | null} */ ($state(null));
  let timelineWidth = $state(0); // measured px width, for a GPU-composited playhead

  let duration = $state(0);
  let currentTime = $state(0);
  let inPoint = $state(0);
  let outPoint = $state(0);
  let playing = $state(false);
  // Playback scope. When on (default), Play is confined to the Region (in →
  // out) so the trimmed moment previews in isolation; when off, Play ignores the
  // trim and runs across the whole Clip from wherever the playhead is. Persisted.
  let selectionOnly = $state(true);
  // When on, playback wraps from the end of the active scope back to its start
  // so the moment can be watched on repeat while fine-tuning. Session-only.
  let loopEnabled = $state(false);
  // Auto-hide chrome (header + dock) while playing and idle, like a video
  // player. Revealed by pointer movement; kept up while paused or interacting.
  let uiVisible = $state(true);
  let overDock = false; // pointer is over the controls → never auto-hide
  let idleTimer = 0;

  // playback-preview volume (does NOT affect exported audio)
  let volume = $state(1);
  let muted = $state(false);

  // normalised audio peaks for the loaded Clip's Timeline (null until loaded)
  let waveform = /** @type {number[] | null} */ ($state(null));
  // filmstrip sprite (cols frames in one row) for the editor's Timeline
  // scrubbing + hover preview. FILM_COLS is fixed so the pure frame mapping stays
  // in sync. Bumped past the old 16 so a wide / fullscreen always-on strip has
  // enough distinct frames to fill without repeating (see the strip canvas).
  const FILM_COLS = 24;
  // Library-card hover sprites stay at the cheaper 16 — they render lazily per
  // card on hover, so their cost is kept off the just-optimised grid path.
  const CARD_COLS = 16;
  let clipFilmstrip = /** @type {string | null} */ ($state(null));
  // The always-on strip is painted on a <canvas> from the already-loaded
  // `clipFilmstrip` sprite (no extra ffmpeg run), and redrawn on resize purely on
  // the GPU — so toggling it on is instant and widening the window re-stripes
  // immediately instead of waiting on a fresh decode.
  /** @type {HTMLCanvasElement|null} */
  let stripCanvas = /** @type {HTMLCanvasElement | null} */ ($state(null));
  // $state so the redraw $effect re-runs when the sprite finishes decoding,
  // rather than relying solely on the imperative drawStrip() in img.onload.
  let stripImg = /** @type {HTMLImageElement | null} */ ($state(null)); // decoded <img> of clipFilmstrip, cached across redraws
  let hoverFrame = /** @type {HoverFrame | null} */ ($state(null)); // { x, idx, time } while hovering the Timeline
  // library-card hover scrubbing: lazily-fetched sprites + the hovered cell
  let filmstrips = /** @type {Record<string, string>} */ ($state({}));
  const filmReq = new Set();
  // Clips whose filmstrip decode permanently failed (e.g. corrupt) — kept out of
  // the retry path so a hover doesn't re-spawn ffmpeg for them on every pass.
  const filmFailed = new Set();
  let cardHover = /** @type {CardHover | null} */ ($state(null)); // { path, idx }
  // lazily-rendered poster thumbnails, keyed by clip path
  let thumbs = /** @type {Record<string, string>} */ ($state({}));
  // Clip durations learned from clip_thumbnail's banner, keyed by path. Fed to
  // clip_filmstrip on card hover so the scrub render skips a redundant probe
  // spawn (the poster already parsed the duration). 0/absent → backend probes.
  let clipDurations = /** @type {Record<string, number>} */ ($state({}));
  // Clips Klipt can't read (ffmpeg can't decode a frame, or the file's banner
  // has no valid duration) — a strong corruption signal. Flagged with a red
  // border in the grid. Detected as a side effect of the thumbnail render.
  let badClips = /** @type {Record<string, boolean>} */ ($state({}));
  const thumbReq = new Set();
  // Paths whose card is currently within the prefetch viewport. A queued
  // thumbnail re-checks this when a worker picks it up: a card the user flicked
  // past before its turn is skipped (and re-enqueues if scrolled back), so a
  // fast scroll through a large library doesn't spawn ffmpeg for every fly-by.
  const thumbVisible = new Set();
  // A LIFO stack, not a FIFO queue: when the user scrolls a large library the
  // IntersectionObserver enqueues every card it sweeps past, but the ones worth
  // rendering first are the most-recently-revealed (where the scroll settled).
  // Popping the newest request means the cards on screen now jump ahead of the
  // backlog of rows already scrolled past — those still render, just last.
  /** @type {string[]} */
  const thumbQueue = [];
  let thumbActive = 0;
  // requestAnimationFrame handle for the precise out-point stop while playing.
  let playRaf = 0;
  // How many ffmpeg processes render thumbnails concurrently. Scaled to the CPU
  // (clamped so a 2-core box still parallelises and a 32-core box doesn't spawn a
  // swarm of 100MB+ ffmpeg processes). Each process now renders a BATCH of cards
  // (see THUMB_BATCH / clip_thumbnails), so total in-flight cards is this times
  // the batch size.
  const THUMB_CONCURRENCY = Math.min(12, Math.max(4, navigator.hardwareConcurrency || 4));
  // Cards rendered per ffmpeg spawn. A single process takes N inputs -> N poster
  // outputs, paying the ~75ms process-spawn+init floor once for the whole batch
  // instead of per card (~2x faster grid fill, since the keyframe-only decode
  // made each poster overhead-bound). Kept small so thumbs still paint in steady
  // chunks during a scroll rather than in big all-or-nothing bursts, and so one
  // unreadable Clip (which aborts its whole ffmpeg batch -> per-card fallback)
  // poisons only a few neighbours.
  const THUMB_BATCH = 4;

  // Monotonic token so only the most recent loadClip() may commit its result;
  // a newer load (or closeClip) supersedes any still-in-flight probe.
  let loadGen = 0;

  let activeHandle = /** @type {"in" | "out" | null} */ ($state(null));
  let dragOver = $state(false);
  let busy = $state(false);
  let busyLabel = $state("");
  let toast = /** @type {Toast | null} */ ($state(null));
  // Compress progress 0..1, streamed from the backend; null when no bar shown.
  let compressProgress = /** @type {number | null} */ ($state(null));

  // Playback / export speed. 1x leaves Lossless a true stream-copy; any other
  // value retimes the clip (setpts + pitch-preserving atempo), which forces a
  // re-encode — so a non-1x Lossless export reroutes through the compress path.
  // Resets to 1x on each clip load so a slow-mo never silently carries over.
  const SPEEDS = [0.25, 0.5, 1, 1.5, 2, 4];
  let speed = $state(1);

  // True fullscreen (covers the taskbar) — distinct from the titlebar's
  // work-area maximize. Tracked so the toggle's icon and the titlebar's
  // visibility stay in sync, including OS-driven exits (Win+D, the OS chrome).
  let isFullscreen = $state(false);

  // export options
  let mode = $state("lossless"); // 'lossless' | 'compress' | 'gif' | 'audio'
  // Keep the audio stream in Lossless / Compress exports (off = silent video).
  let includeAudio = $state(true);
  // Audio-only output container ('m4a' = stream-copy/lossless, 'mp3' = re-encode).
  let audioFormat = $state("m4a"); // 'm4a' | 'mp3'
  let compressBy = $state("size"); // 'size' | 'quality'
  let targetMb = $state(25);
  // Quality mode picks an output resolution (downscale) — "source" keeps native.
  let quality = $state("source");
  let outputName = $state("");
  // Ordered worst → best (left → right): 480p … Source keeps native resolution.
  const QUALITY_PRESETS = [
    ["480", "480p"],
    ["720", "720p"],
    ["1080", "1080p"],
    ["source", "Source"],
  ];
  // GIF / animated-WebP export options (sane defaults; session-only).
  let gifFormat = $state("gif"); // 'gif' | 'webp'
  let gifFps = $state(15);
  let gifWidth = $state(640);
  const GIF_FPS = [10, 15, 24, 30];
  const GIF_WIDTHS = [360, 480, 640];
  // Timeline view toggles (persisted via localStorage). Off by default: the
  // hover preview is the primary scrubbing aid; these reveal the always-on
  // waveform / filmstrip strips.
  let showWaveform = $state(false);
  let showFilmstrip = $state(false);
  // When on, the source clip is moved to the Recycle Bin after a successful
  // save. Sticky across clips (off at launch) — trashing is reversible.
  let deleteOriginal = $state(false);

  // --- spatial crop (editor overlay) -----------------------------------
  // cropRect is the kept rectangle in SOURCE pixels ({x,y,w,h}) or null for the
  // whole frame; cropMode toggles the draw overlay. Per-clip + resolution-
  // specific, so both reset on every load (never persisted). Cropping forces a
  // re-encode (incompatible with lossless -c copy), so it routes via Compress.
  let cropRect = /** @type {{x:number,y:number,w:number,h:number} | null} */ ($state(null));
  let cropMode = $state(false);
  let cropDrag = /** @type {{x0:number,y0:number} | null} */ (null); // in-progress draw
  // Active pointer gesture on the crop overlay: drawing a new rect, moving the
  // existing one, or resizing it from a given handle. null when idle.
  let cropGesture = /** @type {{mode:"draw"|"move"|"resize", handle?:string, startSx:number, startSy:number, startRect?:{x:number,y:number,w:number,h:number}} | null} */ (null);
  let cropCursor = $state("crosshair"); // hover cursor over the overlay
  // The rendered <video> box within the stage, in px — the crop overlay matches
  // it exactly. Measured (not CSS-positioned) so it tracks the letterbox fit as
  // the window resizes or a clip of a different aspect loads.
  let videoBox = $state({ left: 0, top: 0, width: 0, height: 0 });

  // --- output preferences (Settings panel) ------------------------------
  // outputDir: override write location (null = next to the source Clip).
  // namingScheme: template for the default output stem ({name}, {action}).
  // accent: the theme accent colour token, applied live to --accent.
  let outputDir = /** @type {string | null} */ ($state(null));
  let namingScheme = $state("");
  let accent = $state("#fafafa");
  let showSettings = $state(false);
  // Curated accents bright enough for the dark UI text that sits on them.
  const ACCENTS = [
    { v: "#fafafa", label: "Mono" },
    { v: "#fbbf24", label: "Amber" },
    { v: "#4ade80", label: "Green" },
    { v: "#38bdf8", label: "Sky" },
    { v: "#a78bfa", label: "Violet" },
    { v: "#fb7185", label: "Rose" },
  ];

  const SIZE_PRESETS = [10, 25, 50];

  // Dropdown option lists for the export-options bar. Built once from the preset
  // arrays above so the bar reads as a row of compact pickers (current value +
  // caret) instead of long rows of number pills. Numeric lists are reversed so
  // the menu reads highest-at-top, lowest-at-bottom.
  const SPEED_OPTIONS = [...SPEEDS].reverse().map((s) => ({ value: s, label: s === 1 ? "1×" : `${s}×` }));
  const AUDIO_FMT_OPTIONS = [
    { value: "m4a", label: "M4A" },
    { value: "mp3", label: "MP3" },
  ];
  const GIF_FMT_OPTIONS = [
    { value: "gif", label: "GIF" },
    { value: "webp", label: "WebP" },
  ];
  const GIF_FPS_OPTIONS = [...GIF_FPS].reverse().map((f) => ({ value: f, label: String(f) }));
  const GIF_WIDTH_OPTIONS = [...GIF_WIDTHS].reverse().map((w) => ({ value: w, label: `${w} px` }));
  const SIZE_OPTIONS = [...SIZE_PRESETS].reverse().map((mb) => ({ value: mb, label: `${mb} MB` }));
  const RESOLUTION_OPTIONS = [...QUALITY_PRESETS].reverse().map(([value, label]) => ({ value, label }));
  // Waveform as one SVG path (one DOM node) instead of a <rect> per bucket — the
  // data is constant per Clip, so this recomputes only on load. (waveformPath is
  // pure + unit-tested in $lib/format.js.)
  const wavePath = $derived(waveformPath(waveform));
  const selLength = $derived(Math.max(0, outPoint - inPoint));
  // Current frame index for the readout — null when fps is unknown.
  const currentFrame = $derived(clip && clip.fps > 0 ? frameOf(currentTime, clip.fps) : null);
  const baseStem = $derived(clip ? clip.name.replace(/\.[^.]+$/, "") : "");
  // One source of truth for "what does the current mode produce" — the output
  // action token (feeds naming), the file extension, and the button label.
  const exportAction = $derived(
    mode === "compress" ? "small" : mode === "gif" ? gifFormat : mode === "audio" ? "audio" : "trim",
  );
  const defaultStem = $derived(`${baseStem}_${exportAction}`);
  const outExt = $derived(
    mode === "compress"
      ? "mp4"
      : mode === "gif"
        ? gifFormat
        : mode === "audio"
          ? audioFormat
          : clip
            ? clip.name.split(".").pop()
            : "mp4",
  );
  const modeLabel = $derived(
    mode === "compress" ? "Compress" : mode === "gif" ? gifFormat.toUpperCase() : mode === "audio" ? audioFormat.toUpperCase() : "Trim",
  );
  // A crop is active only when a rectangle is set; cropping forces re-encode.
  const cropActive = $derived(cropRect != null);
  const cropPct = $derived(clip ? cropToPercent(cropRect, clip.width, clip.height) : null);

  const games = $derived.by(() => {
    const m = new Map();
    for (const c of recentClips) {
      const g = c.game || "Other";
      m.set(g, (m.get(g) || 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  });
  // Sort the whole library once, keyed only to the ordering inputs — so typing
  // in the search box (which changes `query` below) does NOT re-sort thousands
  // of clips every keystroke. Filtering preserves order, so sorting upstream of
  // the filter is equivalent to the old filter-then-sort, just far cheaper.
  const sortedClips = $derived(sortClips(recentClips, sortKey, sortDir));
  const filteredClips = $derived.by(() => {
    const now = Date.now() / 1000;
    const q = query.trim().toLowerCase();
    return sortedClips.filter((c) => {
      if (gameFilter !== "all" && (c.game || "Other") !== gameFilter) return false;
      if (!matchesDateFilter(c.modified, dateFilter, now)) return false;
      if (q && !c.name.toLowerCase().includes(q) && !(c.game || "").toLowerCase().includes(q))
        return false;
      return true;
    });
  });

  // Render window: the column count the CSS auto-fill grid will produce at the
  // current width, the row pitch, and which slice of `filteredClips` to mount.
  const gridCols = $derived(gridColumns(gridW, GRID_MIN, GRID_GAP));
  const rowH = $derived(cardH + GRID_GAP);
  const gridWin = $derived(
    rowWindow(scrollTop, viewportH, gridTop, rowH, filteredClips.length, gridCols),
  );
  const visibleClips = $derived(filteredClips.slice(gridWin.startIdx, gridWin.endIdx));

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
  const SORT_OPTIONS = [
    { value: "date", label: "Date" },
    { value: "name", label: "Name" },
    { value: "size", label: "Size" },
  ];

  // Reset a filter if its selected value disappears (e.g. after switching folders).
  $effect(() => {
    if (gameFilter !== "all" && !games.some(([g]) => g === gameFilter)) gameFilter = "all";
  });

  // Track the scroll position (rAF-coalesced so a fast scroll updates the window
  // at most once per frame). `landingEl` is the scroll container.
  function onLandingScroll() {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      if (landingEl) scrollTop = landingEl.scrollTop;
    });
  }
  // Sync `scrollTop` to the DOM whenever the landing (re)mounts — returning from
  // the editor remounts it at scroll 0, and stale state would otherwise window to
  // an off-screen row and render blank until the next scroll.
  $effect(() => {
    if (landingEl) scrollTop = landingEl.scrollTop;
  });
  // Measure where the grid sits within the scroll content (the header + filters
  // above it scroll away, so the window math must offset by this) and the real
  // card height. Re-run when the layout that affects them changes: the grid
  // mounts/resizes, the filter bar wraps, or the list transitions empty↔full.
  $effect(() => {
    // deps — re-measure when these change.
    void [gridWrapEl, landingEl, gridW, viewportH, filteredClips.length];
    if (!gridWrapEl || !landingEl) return;
    const wrapTop = gridWrapEl.getBoundingClientRect().top;
    const landTop = landingEl.getBoundingClientRect().top;
    gridTop = wrapTop - landTop + landingEl.scrollTop;
    const card = gridWrapEl.querySelector(".card");
    if (card) {
      const h = /** @type {HTMLElement} */ (card).offsetHeight;
      if (h > 0) cardH = h;
    }
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
            // Keep observing (no unobserve): the visible set must stay accurate
            // as the user scrolls so pumpThumbs can skip cards that have since
            // left the viewport. enqueueThumb dedups, so repeat intersects are cheap.
            thumbVisible.add(current);
            enqueueThumb(current);
          } else {
            thumbVisible.delete(current);
          }
        }
      },
      { rootMargin: "200px" },
    );
    io.observe(node);
    return {
      /** @param {string} next */
      update(next) {
        // The card was recycled to a different clip (windowed grid reuses nodes):
        // the old path is no longer shown by THIS node.
        thumbVisible.delete(current);
        current = next;
      },
      destroy() {
        thumbVisible.delete(current);
        io.disconnect();
      },
    };
  }
  /** @param {string} path */
  function enqueueThumb(path) {
    if (thumbReq.has(path)) return;
    thumbReq.add(path);
    thumbQueue.push(path);
    pumpThumbs();
  }
  function pumpThumbs() {
    while (thumbActive < THUMB_CONCURRENCY && thumbQueue.length) {
      // Collect up to THUMB_BATCH still-visible cards into one ffmpeg batch.
      // Pop newest-first (LIFO) so freshly-revealed cards render before the
      // backlog of rows the user already scrolled past (see thumbQueue).
      /** @type {string[]} */
      const batch = [];
      while (batch.length < THUMB_BATCH && thumbQueue.length) {
        const path = /** @type {string} */ (thumbQueue.pop());
        // The card may have scrolled out of view while queued (fast scroll through
        // a big library). Skip it and clear the dedup marker so it re-enqueues if
        // scrolled back — don't spend an ffmpeg slot on a clip no longer on screen.
        // Already-rendered thumbs are kept (thumbs[path] set) so a still-visible
        // finished card isn't redone.
        if (!thumbVisible.has(path) && !thumbs[path]) {
          thumbReq.delete(path);
          continue;
        }
        batch.push(path);
      }
      if (!batch.length) continue; // everything popped had scrolled off

      thumbActive++;
      // One ffmpeg process renders the whole batch and reports each clip's health
      // (parsed from its banner), so no separate probe is needed: thumb === null
      // (even the per-clip fallback couldn't decode it) or healthy false (no
      // readable duration) flags a corrupt clip.
      invoke("clip_thumbnails", { paths: batch })
        .then(/** @param {import('$lib/types').BatchThumb[]} results */ (results) => {
          for (const res of results) {
            if (res.thumb) {
              thumbs[res.path] = convertFileSrc(res.thumb);
              if (res.duration > 0) clipDurations[res.path] = res.duration;
              if (res.healthy) delete badClips[res.path];
              else badClips[res.path] = true;
            } else {
              badClips[res.path] = true;
            }
          }
        })
        .catch(() => { for (const p of batch) badClips[p] = true; })
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
  // Keep `videoBox` matched to the rendered <video> so the crop overlay aligns
  // through window resizes and aspect changes (ResizeObserver fires on both).
  function measureVideoBox() {
    if (!videoEl) return;
    videoBox = {
      left: videoEl.offsetLeft,
      top: videoEl.offsetTop,
      width: videoEl.offsetWidth,
      height: videoEl.offsetHeight,
    };
  }
  $effect(() => {
    if (!videoEl) return;
    measureVideoBox();
    const ro = new ResizeObserver(measureVideoBox);
    ro.observe(videoEl);
    return () => ro.disconnect();
  });
  function toggleMute() {
    muted = !(muted || volume === 0);
    if (!muted && volume === 0) volume = 0.5;
  }
  function onVolInput() {
    if (volume > 0) muted = false;
    lsSet("volume", String(volume));
  }

  // --- localStorage view-prefs (namespaced + swallow-on-failure) --------
  /** @param {string} k @param {string} v */
  function lsSet(k, v) { try { localStorage.setItem("klipt:" + k, v); } catch {} }
  /** @param {string} k @returns {string | null} */
  function lsGet(k) { try { return localStorage.getItem("klipt:" + k); } catch { return null; } }

  // --- helpers ----------------------------------------------------------
  // fmt / fmtSize / waveformPath / previewName live in $lib/format.js (pure +
  // unit-tested). pct stays here — it reads the reactive `duration`.
  /** @param {number} t */
  function pct(t) {
    return duration > 0 ? (t / duration) * 100 : 0;
  }

  // --- folder / recent clips -------------------------------------------
  // Guards the persist effect so restoring settings on launch doesn't
  // immediately write them back (and so we never persist before the load).
  let settingsLoaded = false;

  async function loadSettings() {
    try {
      const s = /** @type {import('$lib/types').Settings} */ (await invoke("get_settings"));
      watchedFolder = s.watched_folder ?? null;
      if (s.export_mode) mode = s.export_mode;
      if (s.compress_by) compressBy = s.compress_by;
      if (typeof s.target_mb === "number") targetMb = s.target_mb;
      // Migrate legacy low/medium/high quality values to the new resolution set.
      if (s.quality) quality = QUALITY_PRESETS.some(([v]) => v === s.quality) ? s.quality : "source";
      if (typeof s.delete_original === "boolean") deleteOriginal = s.delete_original;
      if (typeof s.include_audio === "boolean") includeAudio = s.include_audio;
      if (s.audio_format) audioFormat = s.audio_format;
      outputDir = s.output_dir ?? null;
      if (typeof s.naming_scheme === "string") namingScheme = s.naming_scheme;
      if (s.accent) accent = s.accent;
      await refreshClips();
    } catch (e) {
      console.error(e);
    } finally {
      settingsLoaded = true;
    }
  }

  // The full settings object — always sent whole so a partial write can't drop
  // the watched folder or a previously-saved preference.
  function settingsPayload() {
    return {
      watched_folder: watchedFolder,
      export_mode: mode,
      compress_by: compressBy,
      target_mb: targetMb,
      quality,
      delete_original: deleteOriginal,
      include_audio: includeAudio,
      audio_format: audioFormat,
      output_dir: outputDir,
      naming_scheme: namingScheme.trim() || null,
      accent,
    };
  }
  async function saveSettings() {
    try {
      await invoke("set_settings", { settings: settingsPayload() });
    } catch (e) {
      console.error(e);
    }
  }

  // Persist export preferences whenever they change (after the initial load).
  $effect(() => {
    // Touch each tracked value so the effect re-runs when any changes.
    void [mode, compressBy, targetMb, quality, deleteOriginal, includeAudio, audioFormat, outputDir, namingScheme, accent];
    if (settingsLoaded) saveSettings();
  });
  // Persist the library sort (a view pref) to localStorage. Gated on the initial
  // load so it never clobbers the restored value with the default on first run.
  $effect(() => {
    void [sortKey, sortDir];
    if (settingsLoaded) { lsSet("sortKey", sortKey); lsSet("sortDir", sortDir); }
  });

  // Apply the theme accent live (and on launch) by overriding the token.
  $effect(() => {
    document.documentElement.style.setProperty("--accent", accent || "#fafafa");
  });

  // Mirror the chosen speed onto the preview element so the editor plays back at
  // the speed it will export. Skipped while a J/K/L shuttle owns playbackRate;
  // the shuttle restores `speed` (not 1x) when it ends.
  $effect(() => {
    if (videoEl && shuttleRate === 0) videoEl.playbackRate = speed;
  });

  // Live preview of the naming scheme, mirroring the Rust resolver (display
  // only — the backend `apply_naming_scheme` is the source of truth). previewName
  // is pure + unit-tested in $lib/format.js.
  const schemePreview = $derived(
    previewName(
      namingScheme,
      baseStem || "clip",
      exportAction,
      mode === "compress" ? "mp4" : mode === "gif" ? gifFormat : mode === "audio" ? audioFormat : "ext",
    ),
  );

  async function chooseOutputDir() {
    const picked = await open({ directory: true, multiple: false, title: "Choose output folder" });
    if (typeof picked === "string") outputDir = picked;
  }
  function resetOutputDir() { outputDir = null; }
  async function refreshClips() {
    if (!watchedFolder) return;
    try {
      recentClips = /** @type {import('$lib/types').ClipEntry[]} */ (await invoke("list_recent_clips", { folder: watchedFolder }));
    } catch {
      recentClips = [];
    }
  }
  async function chooseFolder() {
    const picked = await open({ directory: true, multiple: false, title: "Choose your clips folder" });
    if (typeof picked === "string") {
      watchedFolder = picked;
      gameFilter = "all";
      await saveSettings();
      await refreshClips();
    }
  }
  async function openFileDialog() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: [...VIDEO_EXTS] }],
    });
    if (typeof picked === "string") await loadClip(picked);
  }

  // --- library card context menu ---------------------------------------
  let cardMenu = /** @type {CardMenu | null} */ ($state(null));  // { x, y, clip } when open
  let renaming = /** @type {Renaming | null} */ ($state(null));  // { path, name } while the rename dialog is open

  /** @param {MouseEvent} e @param {import('$lib/types').ClipEntry} c */
  function openCardMenu(e, c) {
    e.preventDefault();
    e.stopPropagation();
    cardMenu = { x: e.clientX, y: e.clientY, clip: c };
  }
  function closeCardMenu() { cardMenu = null; }
  function onWindowPointerDown() { if (cardMenu) closeCardMenu(); }
  // Focus + select a freshly-mounted input (the rename field).
  /** @param {HTMLInputElement} node */
  function focusOnMount(node) { node.focus(); node.select?.(); }

  // Keep Tab focus inside an open modal and restore it to whatever opened the
  // modal once it closes. Escape-to-close is handled elsewhere (onKey / inline).
  const FOCUSABLE =
    'a[href],button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])';
  /** @param {HTMLElement} node */
  function trapFocus(node) {
    const opener = /** @type {HTMLElement | null} */ (document.activeElement);
    /** @param {KeyboardEvent} e */
    function onKeydown(e) {
      if (e.key !== "Tab") return;
      const items = /** @type {HTMLElement[]} */ (
        Array.from(node.querySelectorAll(FOCUSABLE))
      ).filter((el) => el.offsetParent !== null);
      if (items.length === 0) { e.preventDefault(); return; }
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
    node.addEventListener("keydown", onKeydown);
    // Move focus into the dialog on open — unless something inside already has it
    // (e.g. the rename field's use:focusOnMount).
    requestAnimationFrame(() => {
      if (node.contains(document.activeElement)) return;
      const first = /** @type {HTMLElement | undefined} */ (node.querySelectorAll(FOCUSABLE)[0]);
      (first ?? node).focus?.();
    });
    return {
      destroy() {
        node.removeEventListener("keydown", onKeydown);
        opener?.focus?.();
      },
    };
  }

  /** @param {import('$lib/types').ClipEntry} c */
  async function revealClip(c) {
    closeCardMenu();
    try { await revealItemInDir(c.path); } catch (e) { console.error(e); }
  }
  /** @param {import('$lib/types').ClipEntry} c */
  async function copyClip(c) {
    closeCardMenu();
    try {
      await invoke("copy_clip", { path: c.path });
      // Pure confirmation with no action — auto-dismiss after a moment, unless a
      // newer toast has since replaced it (guarded by reference identity).
      const t = { kind: "ok", copied: true, name: c.name };
      toast = t;
      setTimeout(() => { if (toast === t) toast = null; }, 2600);
    } catch (e) {
      toast = { kind: "err", msg: String(e) };
    }
  }
  /** @param {import('$lib/types').ClipEntry} c */
  function startRename(c) {
    closeCardMenu();
    renaming = { path: c.path, name: c.name.replace(/\.[^.]+$/, "") };
  }
  async function commitRename() {
    if (!renaming) return;
    const { path, name } = renaming;
    renaming = null;
    try {
      await invoke("rename_clip", { path, newName: name });
      await refreshClips();
    } catch (e) {
      toast = { kind: "err", msg: String(e) };
    }
  }
  /** @param {import('$lib/types').ClipEntry} c */
  async function deleteClipFromLibrary(c) {
    closeCardMenu();
    try {
      await invoke("delete_clip", { path: c.path });
      await refreshClips();
      toast = deletedToast(c.path);
    } catch (e) {
      toast = { kind: "err", msg: String(e) };
    }
  }

  // Restore a trashed Clip from the Recycle Bin (Undo on the delete toast).
  async function undoDelete() {
    if (!undoAvailable(toast)) return;
    const path = /** @type {Toast} */ (toast).trashedPath;
    toast = restoringToast(/** @type {Toast} */ (toast));
    try {
      await invoke("restore_clip", { path });
      toast = restoredToast(/** @type {Toast} */ (toast));
      await refreshClips();
    } catch (e) {
      toast = restoreFailedToast(/** @type {Toast} */ (toast), e);
    }
  }

  // --- load a clip ------------------------------------------------------
  /** @param {string} path */
  async function loadClip(path) {
    // Never start a load while an export (or another load) holds the busy lock —
    // a drop/open mid-export would otherwise clear the lock and swap the clip out
    // from under the running ffmpeg job.
    if (busy) return;
    const gen = ++loadGen;
    busy = true;
    busyLabel = "Loading";
    toast = null;
    try {
      const info = /** @type {import('$lib/types').ClipInfo} */ (await invoke("probe_clip", { path }));
      if (gen !== loadGen) return; // superseded by a newer load
      clip = { path, name: baseName(path), ...info };
      videoSrc = convertFileSrc(path);
      duration = info.duration || 0;
      inPoint = 0;
      outPoint = duration;
      currentTime = 0;
      speed = 1; // a speed change is per-clip; never carry it into the next clip
      outputName = "";
      // A crop is per-clip and resolution-specific — never carry it across loads.
      cropRect = null;
      cropMode = false;
      cropDrag = null;
      cropGesture = null;
      // mode is a remembered preference now — don't reset it per Clip.
      // Waveform + filmstrip are decorative + lazy — fetch without blocking.
      waveform = null;
      clipFilmstrip = null;
      hoverFrame = null;
      loadWaveform(path, gen);
      loadFilmstrip(path, gen, duration);
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
  // Fetch the Timeline waveform for `path`, committing only if this load is
  // still the active one. Failures are swallowed — the waveform is decorative.
  /** @param {string} path @param {number} gen */
  async function loadWaveform(path, gen) {
    try {
      const data = /** @type {number[]} */ (await invoke("clip_waveform", { path, buckets: 400 }));
      if (gen === loadGen) waveform = data;
    } catch {}
  }
  // Filmstrip for the loaded Clip — lazy, decorative, keyed to the active load.
  // Passes the duration we already probed so the backend skips a redundant probe.
  /** @param {string} path @param {number} gen @param {number} [dur] */
  async function loadFilmstrip(path, gen, dur) {
    try {
      const p = /** @type {string} */ (await invoke("clip_filmstrip", { path, cols: FILM_COLS, duration: dur ?? null }));
      if (gen === loadGen) clipFilmstrip = convertFileSrc(p);
    } catch {}
  }
  // Decode the loaded filmstrip sprite once into an <img> we can blit cells from.
  // Keyed to `clipFilmstrip` (a clip switch nulls then resets it, forcing a
  // reload); resizes reuse the cached image with no disk / ffmpeg work.
  $effect(() => {
    const src = clipFilmstrip;
    stripImg = null;
    if (!src) { drawStrip(); return; }
    const img = new Image();
    img.onload = () => { if (clipFilmstrip === src) { stripImg = img; drawStrip(); } };
    img.src = src;
  });
  // Repaint the always-on strip when it's shown, the sprite changes, the canvas
  // mounts, or the width changes (reading timelineWidth makes resize/fullscreen
  // reactive). All redraws are pure canvas work — no new ffmpeg run.
  $effect(() => {
    const _ = [showFilmstrip, clipFilmstrip, timelineWidth, stripCanvas, stripImg]; // deps
    drawStrip();
  });
  // Blit `k` evenly-spaced cells from the sprite across the strip's width, each
  // at the clip's aspect (fallback 16:9) so frames never stretch. `k` grows with
  // width up to the number of frames we actually have, capped so cells stay sane.
  function drawStrip() {
    const cv = stripCanvas;
    if (!cv || !stripImg) return;
    const cssW = cv.clientWidth;
    const cssH = cv.clientHeight;
    if (cssW <= 0 || cssH <= 0) return;
    const dpr = window.devicePixelRatio || 1;
    cv.width = Math.round(cssW * dpr);
    cv.height = Math.round(cssH * dpr);
    const ctx = cv.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);
    const cellSrcW = stripImg.naturalWidth / FILM_COLS;
    const cellSrcH = stripImg.naturalHeight;
    const aspect = clip && clip.height > 0 ? clip.width / clip.height : 16 / 9;
    const k = Math.max(4, Math.min(FILM_COLS, Math.round(cssW / (cssH * aspect))));
    const dCellW = cssW / k;
    for (let i = 0; i < k; i++) {
      const srcIdx = k === 1 ? 0 : Math.round((i * (FILM_COLS - 1)) / (k - 1));
      ctx.drawImage(stripImg, srcIdx * cellSrcW, 0, cellSrcW, cellSrcH, i * dCellW, 0, dCellW, cssH);
    }
  }
  // Timeline hover preview: map the pointer to a time + filmstrip cell. Skipped
  // while a handle/region drag owns the pointer.
  /** @param {PointerEvent} e */
  function onTimelineHover(e) {
    // Shown on hover, track-scrub, and Region slide (handle drags capture the
    // pointer, so this never fires then). Needs the sprite + a known duration.
    if (!timelineEl || duration <= 0 || !clipFilmstrip) { hoverFrame = null; return; }
    const r = timelineEl.getBoundingClientRect();
    const t = hoverTime(e.clientX, r.left, r.width, duration);
    hoverFrame = { x: e.clientX - r.left, idx: frameIndexAt(t, FILM_COLS, duration), time: t };
  }
  // Keep the preview up while a scrub is mid-drag even if the pointer slips out.
  function clearHoverFrame() { if (!scrubbing) hoverFrame = null; }

  // Library-card hover scrubbing: fetch the sprite on first hover, then map the
  // pointer's x across the card to a frame cell.
  /** @param {string} path */
  function enqueueFilmstrip(path) {
    if (filmReq.has(path) || filmFailed.has(path)) return;
    filmReq.add(path);
    invoke("clip_filmstrip", { path, cols: CARD_COLS, duration: clipDurations[path] ?? null })
      .then(/** @param {string} p */ (p) => (filmstrips[path] = convertFileSrc(p)))
      // A failure is treated as permanent: drop the in-flight marker and remember
      // it failed so hovering the card again doesn't re-spawn ffmpeg endlessly.
      .catch(() => { filmReq.delete(path); filmFailed.add(path); });
  }
  /** @param {PointerEvent} e @param {string} path */
  function onCardHover(e, path) {
    const r = /** @type {HTMLElement} */ (e.currentTarget).getBoundingClientRect();
    const frac = r.width > 0 ? Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) : 0;
    // frac is already 0..1, so a unit "duration" turns it into a cell index.
    cardHover = { path, idx: frameIndexAt(frac, CARD_COLS, 1) };
  }
  /** @param {string} path */
  function clearCardHover(path) {
    if (cardHover?.path === path) cardHover = null;
  }
  function closeClip() {
    loadGen++; // cancel any in-flight loadClip
    stopShuttle();
    if (videoEl) videoEl.pause();
    clip = null;
    videoSrc = null;
    waveform = null;
    clipFilmstrip = null;
    hoverFrame = null;
    cropRect = null;
    cropMode = false;
    cropDrag = null;
    playing = false;
    clearTimeout(idleTimer);
    uiVisible = true;
    refreshClips();
  }

  function onMeta() {
    if (videoEl && isFinite(videoEl.duration)) {
      duration = videoEl.duration;
      if (outPoint === 0 || outPoint > duration) outPoint = duration;
    }
    measureVideoBox(); // the real aspect is known now → realign the crop overlay
  }
  function watchPlayback() {
    if (!videoEl) return;
    currentTime = videoEl.currentTime;
    // Reading in/out fresh each frame lets looping respect handles dragged live.
    const d = loopDecision(currentTime, inPoint, outPoint, loopEnabled, selectionOnly, duration);
    if (d.action === "wrap") {
      videoEl.currentTime = /** @type {number} */ (d.seekTo);
      currentTime = /** @type {number} */ (d.seekTo);
    } else if (d.action === "stop") {
      videoEl.pause();
      videoEl.currentTime = /** @type {number} */ (d.seekTo);
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

  // Toggle taskbar-covering fullscreen. The Rust command owns the state (it
  // implements fullscreen via window geometry, not tao's set_fullscreen) and
  // returns the new value, so we just mirror it.
  async function toggleFullscreen() {
    try {
      isFullscreen = await invoke("toggle_fullscreen");
    } catch (e) {
      console.error(e);
    }
  }

  // --- transport --------------------------------------------------------
  function togglePlay() {
    if (!videoEl) return;
    stopShuttle(); // a normal play/pause cancels any J/K/L shuttle (keeps the chosen speed)
    if (videoEl.paused) {
      // In selection scope, jump to the in-point if the playhead sits outside the
      // Region so Play always previews the trim. In whole-Clip scope, play from
      // wherever we are (but restart from 0 if parked at the very end).
      if (selectionOnly) {
        if (currentTime < inPoint || currentTime >= outPoint) videoEl.currentTime = inPoint;
      } else if (duration > 0 && currentTime >= duration - 0.05) {
        videoEl.currentTime = 0;
      }
      videoEl.play();
      playing = true;
    } else {
      videoEl.pause();
      playing = false;
    }
  }
  function toggleSelectionOnly() {
    selectionOnly = !selectionOnly;
    lsSet("selectionOnly", selectionOnly ? "1" : "0");
  }

  // --- auto-hide chrome (player-style) ----------------------------------
  // Hide the header + dock after a short idle while playing; reveal on pointer
  // movement. Never hide while paused, hovering the controls, exporting, or
  // mid-drag — those are all "the user is using it" states.
  const IDLE_MS = 2600;
  function scheduleHide() {
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      if (playing && !overDock && !busy && !activeHandle && !regionDrag && !scrubbing) uiVisible = false;
    }, IDLE_MS);
  }
  function revealUI() {
    uiVisible = true;
    scheduleHide();
  }
  function onEditorLeave() {
    // Mouse left the window → hide all chrome so only the frame shows, whether
    // playing or paused. (Idle auto-hide stays playing-only.)
    if (!busy) uiVisible = false;
  }
  function onPlay() { playing = true; startWatch(); scheduleHide(); }
  function onPause() { playing = false; stopWatch(); clearTimeout(idleTimer); uiVisible = true; }

  // --- J/K/L shuttle ----------------------------------------------------
  // Forward shuttle steps the native playbackRate up; reverse has no native
  // support, so we walk currentTime backwards on a rAF. shuttleRate is signed:
  // >0 forward, <0 reverse, 0 = not shuttling.
  const SHUTTLE_MAX = 8;
  let shuttleRate = 0;
  let revRaf = 0;
  let revPrev = 0;
  function stopReverse() {
    if (revRaf) { cancelAnimationFrame(revRaf); revRaf = 0; }
    revPrev = 0;
  }
  function stopShuttle() {
    stopReverse();
    shuttleRate = 0;
    if (videoEl) videoEl.playbackRate = speed; // restore the chosen speed, not 1x
  }
  /** @param {number} ts */
  function reverseStep(ts) {
    if (!videoEl || shuttleRate >= 0) { stopReverse(); return; }
    if (!revPrev) revPrev = ts;
    const dt = (ts - revPrev) / 1000;
    revPrev = ts;
    let t = videoEl.currentTime + shuttleRate * dt; // shuttleRate is negative
    if (t <= 0) { t = 0; videoEl.currentTime = 0; currentTime = 0; stopShuttle(); return; }
    videoEl.currentTime = t;
    currentTime = t;
    revRaf = requestAnimationFrame(reverseStep);
  }
  function shuttleForward() {
    if (!videoEl) return;
    stopReverse();
    shuttleRate = shuttleRate >= 1 ? Math.min(shuttleRate * 2, SHUTTLE_MAX) : 1;
    videoEl.playbackRate = shuttleRate;
    videoEl.play();
    playing = true;
  }
  function shuttleRewind() {
    if (!videoEl) return;
    videoEl.pause();
    playing = false;
    videoEl.playbackRate = speed;
    shuttleRate = shuttleRate <= -1 ? Math.max(shuttleRate * 2, -SHUTTLE_MAX) : -1;
    stopReverse();
    revRaf = requestAnimationFrame(reverseStep);
  }
  function shuttlePause() {
    stopShuttle();
    if (videoEl) videoEl.pause();
    playing = false;
  }

  // --- frame stepping (preview navigation only) -------------------------
  // Steps the playhead one frame; does NOT make Trim frame-accurate — lossless
  // Trim still snaps to keyframes (ADR 0002). Falls back to 30fps for stepping
  // when the Clip's real fps is unknown so the keys still do something.
  /** @param {number} dir */
  function stepFrame(dir) {
    if (!videoEl || !clip) return;
    stopShuttle();
    videoEl.pause();
    playing = false;
    const fps = clip.fps > 0 ? clip.fps : 30;
    const next = timeOf(frameOf(currentTime, fps) + dir, fps);
    const t = Math.max(0, Math.min(duration, next));
    videoEl.currentTime = t;
    currentTime = t;
  }

  // --- timeline interaction --------------------------------------------
  // Map an x to a time against the element actually being scrubbed (the timeline
  // by default, but the waveform / filmstrip strips wire the same handlers and
  // must measure against themselves — they're laid out at the timeline's width
  // today, but mapping against the real element keeps that from silently breaking).
  /** @param {number} clientX @param {HTMLElement} [el] */
  function timeFromX(clientX, el) {
    const r = (el ?? /** @type {HTMLElement} */ (timelineEl)).getBoundingClientRect();
    let f = r.width > 0 ? (clientX - r.left) / r.width : 0;
    f = Math.max(0, Math.min(1, f));
    return f * duration;
  }
  /** @param {number} clientX @param {HTMLElement} [el] */
  function seekTo(clientX, el) {
    const t = timeFromX(clientX, el);
    if (videoEl) videoEl.currentTime = t;
    currentTime = t;
  }
  // Click or drag the track to scrub the playhead (YouTube-style). The Region
  // has its own pointer handlers, so this only fires on the bare track.
  let scrubbing = false;
  /** @param {PointerEvent} e */
  function onTrackDown(e) {
    if (activeHandle) return;
    scrubbing = true;
    const el = /** @type {HTMLElement} */ (e.currentTarget);
    try { el.setPointerCapture(e.pointerId); } catch {}
    seekTo(e.clientX, el);
  }
  /** @param {PointerEvent} e */
  function onTimelineMove(e) {
    if (scrubbing) seekTo(e.clientX, /** @type {HTMLElement} */ (e.currentTarget));
    onTimelineHover(e);
  }
  /** @param {PointerEvent} e */
  function onTimelineUp(e) {
    if (!scrubbing) return;
    scrubbing = false;
    try { /** @type {Element} */ (e.currentTarget).releasePointerCapture(e.pointerId); } catch {}
  }
  /** @param {"in" | "out"} which @param {PointerEvent} e */
  function startHandle(which, e) {
    e.stopPropagation();
    activeHandle = which;
    try { /** @type {Element} */ (e.currentTarget).setPointerCapture(e.pointerId); } catch {}
  }
  /** @param {PointerEvent} e */
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
  /** @param {PointerEvent} e */
  function endHandle(e) {
    if (!activeHandle) return;
    try { /** @type {Element} */ (e.currentTarget).releasePointerCapture(e.pointerId); } catch {}
    activeHandle = null;
  }

  // Grab the middle of the Region to slide the whole keep-window. A grab that
  // never crosses the threshold is treated as a click → seek there, so the user
  // can still scrub into the middle even when the Region spans the whole Clip.
  // Sliding applies an absolute delta from the grab point (accurate even if
  // pointer events coalesce); slideRegion preserves length and clamps to the Clip.
  let regionDrag = /** @type {RegionDrag | null} */ ($state(null)); // { startX, startIn, startOut, moved, scrub }
  const REGION_DRAG_THRESHOLD = 4; // px before a grab becomes a slide vs. a click
  // The Region spans (almost) the whole Clip → there's nowhere to slide, so a
  // drag scrubs the playhead instead. (This also fixes the playhead snapping to
  // 0, which happened when a full-width slide pinned currentTime to inPoint.)
  function regionIsFullClip() {
    return duration > 0 && inPoint <= 0.001 && outPoint >= duration - 0.001;
  }
  /** @param {PointerEvent} e */
  function startRegionDrag(e) {
    e.stopPropagation(); // the Region owns this gesture, not the track scrub
    regionDrag = { startX: e.clientX, startIn: inPoint, startOut: outPoint, moved: false, scrub: regionIsFullClip() };
    try { /** @type {Element} */ (e.currentTarget).setPointerCapture(e.pointerId); } catch {}
  }
  /** @param {PointerEvent} e */
  function moveRegionDrag(e) {
    if (!regionDrag || !timelineEl) return;
    const r = timelineEl.getBoundingClientRect();
    const deltaPx = e.clientX - regionDrag.startX;
    if (!regionDrag.moved && Math.abs(deltaPx) < REGION_DRAG_THRESHOLD) {
      onTimelineHover(e); // still a potential click — keep the preview live
      return;
    }
    regionDrag.moved = true;
    if (regionDrag.scrub) {
      seekTo(e.clientX); // full-Clip Region → scrub the playhead
      onTimelineHover(e);
      return;
    }
    const deltaSecs = (deltaPx / r.width) * duration;
    const next = slideRegion(deltaSecs, regionDrag.startIn, regionDrag.startOut, duration);
    inPoint = next.inPoint;
    outPoint = next.outPoint;
    if (videoEl) { videoEl.currentTime = inPoint; currentTime = inPoint; }
    onTimelineHover(e);
  }
  /** @param {PointerEvent} e */
  function endRegionDrag(e) {
    if (!regionDrag) return;
    try { /** @type {Element} */ (e.currentTarget).releasePointerCapture(e.pointerId); } catch {}
    if (!regionDrag.moved) seekTo(e.clientX); // a tap on the Region seeks
    regionDrag = null;
  }
  function setInHere() { inPoint = Math.min(currentTime, outPoint - 0.05); }
  function setOutHere() { outPoint = Math.max(currentTime, inPoint + 0.05); }

  // Keyboard nudging for the In/Out handles when focused (the only points with no
  // global keyboard equivalent). One frame per Arrow, ~1s with Shift; clamped so
  // the Region stays valid. stopPropagation keeps the global frame-step off.
  /** @param {"in" | "out"} which @param {number} delta */
  function nudgeHandle(which, delta) {
    if (which === "in") inPoint = Math.max(0, Math.min(inPoint + delta, outPoint - 0.05));
    else outPoint = Math.min(duration, Math.max(outPoint + delta, inPoint + 0.05));
    if (videoEl) { videoEl.currentTime = which === "in" ? inPoint : outPoint; currentTime = videoEl.currentTime; }
  }
  /** @param {"in" | "out"} which @param {KeyboardEvent} e */
  function onHandleKey(which, e) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    e.stopPropagation();
    const frame = clip && clip.fps > 0 ? 1 / clip.fps : 1 / 30;
    const step = e.shiftKey ? 1 : frame;
    nudgeHandle(which, e.key === "ArrowLeft" ? -step : step);
  }

  /** @param {string} m */
  function setMode(m) {
    mode = m;
    // Crop only applies to the video re-encode path (Compress); drop it when
    // switching to GIF/Audio so a stale rect can't ride along unexpectedly.
    if (m !== "compress" && m !== "lossless") {
      cropMode = false;
      cropRect = null;
      cropDrag = null;
      cropGesture = null;
    }
  }

  // --- spatial crop overlay --------------------------------------------
  function toggleCrop() {
    if (!clip) return;
    cropMode = !cropMode;
    // Turning the tool off removes the crop entirely — the box must not linger
    // (and an unset crop re-enables the Lossless path).
    if (!cropMode) {
      cropRect = null;
      cropDrag = null;
      cropGesture = null;
    }
  }
  // Source-px tolerance for grabbing a crop edge/corner — ~12 CSS px mapped into
  // source space via the rendered video box.
  function cropHandleTol() {
    if (!clip || videoBox.width <= 0) return 12;
    return (12 * clip.width) / videoBox.width;
  }
  /** @param {string} h */
  function cursorForHandle(h) {
    if (h === "move") return "move";
    if (h === "nw" || h === "se") return "nwse-resize";
    if (h === "ne" || h === "sw") return "nesw-resize";
    if (h === "n" || h === "s") return "ns-resize";
    if (h === "e" || h === "w") return "ew-resize";
    return cropMode ? "crosshair" : "default";
  }
  /** @param {PointerEvent} e */
  function onCropDown(e) {
    if (!clip) return;
    e.stopPropagation(); // don't toggle play (the overlay sits over the video)
    const r = /** @type {HTMLElement} */ (e.currentTarget).getBoundingClientRect();
    const { sx, sy } = screenToSource(e.clientX, e.clientY, r, clip.width, clip.height);
    // Grab an existing crop's handle/body first; only an empty-area press starts a
    // brand-new rectangle (so a finished crop is editable, not wiped on click).
    const hit = cropActive ? hitTestCrop(sx, sy, cropRect, cropHandleTol()) : null;
    if (hit === "move") {
      cropGesture = { mode: "move", startSx: sx, startSy: sy, startRect: /** @type {any} */ ({ ...cropRect }) };
    } else if (hit) {
      cropGesture = { mode: "resize", handle: hit, startSx: sx, startSy: sy, startRect: /** @type {any} */ ({ ...cropRect }) };
    } else {
      cropGesture = { mode: "draw", startSx: sx, startSy: sy };
      cropDrag = { x0: sx, y0: sy };
      cropRect = null; // rebuild as the drag grows
    }
    try { /** @type {Element} */ (e.currentTarget).setPointerCapture(e.pointerId); } catch {}
  }
  /** @param {PointerEvent} e */
  function onCropMove(e) {
    if (!clip) return;
    const r = /** @type {HTMLElement} */ (e.currentTarget).getBoundingClientRect();
    const { sx, sy } = screenToSource(e.clientX, e.clientY, r, clip.width, clip.height);
    if (!cropGesture) {
      // Idle hover — reflect what a press here would do via the cursor.
      const hit = cropActive ? hitTestCrop(sx, sy, cropRect, cropHandleTol()) : null;
      cropCursor = cursorForHandle(hit ?? "");
      return;
    }
    if (cropGesture.mode === "draw" && cropDrag) {
      cropRect = normalizeCrop(cropDrag.x0, cropDrag.y0, sx, sy, clip.width, clip.height);
    } else if (cropGesture.mode === "move" && cropGesture.startRect) {
      cropRect = moveCrop(cropGesture.startRect, sx - cropGesture.startSx, sy - cropGesture.startSy, clip.width, clip.height);
    } else if (cropGesture.mode === "resize" && cropGesture.startRect && cropGesture.handle) {
      cropRect = resizeCrop(cropGesture.startRect, cropGesture.handle, sx, sy, clip.width, clip.height);
    }
  }
  /** @param {PointerEvent} e */
  function onCropUp(e) {
    if (!cropGesture) return;
    try { /** @type {Element} */ (e.currentTarget).releasePointerCapture(e.pointerId); } catch {}
    cropGesture = null;
    cropDrag = null;
    // A real crop forces a re-encode — leave the lossless path automatically.
    if (cropRect && mode === "lossless") mode = "compress";
  }

  function toggleWaveform() {
    showWaveform = !showWaveform;
    lsSet("showWaveform", showWaveform ? "1" : "0");
  }
  function toggleFilmstrip() {
    showFilmstrip = !showFilmstrip;
    lsSet("showFilmstrip", showFilmstrip ? "1" : "0");
  }
  function toggleSortDir() {
    sortDir = sortDir === "asc" ? "desc" : "asc";
  }

  // --- export -----------------------------------------------------------
  async function exportClip() {
    if (busy || !clip || selLength <= 0) return; // guard: Enter can fire while already exporting
    busy = true;
    // A non-1x Lossless export is really a re-encode (it can't stream-copy), so
    // it gets the "Re-encoding" label and the streamed progress bar too.
    const reencode = mode === "compress" || (mode === "lossless" && speed !== 1);
    busyLabel =
      mode === "compress" ? "Compressing" : mode === "gif" ? "Rendering" : mode === "audio" ? "Extracting audio" : speed !== 1 ? "Re-encoding" : "Trimming";
    uiVisible = true; // keep the dock (progress) visible even if idle-hidden
    // Re-encodes (Compress, and speed-changed Lossless) stream a progress bar;
    // a plain stream-copy Trim / GIF / audio keeps the spinner.
    compressProgress = reencode ? 0 : null;
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
          targetMb: compressBy === "size" ? targetMb : null,
          quality: compressBy === "quality" ? quality : null,
          includeAudio,
          crop: cropRect,
          speed,
        });
      } else if (mode === "gif") {
        res = await invoke("gif_clip", {
          path: clip.path,
          start: inPoint,
          end: outPoint,
          outputName: name,
          format: gifFormat,
          fps: gifFps,
          width: gifWidth,
          speed,
        });
      } else if (mode === "audio") {
        res = await invoke("audio_clip", {
          path: clip.path,
          start: inPoint,
          end: outPoint,
          outputName: name,
          format: audioFormat,
          speed,
        });
      } else if (speed !== 1) {
        // Lossless can't retime a stream copy, so a speed change re-encodes at
        // near-lossless quality (source resolution, no crop) via the compress
        // path. The user opted into this when they chose a non-1x speed.
        res = await invoke("compress_clip", {
          path: clip.path,
          start: inPoint,
          end: outPoint,
          outputName: name,
          mode: "quality",
          targetMb: null,
          quality: "source",
          includeAudio,
          crop: null,
          speed,
        });
      } else {
        res = await invoke("trim_clip", {
          path: clip.path,
          start: inPoint,
          end: outPoint,
          outputName: name,
          includeAudio,
        });
      }

      // GIF/WebP and audio-only are derivatives, never a replacement — never
      // trash the source video for them.
      if (deleteOriginal && mode !== "gif" && mode !== "audio") {
        // Release the file handle the <video> holds before trashing, otherwise
        // Windows refuses to move a file that's still open for playback.
        const original = clip.path;
        if (videoEl) { videoEl.pause(); videoEl.removeAttribute("src"); videoEl.load(); }
        try {
          await invoke("delete_clip", { path: original });
          toast = trashedToast(res, original);
        } catch (e) {
          toast = { kind: "ok", ...res, trashError: String(e) };
        }
        // Either way the trimmed file is saved; return to the library so we're
        // never left showing a clip whose source handle we just released.
        closeClip();
        return;
      }

      toast = { kind: "ok", ...res };
    } catch (e) {
      toast = { kind: "err", msg: String(e) };
    } finally {
      busy = false;
      busyLabel = "";
      compressProgress = null;
    }
  }
  async function revealOutput() {
    if (toast?.path) {
      try { await revealItemInDir(toast.path); } catch (e) { console.error(e); }
    }
  }

  // --- keyboard ---------------------------------------------------------
  /** @param {EventTarget | null} t */
  function isTypingTarget(t) {
    if (!t) return false;
    const el = /** @type {HTMLElement} */ (t);
    return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable;
  }
  /** @param {KeyboardEvent} e */
  function onKey(e) {
    if (showSettings && e.key === "Escape") { showSettings = false; return; }
    if (cardMenu && e.key === "Escape") { closeCardMenu(); return; }
    // Escape leaves fullscreen before it would fall through to "back" (close clip).
    if (isFullscreen && e.key === "Escape") { e.preventDefault(); toggleFullscreen(); return; }
    // F11 toggles fullscreen everywhere (editor *and* library) — not just when a
    // clip is open — so you can still exit fullscreen after pressing Back.
    if (e.key === "F11" && !isTypingTarget(e.target)) { e.preventDefault(); toggleFullscreen(); return; }
    const action = resolveKey(e, { hasClip: !!clip, isTyping: isTypingTarget(e.target) });
    if (!action) return;
    e.preventDefault();
    switch (action) {
      case "trim": exportClip(); break;
      case "back": closeClip(); break;
      case "playPause": togglePlay(); break;
      case "setIn": setInHere(); break;
      case "setOut": setOutHere(); break;
      case "shuttleRewind": shuttleRewind(); break;
      case "shuttlePause": shuttlePause(); break;
      case "shuttleForward": shuttleForward(); break;
      case "frameBack": stepFrame(-1); break;
      case "frameForward": stepFrame(1); break;
      case "fullscreen": toggleFullscreen(); break;
    }
  }

  onMount(() => {
    loadSettings();
    const v = parseFloat(/** @type {string} */ (lsGet("volume")));
    if (isFinite(v)) volume = Math.max(0, Math.min(1, v));
    showWaveform = lsGet("showWaveform") === "1";
    showFilmstrip = lsGet("showFilmstrip") === "1";
    if (lsGet("selectionOnly") === "0") selectionOnly = false;
    const sk = lsGet("sortKey");
    if (sk && SORT_OPTIONS.some((o) => o.value === sk)) sortKey = sk;
    const sd = lsGet("sortDir");
    if (sd === "asc" || sd === "desc") sortDir = sd;
    // Live compression progress from the backend's streamed ffmpeg run.
    // `disposed` guards the async listen() registrations: if the component
    // unmounts before a promise resolves, the unlisten fn is called immediately
    // instead of being stored too late to ever run (a leaked listener).
    let disposed = false;
    /** @type {(() => void) | undefined} */
    let unProgress;
    listen("compress-progress", /** @param {{payload: number}} e */ (e) => { compressProgress = e.payload; })
      .then((f) => { if (disposed) f(); else unProgress = f; });
    /** @type {(() => void) | undefined} */
    let un;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "over" || p.type === "enter") dragOver = true;
        else if (p.type === "leave") dragOver = false;
        else if (p.type === "drop") {
          dragOver = false;
          const f = (p.paths || []).find(isVideoFile);
          if (f) loadClip(f);
        }
      })
      .then((f) => { if (disposed) f(); else un = f; });
    return () => { disposed = true; un && un(); unProgress && unProgress(); };
  });
</script>

<svelte:window onkeydown={onKey} onpointerdown={onWindowPointerDown} />

<div class="app">
  <!-- ============ TITLEBAR ============ -->
  <!-- The custom chrome would overlap a true-fullscreen view, so drop it there. -->
  {#if !isFullscreen}
    <Titlebar />
  {/if}

  <div class="body">
    {#if !clip}
      <!-- ============ LANDING ============ -->
      <section class="landing" bind:this={landingEl} bind:clientHeight={viewportH} onscroll={onLandingScroll}>
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
          <button class="iconlink" onclick={() => (showSettings = true)} aria-label="Settings" title="Settings">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
          </button>
        </div>

        {#if recentClips.length > 0}
          <div class="filters">
            <div class="fgroup">
              <label class="search">
                <svg width="13" height="13" viewBox="0 0 16 16" fill="none" aria-hidden="true"><circle cx="7" cy="7" r="4.5" stroke="currentColor" stroke-width="1.3"/><path d="M10.5 10.5 L14 14" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>
                <input type="text" bind:value={query} placeholder="Search clips" aria-label="Search clips" spellcheck="false" autocomplete="off" />
                {#if query}
                  <button class="searchclear" onclick={() => (query = "")} aria-label="Clear search" title="Clear">
                    <svg width="11" height="11" viewBox="0 0 11 11"><path d="M1 1 L10 10 M10 1 L1 10" stroke="currentColor" stroke-width="1.2"/></svg>
                  </button>
                {/if}
              </label>
              <Dropdown bind:value={gameFilter} options={gameOptions} label="Game" ariaLabel="Filter by game" />
              <Dropdown bind:value={dateFilter} options={DATE_OPTIONS} label="Date" ariaLabel="Filter by date" />
              <Dropdown bind:value={sortKey} options={SORT_OPTIONS} label="Sort" ariaLabel="Sort clips" />
              <button class="iconlink sortdir" class:asc={sortDir === "asc"} onclick={toggleSortDir} aria-label="Toggle sort direction" title={sortDir === "asc" ? "Ascending — click for descending" : "Descending — click for ascending"}>
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M8 3 V13 M4.5 9.5 L8 13 L11.5 9.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/></svg>
              </button>
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
                : "Try a different search, game, or date filter."}
            </p>
          </div>
        {:else}
          <!-- Windowed grid: only the cards in/near the viewport are mounted.
               `gridwrap` reserves the full scroll height; the inner grid is
               offset to the first rendered row. -->
          <div class="gridwrap" bind:this={gridWrapEl} bind:clientWidth={gridW} style="height:{gridWin.totalHeight}px">
            <div class="grid" style="transform:translateY({gridWin.padTop}px)">
              {#each visibleClips as c (c.path)}
                <button class="card" class:bad={badClips[c.path]} use:thumbOnVisible={c.path}
                  onclick={() => loadClip(c.path)} oncontextmenu={(/** @type {MouseEvent} */ e) => openCardMenu(e, c)} title={c.path}
                  onpointerenter={() => enqueueFilmstrip(c.path)}
                  onpointermove={(/** @type {PointerEvent} */ e) => onCardHover(e, c.path)}
                  onpointerleave={() => clearCardHover(c.path)}>
                <div class="thumb" class:loaded={thumbs[c.path]}>
                  {#if thumbs[c.path]}
                    <img src={thumbs[c.path]} alt="" loading="lazy" draggable="false" />
                  {:else}
                    <svg width="22" height="22" viewBox="0 0 24 24" fill="none"><path d="M8 6 L18 12 L8 18 Z" fill="currentColor"/></svg>
                  {/if}
                  {#if cardHover?.path === c.path && filmstrips[c.path]}
                    <div class="thumbscrub" style="background-image:url({filmstrips[c.path]}); background-size:{CARD_COLS * 100}% 100%; background-position-x:{(cardHover.idx / (CARD_COLS - 1)) * 100}%"></div>
                  {/if}
                  {#if badClips[c.path]}
                    <div class="badtag" title="This file may be corrupted or unreadable">
                      <svg width="11" height="11" viewBox="0 0 16 16" fill="none"><path d="M8 2 L14.5 13.5 H1.5 Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/><path d="M8 6.5 V9.2 M8 11 V11.4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>
                      Can't read
                    </div>
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
          </div>
        {/if}
      </section>
    {:else}
      <!-- ============ EDITOR ============ -->
      <section class="editor" class:uihidden={!uiVisible} onpointermove={revealUI} onpointerleave={onEditorLeave}>
        <div class="stage">
          <!-- svelte-ignore a11y_media_has_caption -->
          <video
            bind:this={videoEl}
            src={videoSrc}
            onloadedmetadata={onMeta}
            onplay={onPlay}
            onpause={onPause}
            onclick={(/** @type {MouseEvent} */ e) => { if (e.detail === 1) togglePlay(); }}
            ondblclick={toggleFullscreen}
          ></video>
          {#if clip && (cropMode || cropActive)}
            <!-- Crop overlay sized to the rendered video box (measured). Captures
                 pointers only while drawing so play/pause still works otherwise. -->
            <div
              class="cropoverlay"
              class:drawing={cropMode}
              style="left:{videoBox.left}px; top:{videoBox.top}px; width:{videoBox.width}px; height:{videoBox.height}px; cursor:{cropCursor}"
              onpointerdown={onCropDown}
              onpointermove={onCropMove}
              onpointerup={onCropUp}
              role="presentation"
            >
              {#if cropPct}
                <div class="croprect" style="left:{cropPct.left}%; top:{cropPct.top}%; width:{cropPct.width}%; height:{cropPct.height}%">
                  {#each ["nw", "n", "ne", "e", "se", "s", "sw", "w"] as h}
                    <span class="crophandle {h}" aria-hidden="true"></span>
                  {/each}
                </div>
              {/if}
              {#if cropMode && !cropActive}
                <div class="crophint">Drag to set the crop area</div>
              {/if}
            </div>
          {/if}
        </div>

        <!-- top overlay -->
        <header class="ehead">
          <button class="btn ghost sm glass" onclick={closeClip}>
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M8.5 3 L4.5 7 L8.5 11" stroke="currentColor" stroke-width="1.3"/></svg>
            Back
          </button>
          <div class="ename">{clip.name}</div>
          <div class="emeta mono">{clip.width}×{clip.height}{#if cropActive && cropRect} · ✂ {cropRect.w}×{cropRect.h}{/if} · {fmtSize(clip.size_bytes)}</div>
        </header>

        <!-- bottom overlay dock -->
        <div class="dock" onpointerenter={() => (overDock = true)} onpointerleave={() => (overDock = false)} role="toolbar" tabindex="-1">
          <div
            class="timeline"
            bind:this={timelineEl}
            bind:clientWidth={timelineWidth}
            onpointerdown={onTrackDown}
            onpointermove={onTimelineMove}
            onpointerup={onTimelineUp}
            onpointerleave={clearHoverFrame}
            role="slider" tabindex="0" aria-label="Trim timeline"
            aria-valuemin="0" aria-valuemax={duration} aria-valuenow={currentTime} aria-valuetext={fmt(currentTime)}
          >
            <div class="track"></div>
            <div
              class="region"
              class:dragging={regionDrag}
              style="left:{pct(inPoint)}%; width:{pct(selLength)}%"
              onpointerdown={startRegionDrag}
              onpointermove={moveRegionDrag}
              onpointerup={endRegionDrag}
              role="presentation"
            ></div>
            <div class="playhead" style="transform:translateX({(duration > 0 ? (currentTime / duration) * timelineWidth : 0) - 1}px)"></div>
            <div class="handle in" class:active={activeHandle === "in"} style="left:{pct(inPoint)}%"
              onpointerdown={(e) => startHandle("in", e)} onpointermove={moveHandle} onpointerup={endHandle}
              onkeydown={(/** @type {KeyboardEvent} */ e) => onHandleKey("in", e)}
              role="slider" tabindex="0" aria-label="In point"
              aria-valuemin="0" aria-valuemax={duration} aria-valuenow={inPoint} aria-valuetext={fmt(inPoint)}></div>
            <div class="handle out" class:active={activeHandle === "out"} style="left:{pct(outPoint)}%"
              onpointerdown={(e) => startHandle("out", e)} onpointermove={moveHandle} onpointerup={endHandle}
              onkeydown={(/** @type {KeyboardEvent} */ e) => onHandleKey("out", e)}
              role="slider" tabindex="0" aria-label="Out point"
              aria-valuemin="0" aria-valuemax={duration} aria-valuenow={outPoint} aria-valuetext={fmt(outPoint)}></div>

            {#if hoverFrame && clipFilmstrip}
              <div class="hoverframe" style="left:{hoverFrame.x}px">
                <div class="hfimg" style="background-image:url({clipFilmstrip}); background-size:{FILM_COLS * 100}% 100%; background-position-x:{(hoverFrame.idx / (FILM_COLS - 1)) * 100}%"></div>
                <div class="hftime mono">{fmt(hoverFrame.time)}</div>
              </div>
            {/if}
          </div>

          {#if showWaveform && waveform}
            <button class="wavestrip" onpointerdown={onTrackDown} onpointermove={onTimelineMove} onpointerup={onTimelineUp} onpointerleave={clearHoverFrame} aria-label="Audio waveform scrubber">
              <svg class="wave" viewBox="0 0 {waveform.length} 100" preserveAspectRatio="none" aria-hidden="true">
                <path d={wavePath} />
              </svg>
            </button>
          {/if}

          {#if showFilmstrip && clipFilmstrip}
            <canvas class="filmstrip" bind:this={stripCanvas} onpointerdown={onTrackDown} onpointermove={onTimelineMove} onpointerup={onTimelineUp} onpointerleave={clearHoverFrame} role="slider" tabindex="0" aria-label="Filmstrip scrubber" aria-valuemin="0" aria-valuemax={duration} aria-valuenow={currentTime} aria-valuetext={fmt(currentTime)}></canvas>
          {/if}

          <!-- options bar: output mode, inline compress controls, output name -->
          <div class="optbar">
            <div class="obleft">
              <div class="seg">
                <button class="seg-btn" class:on={mode === "lossless"} onclick={() => setMode("lossless")} disabled={cropActive} title={cropActive ? "Cropping requires re-encoding — use Compress" : "Stream-copy trim (no re-encode)"}>Lossless</button>
                <button class="seg-btn" class:on={mode === "compress"} onclick={() => setMode("compress")}>Compress</button>
                <button class="seg-btn" class:on={mode === "gif"} onclick={() => setMode("gif")}>GIF</button>
                <button class="seg-btn" class:on={mode === "audio"} onclick={() => setMode("audio")}>Audio</button>
              </div>

              <Dropdown bind:value={speed} options={SPEED_OPTIONS} label="Speed" ariaLabel="Playback and export speed" />

              {#if mode === "gif"}
                <Dropdown bind:value={gifFormat} options={GIF_FMT_OPTIONS} label="Format" ariaLabel="GIF format" />
                <Dropdown bind:value={gifFps} options={GIF_FPS_OPTIONS} label="FPS" ariaLabel="GIF frames per second" />
                <Dropdown bind:value={gifWidth} options={GIF_WIDTH_OPTIONS} label="Width" ariaLabel="GIF width" custom={{ min: 64, max: 1920, unit: "px" }} />
              {:else if mode === "compress"}
                <div class="seg sub">
                  <button class="seg-btn" class:on={compressBy === "size"} onclick={() => (compressBy = "size")}>Size</button>
                  <button class="seg-btn" class:on={compressBy === "quality"} onclick={() => (compressBy = "quality")}>Quality</button>
                </div>
                {#if compressBy === "size"}
                  <Dropdown bind:value={targetMb} options={SIZE_OPTIONS} label="Target" ariaLabel="Target size" custom={{ min: 1, max: 500, unit: "MB" }} />
                {:else}
                  <Dropdown bind:value={quality} options={RESOLUTION_OPTIONS} label="Resolution" ariaLabel="Output resolution" />
                {/if}
              {:else if mode === "audio"}
                <Dropdown bind:value={audioFormat} options={AUDIO_FMT_OPTIONS} label="Format" ariaLabel="Audio format" />
              {/if}

              {#if mode === "lossless" || mode === "compress"}
                <div class="cropctl">
                  <button class="btn ghost sm glass toggle icon" class:on={cropMode} onclick={toggleCrop} aria-pressed={cropMode} title="Crop the frame (re-encodes via Compress)">
                    <svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M4.5 1 V11.5 H15 M1 4.5 H11.5 V15" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg>
                  </button>
                  {#if cropActive && cropRect}
                    <span class="cropdim mono">{cropRect.w}×{cropRect.h}</span>
                  {/if}
                </div>
              {/if}
            </div>

            <div class="obright">
              {#if mode === "lossless" || mode === "compress"}
                <label class="check" title="Include the audio track in the export">
                  <input type="checkbox" bind:checked={includeAudio} />
                  <span class="checkbox" aria-hidden="true">
                    <svg width="11" height="11" viewBox="0 0 12 12" fill="none"><path d="M2.5 6.2 L5 8.5 L9.5 3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
                  </span>
                  <span class="checktext">Audio</span>
                </label>
              {/if}
              {#if mode !== "gif" && mode !== "audio"}
                <label class="check" title="Move the source clip to the Recycle Bin after saving">
                  <input type="checkbox" bind:checked={deleteOriginal} />
                  <span class="checkbox" aria-hidden="true">
                    <svg width="11" height="11" viewBox="0 0 12 12" fill="none"><path d="M2.5 6.2 L5 8.5 L9.5 3.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
                  </span>
                  <span class="checktext">Delete original</span>
                </label>
              {/if}

              <label class="obname">
                <span class="oblabel">Save as</span>
                <div class="nameinput">
                  <input bind:value={outputName} placeholder={defaultStem} spellcheck="false" />
                  <span class="ext mono">.{outExt}</span>
                </div>
              </label>
            </div>
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
              <button class="btn ghost sm glass toggle" class:on={selectionOnly} onclick={toggleSelectionOnly} aria-pressed={selectionOnly} title={selectionOnly ? "Playing the selection only — click to play the whole clip" : "Playing the whole clip — click to play the selection only"}>
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M4.5 3 V13 M11.5 3 V13" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><path d="M4.5 8 H11.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></svg>
                Selection
              </button>
              <button class="btn ghost sm glass toggle" class:on={loopEnabled} onclick={() => (loopEnabled = !loopEnabled)} aria-pressed={loopEnabled} title="Loop playback">
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none"><path d="M4 5 H10.5 A2.5 2.5 0 0 1 13 7.5 A2.5 2.5 0 0 1 10.5 10 H3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><path d="M5.5 3 L3.3 5 L5.5 7" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/></svg>
                Loop
              </button>
              <!-- View toggles merged into one segmented bar (waveform / filmstrip / fullscreen). -->
              <div class="iconbar">
                <button class="btn ghost sm glass toggle icon" class:on={showWaveform} onclick={toggleWaveform} aria-pressed={showWaveform} title="Show audio waveform">
                  <svg width="15" height="15" viewBox="0 0 16 16" fill="currentColor"><rect x="2" y="6" width="1.5" height="4" rx="0.5"/><rect x="5" y="3" width="1.5" height="10" rx="0.5"/><rect x="8" y="5" width="1.5" height="6" rx="0.5"/><rect x="11" y="2.5" width="1.5" height="11" rx="0.5"/></svg>
                </button>
                <button class="btn ghost sm glass toggle icon" class:on={showFilmstrip} onclick={toggleFilmstrip} aria-pressed={showFilmstrip} title="Show filmstrip">
                  <svg width="15" height="15" viewBox="0 0 16 16" fill="none"><rect x="2" y="3.5" width="12" height="9" rx="1" stroke="currentColor" stroke-width="1.1"/><path d="M6 3.5 V12.5 M10 3.5 V12.5" stroke="currentColor" stroke-width="1.1"/></svg>
                </button>
                <button class="btn ghost sm glass toggle icon" class:on={isFullscreen} onclick={toggleFullscreen} aria-pressed={isFullscreen} title="Fullscreen (F11) — covers the taskbar; Esc to exit">
                  {#if isFullscreen}
                    <svg width="15" height="15" viewBox="0 0 16 16" fill="none"><path d="M6 2 V6 H2 M10 2 V6 H14 M6 14 V10 H2 M10 14 V10 H14" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg>
                  {:else}
                    <svg width="15" height="15" viewBox="0 0 16 16" fill="none"><path d="M2 6 V2 H6 M14 6 V2 H10 M2 10 V14 H6 M14 10 V14 H10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/></svg>
                  {/if}
                </button>
              </div>

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
                {#if currentFrame != null}<span class="frame" title="Current frame">f{currentFrame}</span>{/if}
                <span class="sep">·</span>
                <span class="lbl">IN</span> {fmt(inPoint)}
                <span class="lbl">OUT</span> {fmt(outPoint)}
                <span class="lbl">LEN</span> <span class="strong">{fmt(selLength)}</span>
              </div>
            </div>

            <div class="right">
              <button class="btn primary export" onclick={exportClip} disabled={busy || selLength <= 0}>
                {#if busy}<span class="spin"></span>{busyLabel}…{#if compressProgress !== null}<span class="blen mono">{Math.round(compressProgress * 100)}%</span>{/if}{:else}{modeLabel}<span class="blen mono">{fmt(selLength)}</span>{/if}
              </button>
            </div>
          </div>

          {#if busy && compressProgress !== null}
            <div class="progwrap" role="progressbar" aria-label="Compression progress" aria-valuenow={Math.round(compressProgress * 100)} aria-valuemin="0" aria-valuemax="100">
              <div class="progfill" style="width:{compressProgress * 100}%"></div>
            </div>
          {/if}
        </div>
      </section>
    {/if}
  </div>

  {#if dragOver}
    <div class="dropmask"><div class="dropcard">Drop to load</div></div>
  {/if}

  {#if toast}
    <div class="toast {toast.kind}">
      {#if toast.kind === "ok" && toast.restored}
        <span class="tdot ok"></span>
        <span>Restored <strong>{baseName(toast.trashedPath)}</strong> to its folder</span>
      {:else if toast.kind === "ok" && toast.deleted}
        <span class="tdot ok"></span>
        <span>Moved <strong>{baseName(toast.path)}</strong> to the Recycle Bin
          {#if toast.restoreError}<span class="trashwarn"> · couldn't restore: {toast.restoreError}</span>{/if}</span>
        {#if undoAvailable(toast)}<button class="link" onclick={undoDelete}>Undo</button>
        {:else if toast.undo === "restoring"}<span class="muted mono">Restoring…</span>{/if}
      {:else if toast.kind === "ok" && toast.copied}
        <span class="tdot ok"></span>
        <span>Copied <strong>{toast.name}</strong> to the clipboard</span>
      {:else if toast.kind === "ok"}
        <span class="tdot ok"></span>
        <span>Saved <strong>{baseName(toast.path)}</strong>
          <span class="mono muted"> · {fmtSize(toast.size_bytes)}{toast.encoder ? ` · ${toast.encoder}` : ""}</span>
          {#if toast.trashed}<span class="muted"> · original moved to Recycle Bin</span>{/if}
          {#if toast.trashError}<span class="trashwarn"> · couldn't remove original</span>{/if}
          {#if toast.restoreError}<span class="trashwarn"> · couldn't restore: {toast.restoreError}</span>{/if}</span>
        {#if undoAvailable(toast)}<button class="link" onclick={undoDelete}>Undo</button>
        {:else if toast.undo === "restoring"}<span class="muted mono">Restoring…</span>{/if}
        <button class="link" onclick={revealOutput}>Show in folder</button>
      {:else}
        <span class="tdot err"></span><span class="errmsg">{toast.msg}</span>
      {/if}
      <button class="link" onclick={() => (toast = null)}>Dismiss</button>
    </div>
  {/if}

  {#if cardMenu}
    <div class="ctxmenu" style="left:{cardMenu.x}px; top:{cardMenu.y}px" onpointerdown={(e) => e.stopPropagation()} role="menu" tabindex="-1">
      <button class="ctxitem" role="menuitem" onclick={() => { loadClip(/** @type {CardMenu} */ (cardMenu).clip.path); closeCardMenu(); }}>Open</button>
      <button class="ctxitem" role="menuitem" onclick={() => revealClip(/** @type {CardMenu} */ (cardMenu).clip)}>Reveal in folder</button>
      <button class="ctxitem" role="menuitem" onclick={() => copyClip(/** @type {CardMenu} */ (cardMenu).clip)}>Copy</button>
      <button class="ctxitem" role="menuitem" onclick={() => startRename(/** @type {CardMenu} */ (cardMenu).clip)}>Rename…</button>
      <div class="ctxsep"></div>
      <button class="ctxitem danger" role="menuitem" onclick={() => deleteClipFromLibrary(/** @type {CardMenu} */ (cardMenu).clip)}>Delete</button>
    </div>
  {/if}

  {#if showSettings}
    <div class="modalmask" onpointerdown={() => (showSettings = false)} role="presentation">
      <div class="modal settings" onpointerdown={(/** @type {PointerEvent} */ e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" use:trapFocus>
        <div class="settingshead">
          <div class="modaltitle">Settings</div>
          <button class="iconlink sm" onclick={() => (showSettings = false)} aria-label="Close settings">
            <svg width="13" height="13" viewBox="0 0 13 13" fill="none"><path d="M2 2 L11 11 M11 2 L2 11" stroke="currentColor" stroke-width="1.3"/></svg>
          </button>
        </div>

        <!-- Output location -->
        <div class="sgroup">
          <div class="slabel">Output location</div>
          <div class="shint">Where Trims and Compresses are written.</div>
          <div class="outloc">
            <span class="outpath mono" title={outputDir || "Next to the original clip"}>
              {outputDir || "Next to the original clip"}
            </span>
            <button class="btn ghost sm" onclick={chooseOutputDir}>Choose…</button>
            {#if outputDir}<button class="link" onclick={resetOutputDir}>Reset</button>{/if}
          </div>
        </div>

        <!-- Naming scheme -->
        <div class="sgroup">
          <div class="slabel">Default name</div>
          <div class="shint">Template for the suggested file name. Tokens:
            <code>{"{name}"}</code> <code>{"{action}"}</code>.</div>
          <input class="sinput mono" bind:value={namingScheme} placeholder="{'{name}_{action}'}" spellcheck="false" />
          <div class="spreview mono">Preview: <span>{schemePreview}</span></div>
        </div>

        <!-- Theme accent -->
        <div class="sgroup">
          <div class="slabel">Accent</div>
          <div class="swatches">
            {#each ACCENTS as a}
              <button
                class="swatch"
                class:on={accent.toLowerCase() === a.v}
                style="--sw:{a.v}"
                onclick={() => (accent = a.v)}
                aria-label={a.label}
                title={a.label}
              ></button>
            {/each}
          </div>
        </div>
      </div>
    </div>
  {/if}

  {#if renaming}
    <div class="modalmask" onpointerdown={() => (renaming = null)} role="presentation">
      <div class="modal" onpointerdown={(/** @type {PointerEvent} */ e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-label="Rename clip" tabindex="-1" use:trapFocus>
        <div class="modaltitle">Rename clip</div>
        <input
          class="renameinput mono"
          bind:value={renaming.name}
          use:focusOnMount
          spellcheck="false"
          onkeydown={(/** @type {KeyboardEvent} */ e) => { if (e.key === "Enter") commitRename(); else if (e.key === "Escape") (renaming = null); }}
        />
        <div class="modalrow">
          <button class="btn ghost sm" onclick={() => (renaming = null)}>Cancel</button>
          <button class="btn primary sm" onclick={commitRename}>Save</button>
        </div>
      </div>
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
    /* Corner-radius scale — tight + crisp. Dial the whole app's feel here. */
    --r-xs: 3px;
    --r-sm: 4px;
    --r-md: 6px;
    --r-lg: 8px;
    /* Compact spacing scale + a faint top-edge highlight for recessed controls. */
    --s-1: 4px;
    --s-2: 6px;
    --s-3: 8px;
    --s-4: 10px;
    --s-5: 14px;
    --inset-hi: inset 0 1px 0 rgba(255,255,255,0.045);
    color-scheme: dark;
  }
  :global(body) { margin: 0; font-family: var(--ui); -webkit-font-smoothing: antialiased; }
  :global(*) { box-sizing: border-box; }

  .app { height: 100vh; display: flex; flex-direction: column; background: var(--bg); color: var(--text); overflow: hidden; user-select: none; }

  /* titlebar chrome moved to $lib/Titlebar.svelte */

  .body { flex: 1; min-height: 0; display: flex; flex-direction: column; }

  /* ---------- buttons ---------- */
  .btn { font: inherit; cursor: pointer; display: inline-flex; align-items: center; gap: 7px; border-radius: var(--r-sm); border: 1px solid var(--border-2); background: var(--panel-2); color: var(--text); padding: 8px 14px; font-size: 13px; transition: background 0.15s, border-color 0.15s, transform 0.05s; }
  .btn:hover { background: var(--panel-3); border-color: #41414a; }
  .btn:active { transform: translateY(1px); }
  .btn.ghost { background: transparent; }
  .btn.ghost:hover { background: var(--panel-2); }
  .btn.sm { padding: 5px 10px; font-size: 12.5px; }
  .btn.glass { background: rgba(20,20,22,0.55); backdrop-filter: blur(10px); border-color: rgba(255,255,255,0.12); }
  .btn.glass:hover, .btn.glass.on { background: rgba(40,40,44,0.7); }
  .btn.primary { background: var(--accent); color: #0a0a0b; border-color: var(--accent); font-weight: 600; padding: 11px 20px; font-size: 14px; border-radius: var(--r-md); box-shadow: 0 8px 24px -10px rgba(255,255,255,0.4); }
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
  .srcbtn { display: inline-flex; align-items: center; gap: 8px; min-width: 0; max-width: 62%; font: inherit; font-size: 12.5px; color: var(--text); background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-sm); padding: 6px 11px; cursor: pointer; transition: background 0.14s, border-color 0.14s; }
  .srcbtn:hover { background: var(--panel-2); border-color: var(--border-2); }
  .srcbtn svg { flex: 0 0 auto; color: var(--faint); }
  .srcbtn:hover svg { color: var(--muted); }
  .srcpath { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11.5px; color: var(--muted); }
  .srcbtn:hover .srcpath { color: var(--text); }
  .iconlink { display: grid; place-items: center; width: 30px; height: 30px; flex: 0 0 auto; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--muted); cursor: pointer; transition: background 0.14s, color 0.14s; }
  .iconlink:hover { background: var(--panel-2); color: var(--text); }
  /* sort-direction toggle: the arrow flips for ascending */
  .sortdir svg { transition: transform 0.15s; }
  .sortdir.asc svg { transform: rotate(180deg); }

  /* ---------- filters ---------- */
  .filters { display: flex; flex-wrap: wrap; gap: 14px; justify-content: space-between; align-items: center; padding-bottom: 14px; margin-bottom: 18px; border-bottom: 1px solid var(--border); }
  .fgroup { display: flex; flex-wrap: wrap; gap: 9px; align-items: center; }
  .fcount { font-size: 11.5px; color: var(--faint); flex: 0 0 auto; }
  .search { display: inline-flex; align-items: center; gap: 7px; height: 30px; padding: 0 9px; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-sm); transition: border-color 0.14s; }
  .search:focus-within { border-color: var(--border-2); }
  .search svg { flex: 0 0 auto; color: var(--faint); }
  .search input { border: 0; background: transparent; color: var(--text); font: inherit; font-size: 12.5px; width: 150px; outline: none; padding: 0; }
  .search input::placeholder { color: var(--faint); }
  .searchclear { display: grid; place-items: center; width: 16px; height: 16px; flex: 0 0 auto; padding: 0; border: 0; border-radius: 3px; background: transparent; color: var(--faint); cursor: pointer; }
  .searchclear:hover { color: var(--text); background: var(--panel-2); }

  /* gridwrap reserves the full scroll height; the grid is taken out of flow and
     translated to the first rendered row, so only ~viewport cards are mounted. */
  .gridwrap { position: relative; width: 100%; }
  .gridwrap .grid { position: absolute; top: 0; left: 0; right: 0; will-change: transform; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 15px; }
  /* No entrance animation on real cards: windowing remounts them on scroll, so an
     entrance would replay on every recycle. (Skeletons keep their own pulse.) */
  .card { background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-md); padding: 0; overflow: hidden; text-align: left; color: var(--text); cursor: pointer; transition: transform 0.18s cubic-bezier(0.16,1,0.3,1), border-color 0.18s, background 0.18s; box-shadow: inset 0 1px 0 rgba(255,255,255,0.02); }
  .card:hover { transform: translateY(-3px); border-color: var(--border-2); background: var(--panel-2); }
  .card:active { transform: translateY(-1px) scale(0.99); }
  /* corrupted / unreadable clip — red border + warning tag */
  .card.bad { border-color: #b4232a; }
  .card.bad:hover { border-color: #d2353c; }
  .badtag { position: absolute; top: 7px; left: 7px; display: inline-flex; align-items: center; gap: 4px; padding: 3px 7px; font-size: 10.5px; font-weight: 600; color: #fff; background: rgba(180,35,42,0.92); border-radius: var(--r-xs); box-shadow: 0 2px 8px -2px rgba(0,0,0,0.6); z-index: 2; }
  .thumb { position: relative; height: 104px; display: grid; place-items: center; color: var(--faint); background: linear-gradient(150deg, #161619, #1e1e23); border-bottom: 1px solid var(--border); overflow: hidden; }
  .card:hover .thumb { color: var(--muted); }
  .thumb img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; display: block; animation: fade 0.3s ease; }
  /* hover-scrub frame from the filmstrip sprite, over the poster */
  .thumbscrub { position: absolute; inset: 0; background-repeat: no-repeat; background-color: #000; z-index: 1; animation: fade 0.2s ease; }
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
  /* auto-hide chrome while playing + idle (revealed on pointer movement) */
  .editor.uihidden { cursor: none; }
  .editor.uihidden .ehead, .editor.uihidden .dock { opacity: 0; pointer-events: none; }
  .stage { position: absolute; inset: 0; display: grid; place-items: center; padding: 8px; }
  video { display: block; max-width: 100%; max-height: 100%; border-radius: var(--r-sm); background: #000; }

  /* crop overlay — absolutely sized to the measured video box (videoBox). Only
     captures pointers while drawing so play/pause keeps working otherwise. */
  /* No z-index: DOM order keeps it above the <video> but below the header/dock
     (the video can extend behind those, and the overlay must not cover them). */
  .cropoverlay { position: absolute; pointer-events: none; overflow: hidden; border-radius: var(--r-sm); }
  .cropoverlay.drawing { pointer-events: auto; cursor: crosshair; touch-action: none; }
  .croprect { position: absolute; outline: 1.5px solid var(--accent); outline-offset: -1px; box-shadow: 0 0 0 1px rgba(0,0,0,0.7); }
  /* while drawing, dim everything outside the kept rectangle for clear feedback */
  .cropoverlay.drawing .croprect { box-shadow: 0 0 0 1px rgba(0,0,0,0.7), 0 0 0 9999px rgba(0,0,0,0.5); }
  /* resize handles — visual affordance only; hit-testing happens in JS on the
     overlay, so these never intercept pointers. Shown only while the tool is on. */
  .crophandle { position: absolute; width: 9px; height: 9px; background: var(--accent); border: 1px solid rgba(0,0,0,0.7); border-radius: 2px; transform: translate(-50%, -50%); pointer-events: none; display: none; }
  .cropoverlay.drawing .crophandle { display: block; }
  .crophandle.nw { left: 0; top: 0; }
  .crophandle.n  { left: 50%; top: 0; }
  .crophandle.ne { left: 100%; top: 0; }
  .crophandle.e  { left: 100%; top: 50%; }
  .crophandle.se { left: 100%; top: 100%; }
  .crophandle.s  { left: 50%; top: 100%; }
  .crophandle.sw { left: 0; top: 100%; }
  .crophandle.w  { left: 0; top: 50%; }
  .crophint { position: absolute; left: 50%; top: 12px; transform: translateX(-50%); padding: 5px 11px; font-size: 12px; color: var(--text); background: rgba(10,10,11,0.78); border: 1px solid var(--border-2); border-radius: var(--r-sm); pointer-events: none; }
  /* inline crop controls in the options bar */
  .cropctl { display: inline-flex; align-items: center; gap: 8px; }
  .cropdim { font-size: 12px; color: var(--muted); }

  .ehead { position: absolute; top: 0; left: 0; right: 0; display: flex; align-items: center; gap: 14px; padding: 10px 16px 22px; background: linear-gradient(to bottom, rgba(0,0,0,0.6), transparent); pointer-events: none; transition: opacity 0.35s ease; }
  .ehead > * { pointer-events: auto; }
  .ename { font-weight: 600; font-size: 13.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-shadow: 0 1px 3px rgba(0,0,0,0.6); }
  .emeta { font-size: 12px; color: var(--muted); margin-left: auto; flex: 0 0 auto; text-shadow: 0 1px 3px rgba(0,0,0,0.6); }

  .dock { position: absolute; left: 0; right: 0; bottom: 0; padding: 28px 16px 12px; background: linear-gradient(to top, rgba(8,8,9,0.92) 60%, rgba(8,8,9,0.5) 84%, transparent); transition: opacity 0.35s ease; }
  /* compression progress — sits under the transport, inset from the edges */
  .progwrap { height: 4px; margin: 8px 10px 2px; border-radius: 99px; background: rgba(255,255,255,0.08); overflow: hidden; }
  .progfill { height: 100%; border-radius: 99px; background: var(--accent); box-shadow: 0 0 10px -1px var(--accent); transition: width 0.18s linear; }
  .timeline { position: relative; height: 30px; margin-bottom: 6px; cursor: pointer; touch-action: none; }
  .track { position: absolute; top: 50%; left: 0; right: 0; height: 8px; transform: translateY(-50%); background: rgba(255,255,255,0.1); border: 1px solid rgba(255,255,255,0.12); border-radius: var(--r-xs); }
  /* optional audio waveform — its own strip beneath the Timeline (opt-in) */
  .wavestrip { position: relative; display: block; width: 100%; height: 26px; margin-bottom: 7px; padding: 0; border: 1px solid var(--border); border-radius: var(--r-xs); background: rgba(255,255,255,0.02); cursor: pointer; overflow: hidden; animation: fade 0.4s ease; }
  .wave { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
  .wave path { fill: rgba(255,255,255,0.32); }
  .region { position: absolute; top: 50%; height: 16px; transform: translateY(-50%); background: rgba(255,255,255,0.2); border-top: 1px solid rgba(255,255,255,0.6); border-bottom: 1px solid rgba(255,255,255,0.6); cursor: grab; touch-action: none; }
  .region:hover { background: rgba(255,255,255,0.26); }
  .region.dragging { cursor: grabbing; }
  /* GPU-composited: positioned via transform (not left%) so it stays smooth and
     doesn't flicker while sweeping during playback. */
  .playhead { position: absolute; top: 2px; bottom: 2px; left: 0; width: 2px; background: var(--text); pointer-events: none; border-radius: 2px; box-shadow: 0 0 6px rgba(0,0,0,0.6); will-change: transform; }
  .handle { position: absolute; top: 50%; width: 12px; height: 24px; transform: translate(-50%, -50%); background: var(--accent); border-radius: var(--r-xs); cursor: ew-resize; box-shadow: 0 0 0 1px #000, 0 4px 12px -4px rgba(0,0,0,0.8); touch-action: none; transition: box-shadow 0.15s; }
  .handle::after { content: ""; position: absolute; left: 50%; top: 50%; width: 2px; height: 12px; background: #0a0a0b40; transform: translate(-50%,-50%); border-radius: 2px; }
  .handle:hover, .handle.active { box-shadow: 0 0 0 1px #000, 0 0 0 4px rgba(255,255,255,0.18); }

  /* filmstrip strip beneath the Timeline — a <canvas> the script blits frames
     onto at the clip's aspect (cell count scales with width); height sets the
     cell aspect, so keep it ~16:9-friendly. */
  .filmstrip { display: block; width: 100%; height: 38px; margin-bottom: 7px; padding: 0; border: 1px solid var(--border); border-radius: var(--r-xs); background: #000; cursor: pointer; opacity: 0.78; transition: opacity 0.15s; animation: fade 0.4s ease; }
  .filmstrip:hover { opacity: 1; }
  /* hover preview frame floating above the Timeline */
  .hoverframe { position: absolute; bottom: calc(100% + 6px); transform: translateX(-50%); pointer-events: none; z-index: 22; display: flex; flex-direction: column; align-items: center; gap: 4px; }
  .hfimg { width: 132px; height: 74px; border: 1px solid var(--border-2); border-radius: var(--r-sm); background-color: #000; background-repeat: no-repeat; box-shadow: 0 10px 28px -10px rgba(0,0,0,0.85); }
  .hftime { font-size: 10.5px; color: var(--text); background: rgba(10,10,11,0.82); border: 1px solid var(--border); padding: 2px 6px; border-radius: var(--r-xs); }

  .dockrow { display: flex; align-items: center; justify-content: space-between; gap: 10px; flex-wrap: wrap; }
  .left { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .right { display: flex; align-items: center; gap: 9px; }
  .round { width: 32px; height: 32px; border-radius: 50%; border: 0; background: var(--accent); color: #0a0a0b; cursor: pointer; display: grid; place-items: center; transition: transform 0.05s, background 0.15s; flex: 0 0 auto; }
  .round:hover { background: #fff; }
  .round:active { transform: scale(0.94); }
  /* keyboard focus ring (e.g. after Space to pause): a dark gap then a thin
     halo following the circle, so it floats around the button instead of the
     fat UA outline that hugged the edges. focus-visible → never flashes on click. */
  .round:focus-visible { outline: none; box-shadow: 0 0 0 2px var(--bg), 0 0 0 4px rgba(255,255,255,0.45); }
  .readout { display: flex; align-items: baseline; gap: 7px; font-size: 12px; white-space: nowrap; overflow: hidden; }
  .readout .time { font-size: 12.5px; }
  .readout .sep { color: var(--faint); }
  .readout .frame { color: var(--faint); font-size: 11px; padding: 1px 5px; border: 1px solid var(--border); border-radius: var(--r-xs); }
  .readout .lbl { font-size: 9.5px; letter-spacing: 0.1em; color: var(--faint); }
  .readout .strong { color: var(--text); font-weight: 600; }

  /* segmented + pills */
  .seg { display: inline-flex; padding: 3px; background: rgba(10,10,11,0.6); backdrop-filter: blur(10px); border: 1px solid var(--border); border-radius: var(--r-md); gap: 3px; box-shadow: var(--inset-hi); }
  .seg.sub { background: var(--panel); backdrop-filter: none; }
  .seg-btn { border: 0; background: transparent; color: var(--muted); cursor: pointer; font: inherit; font-size: 12.5px; padding: 5px 12px; border-radius: var(--r-sm); transition: background 0.15s, color 0.15s; }
  .seg-btn:hover { color: var(--text); }
  .seg-btn.on { background: var(--panel-3); color: var(--text); box-shadow: inset 0 1px 0 rgba(255,255,255,0.05); }
  .seg-btn:focus-visible { outline: none; box-shadow: 0 0 0 2px var(--bg), 0 0 0 4px rgba(255,255,255,0.35); }
  .seg-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .seg-btn:disabled:hover { color: var(--muted); }

  /* options bar (between timeline and transport) */
  .optbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin-bottom: 8px; }
  .obleft { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; min-width: 0; }
  .obright { display: flex; align-items: center; gap: 12px; flex: 0 1 auto; min-width: 0; flex-wrap: wrap; justify-content: flex-end; }
  .obname { display: flex; align-items: center; gap: 7px; flex: 0 1 auto; min-width: 0; }
  .oblabel { font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); flex: 0 0 auto; }

  /* delete-original toggle */
  .check { display: inline-flex; align-items: center; gap: 8px; cursor: pointer; flex: 0 0 auto; user-select: none; }
  .check input { position: absolute; opacity: 0; width: 0; height: 0; }
  .checkbox { width: 17px; height: 17px; display: grid; place-items: center; border: 1px solid var(--border-2); border-radius: var(--r-sm); background: rgba(10,10,11,0.55); color: transparent; transition: background 0.14s, border-color 0.14s, color 0.14s; }
  .check:hover .checkbox { border-color: #4a4a52; }
  .check input:checked + .checkbox { background: var(--accent); border-color: var(--accent); color: #0a0a0b; }
  .check input:focus-visible + .checkbox { box-shadow: 0 0 0 3px rgba(255,255,255,0.18); }
  .checktext { font-size: 12.5px; color: var(--muted); white-space: nowrap; }
  .check:hover .checktext { color: var(--text); }

  .nameinput { display: flex; align-items: center; width: 168px; max-width: 100%; background: rgba(10,10,11,0.55); backdrop-filter: blur(10px); border: 1px solid var(--border-2); border-radius: var(--r-sm); padding-right: 8px; overflow: hidden; }
  .nameinput:focus-within { border-color: #4a4a52; }
  .nameinput input { flex: 1; min-width: 0; background: transparent; border: 0; color: var(--text); font: inherit; font-size: 12.5px; padding: 6px 9px; outline: none; }
  .ext { color: var(--faint); font-size: 12.5px; }

  /* volume (preview only) */
  .vol { display: flex; align-items: center; gap: 6px; flex: 0 0 auto; }
  .volbtn { width: 30px; height: 30px; display: grid; place-items: center; border: 0; border-radius: var(--r-sm); background: transparent; color: var(--muted); cursor: pointer; transition: background 0.14s, color 0.14s; }
  .volbtn:hover { background: rgba(255,255,255,0.08); color: var(--text); }
  .volslider { -webkit-appearance: none; appearance: none; width: 74px; height: 4px; border-radius: 3px; cursor: pointer; background: linear-gradient(to right, var(--text) var(--vfill), rgba(255,255,255,0.16) var(--vfill)); outline: none; }
  .volslider::-webkit-slider-thumb { -webkit-appearance: none; appearance: none; width: 12px; height: 12px; border-radius: 50%; background: var(--text); border: 0; box-shadow: 0 1px 4px rgba(0,0,0,0.6); cursor: pointer; }
  .volslider:focus-visible { box-shadow: 0 0 0 3px rgba(255,255,255,0.18); }

  /* shared toggle buttons (Loop, Waveform, Filmstrip). High specificity so the
     on-state beats `.btn.glass.on` and stays white instead of going dark. */
  .toggle { flex: 0 0 auto; }
  .toggle.icon { padding: 6px 9px; }
  .btn.glass.toggle.on { background: var(--accent); border-color: var(--accent); color: #0a0a0b; font-weight: 600; }
  .btn.glass.toggle.on:hover { background: #fff; border-color: #fff; }

  /* Merge adjacent icon toggles into one segmented rectangle: a single glass
     frame, internal hairline dividers, square inner cells. Rules sit after the
     .btn/.glass/.toggle rules above so they win the specificity ties. */
  .iconbar { display: inline-flex; align-items: center; flex: 0 0 auto; border: 1px solid rgba(255,255,255,0.12); border-radius: var(--r-sm); background: rgba(20,20,22,0.55); backdrop-filter: blur(10px); box-shadow: var(--inset-hi); overflow: hidden; }
  .iconbar .btn { border: 0; border-radius: 0; background: transparent; backdrop-filter: none; }
  .iconbar .btn + .btn { border-left: 1px solid rgba(255,255,255,0.10); }
  .iconbar .btn:hover { background: rgba(255,255,255,0.07); }
  .iconbar .btn.glass.toggle.on { background: var(--accent); color: #0a0a0b; }
  .iconbar .btn.glass.toggle.on:hover { background: #fff; }

  .export { flex: 0 0 auto; position: relative; z-index: 21; }
  .blen { font-size: 12px; opacity: 0.55; }
  .spin { width: 13px; height: 13px; border-radius: 50%; border: 2px solid rgba(10,10,11,0.25); border-top-color: #0a0a0b; animation: spin 0.7s linear infinite; display: inline-block; }

  /* ---------- overlays ---------- */
  .dropmask { position: fixed; inset: 0; display: grid; place-items: center; background: rgba(5,5,6,0.7); backdrop-filter: blur(2px); z-index: 50; }
  .dropcard { padding: 30px 56px; border: 1.5px dashed rgba(255,255,255,0.35); border-radius: var(--r-lg); font-size: 18px; font-weight: 600; background: rgba(20,20,22,0.6); }
  .toast { position: fixed; bottom: 20px; left: 50%; display: flex; align-items: center; gap: 13px; padding: 11px 15px; border-radius: var(--r-md); border: 1px solid var(--border-2); background: var(--panel-3); font-size: 13px; z-index: 60; box-shadow: 0 18px 50px -16px rgba(0,0,0,0.8); max-width: 70vw; animation: toastin 0.25s cubic-bezier(0.16,1,0.3,1); transform: translateX(-50%); }
  .tdot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
  .tdot.ok { background: #4ade80; }
  .tdot.err { background: #f87171; }
  .errmsg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 46vw; }
  .trashwarn { color: #f0a35e; }

  /* ---------- library card context menu ---------- */
  .ctxmenu { position: fixed; z-index: 70; min-width: 168px; padding: 5px; background: var(--panel-2); border: 1px solid var(--border-2); border-radius: var(--r-md); box-shadow: 0 18px 44px -16px rgba(0,0,0,0.85), inset 0 1px 0 rgba(255,255,255,0.04); animation: rise 0.12s cubic-bezier(0.16,1,0.3,1); }
  .ctxitem { width: 100%; display: block; text-align: left; font: inherit; font-size: 12.5px; color: var(--muted); background: transparent; border: 0; border-radius: var(--r-sm); padding: 7px 9px; cursor: pointer; white-space: nowrap; }
  .ctxitem:hover { background: var(--panel-3); color: var(--text); }
  .ctxitem.danger:hover { background: #b4232a; color: #fff; }
  .ctxsep { height: 1px; margin: 4px 6px; background: var(--border); }

  /* ---------- rename dialog ---------- */
  .modalmask { position: fixed; inset: 0; z-index: 75; display: grid; place-items: center; background: rgba(5,5,6,0.55); backdrop-filter: blur(2px); }
  .modal { width: 320px; max-width: 86vw; padding: 16px; background: var(--panel-2); border: 1px solid var(--border-2); border-radius: var(--r-lg); box-shadow: 0 24px 60px -20px rgba(0,0,0,0.85); }
  .modaltitle { font-size: 13px; font-weight: 600; margin-bottom: 11px; }
  .renameinput { width: 100%; background: var(--bg); border: 1px solid var(--border-2); color: var(--text); border-radius: var(--r-sm); padding: 9px 11px; font-size: 13px; outline: none; }
  .renameinput:focus { border-color: #4a4a52; }
  .modalrow { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }

  /* ---------- settings panel ---------- */
  .modal.settings { width: 440px; }
  .settingshead { display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px; }
  .settingshead .modaltitle { margin-bottom: 0; font-size: 14px; }
  .iconlink.sm { width: 24px; height: 24px; }
  .sgroup { padding: 14px 0; border-top: 1px solid var(--border); }
  .sgroup:first-of-type { border-top: 0; padding-top: 0; }
  .slabel { font-size: 12.5px; font-weight: 600; margin-bottom: 3px; }
  .shint { font-size: 11.5px; color: var(--muted); line-height: 1.5; margin-bottom: 9px; }
  .shint code { font-family: var(--mono); font-size: 11px; color: var(--text); background: var(--panel-3); border: 1px solid var(--border); border-radius: var(--r-xs); padding: 1px 4px; }
  .outloc { display: flex; align-items: center; gap: 9px; }
  .outpath { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11.5px; color: var(--muted); background: var(--bg); border: 1px solid var(--border); border-radius: var(--r-sm); padding: 7px 10px; }
  .sinput { width: 100%; background: var(--bg); border: 1px solid var(--border-2); color: var(--text); border-radius: var(--r-sm); padding: 8px 11px; font-size: 12.5px; outline: none; }
  .sinput:focus { border-color: #4a4a52; }
  .spreview { font-size: 11.5px; color: var(--faint); margin-top: 8px; }
  .spreview span { color: var(--muted); }
  .swatches { display: flex; gap: 9px; }
  .swatch { width: 26px; height: 26px; border-radius: 50%; background: var(--sw); border: 2px solid transparent; box-shadow: 0 0 0 1px var(--border-2); cursor: pointer; transition: transform 0.12s, box-shadow 0.12s; }
  .swatch:hover { transform: scale(1.1); }
  .swatch.on { box-shadow: 0 0 0 2px var(--bg), 0 0 0 4px var(--sw); }

  @keyframes rise { from { opacity: 0; transform: translateY(8px); } to { opacity: 1; transform: translateY(0); } }
  @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
  @keyframes pulse { 0%, 100% { opacity: 0.5; } 50% { opacity: 0.85; } }
  @keyframes spin { to { transform: rotate(360deg); } }
  @keyframes toastin { from { opacity: 0; transform: translateX(-50%) translateY(8px); } to { opacity: 1; transform: translateX(-50%) translateY(0); } }
</style>

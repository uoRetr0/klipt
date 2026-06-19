# Restrictive CSP + runtime-granted asset scope

Klipt shipped with `csp: null` (no Content-Security-Policy) and an
`assetProtocol.scope` of `["**"]` — the webview could load any URL and the asset
protocol could read **any file on disk**. For a desktop app that renders local
video and ffmpeg-generated images, that's far more reach than the UI needs.

## Decision

**CSP.** Set an explicit, restrictive policy (`default-src 'self'`) and open up
only what the UI actually uses:

- `img-src` / `media-src`: `'self'` + the asset protocol (`asset:`,
  `http(s)://asset.localhost`) so `convertFileSrc` thumbnails, filmstrips, and
  `<video>` clips load. `data:`/`blob:` for canvas/filmstrip paint.
- `style-src 'self' 'unsafe-inline'`: the app uses inline `style="…"` attributes
  (e.g. the GPU-composited playhead transform) and an inline `<style>` in
  `app.html`. `'unsafe-inline'` for styles is low-risk; scripts stay locked down.
- `font-src 'self'`: the Geist / Space Grotesk faces are self-hosted woff2.
- `script-src 'self'`: no inline scripts. Tauri appends its own nonces for the
  IPC bootstrap automatically.
- `connect-src` includes `ipc:`/`http://ipc.localhost` (Tauri IPC) and
  `ws://localhost:1420` (Vite HMR in `tauri dev`; inert in a packaged build).

**Asset scope.** The watched folder is chosen by the user at runtime and can sit
on any drive, so a static config glob can't both be tight *and* cover it. Instead
the static `assetProtocol.scope` is **empty**, and access is granted at runtime to
exactly Klipt's own directories via `asset_protocol_scope().allow_directory(…)`:

- the **app cache dir** (thumbnails / filmstrips / waveforms), granted at startup;
- the **watched folder** and **output dir**, granted at startup from saved
  settings and re-granted in `set_settings` whenever the user picks a new folder.

The result: the webview can read the clips library, the export dir, and Klipt's
cache — and nothing else — instead of the entire filesystem.

## Trade-off

Playback of a clip outside the watched/output folders (e.g. a path typed in by a
future feature) would need an explicit grant. That's the point: access is additive
and intentional rather than wide-open. If a future flow needs a new location, call
`allow_directory` for it rather than widening the static scope back to `**`.

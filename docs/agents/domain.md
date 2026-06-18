# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root (the glossary: Clip, Region, In-point/Out-point, Timeline, Trim, Watched folder).
- **`docs/adr/`** — read ADRs that touch the area you're about to work in:
  - `0001-tauri-svelte-stack.md` — Tauri + Svelte, lightweight Windows shell.
  - `0002-lossless-stream-copy.md` — Trim is `ffmpeg -c copy`; cuts snap to keyframes; frame-accurate "precise mode" is a deliberate future option, not the default.
  - `0003-bundled-ffmpeg-sidecar.md` — FFmpeg ships as a Tauri sidecar binary.

If any of these files don't exist, proceed silently.

## File structure

Single-context repo:

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-tauri-svelte-stack.md
│   ├── 0002-lossless-stream-copy.md
│   └── 0003-bundled-ffmpeg-sidecar.md
└── src/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids (e.g. use **Trim**, not export/render/save; use **Region**, not segment/selection; use **Clip**, not video/recording/footage).

If the concept you need isn't in the glossary yet, that's a signal — either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/grill-with-docs`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding.

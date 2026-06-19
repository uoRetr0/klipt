# Tauri + Svelte for a lightweight Windows desktop shell

The whole reason Klipt exists is that the alternatives are bloated (ClipChamp) or
ugly (LosslessCut). We chose **Tauri** (Rust shell + system WebView2) over Electron so
the shipped app is a few MB with low RAM, not a 100+ MB Chromium bundle — bloat is the
enemy we're defining ourselves against. For the frontend we chose **Svelte** over React
because it compiles to plain JS with no framework runtime, keeping the bundle and memory
footprint minimal while still giving full modern HTML/CSS control over the design.

Trade-off accepted: the dev toolchain is heavier (Rust + MSVC C++ Build Tools, a one-time
multi-GB install) than a Node-only stack. We took the heavier *build* setup to get the
lighter *shipped* product. Windows-only for now.

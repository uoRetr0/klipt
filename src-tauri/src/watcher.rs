//! Watched-folder change detection: a filesystem watcher over the library
//! roots (watched folder + output folder) that emits a debounced
//! `library-changed` event, so the grid refreshes itself the moment a new
//! recording or export lands — no manual refresh button pressing.

use std::path::Path;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager};

/// The live watcher (plus its shutdown flag), swapped out atomically when the
/// watched roots change. Held in Tauri managed state so the previous watcher
/// (and its debounce thread) is dropped/stopped when a new one replaces it.
#[derive(Default)]
pub(crate) struct WatchState(Mutex<Option<RecommendedWatcher>>);

/// How long to coalesce a burst of filesystem events into one refresh. An
/// export being written produces many modify events; one refresh at the end
/// (well, at most one per window) is all the grid needs.
const DEBOUNCE: Duration = Duration::from_millis(800);

/// Whether a filesystem event at `path` could change what the library shows —
/// i.e. it touches a file with a media extension. Directory-level events
/// (create/remove of a folder) have no extension and also return true, since a
/// per-game folder appearing usually brings clips with it.
fn affects_library(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        // No extension: likely a directory event — worth a rescan.
        None => true,
        Some(ext) => crate::library::is_media_ext(ext),
    }
}

/// (Re)start watching `folders` for media changes. Replaces any previous
/// watcher. Non-existent/duplicate folders are skipped; an empty list just
/// stops watching. Events are debounced off-thread and surface as a
/// `library-changed` event the frontend listens for.
#[tauri::command]
pub(crate) fn watch_library(app: AppHandle, folders: Vec<String>) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<()>();

    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if event.paths.iter().any(|p| affects_library(p)) {
                    let _ = tx.send(());
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Watch each distinct existing root. A nested output dir is covered by the
    // watched folder already, but double-watching only costs duplicate events —
    // the debounce collapses them.
    let mut watched_any = false;
    let mut seen: Vec<String> = Vec::new();
    for f in folders {
        let f = f.trim().to_string();
        if f.is_empty() || seen.contains(&f) || !Path::new(&f).is_dir() {
            continue;
        }
        if watcher
            .watch(Path::new(&f), RecursiveMode::Recursive)
            .is_ok()
        {
            watched_any = true;
        }
        seen.push(f);
    }

    // The debounce thread: absorb bursts, emit one event per quiet-ish window.
    // It ends naturally when the watcher (and thus `tx`) is dropped on replace.
    let emitter = app.clone();
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Swallow the rest of the burst before emitting.
            while rx.recv_timeout(DEBOUNCE).is_ok() {}
            let _ = emitter.emit("library-changed", ());
        }
    });

    // Swap in the new watcher; dropping the old one stops its callbacks and
    // lets its debounce thread exit once its channel disconnects.
    let state = app.state::<WatchState>();
    let mut slot = state.0.lock().map_err(|_| "watcher lock poisoned")?;
    *slot = if watched_any { Some(watcher) } else { None };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affects_library_matches_media_and_directories() {
        assert!(affects_library(Path::new("C:/clips/game/new.mp4")));
        assert!(affects_library(Path::new("C:/clips/out.gif")));
        assert!(affects_library(Path::new("C:/clips/out.m4a")));
        // Directory-shaped path (no extension) → rescan.
        assert!(affects_library(Path::new("C:/clips/NewGame")));
        // Non-media files never trigger a refresh.
        assert!(!affects_library(Path::new("C:/clips/notes.txt")));
        assert!(!affects_library(Path::new("C:/clips/thumb.jpg.tmp")));
    }
}

//! Render-cache eviction. The lazy caches (thumbnails / filmstrips / waveforms
//! / probes) are keyed by path + mtime: when a clip changes or disappears, its
//! old entries are simply never referenced again — nothing deletes them, so the
//! cache directory grows without bound. This sweep runs once per launch, off
//! the startup path, and evicts oldest-first once the total crosses a cap.

use std::path::{Path, PathBuf};

/// Start evicting once the caches exceed this many bytes...
const CAP_BYTES: u64 = 512 * 1024 * 1024;
/// ...and delete oldest-first until back under this (hysteresis, so the sweep
/// doesn't shave a few files on every single launch once near the cap).
const TARGET_BYTES: u64 = 384 * 1024 * 1024;

/// The cache subdirectories the lazy renderers write into.
const CACHE_SUBDIRS: [&str; 4] = ["thumbs", "filmstrips", "waveforms", "probes"];

/// Pick which files to evict: given `(mtime, size)` pairs, returns the indices
/// of the oldest files whose removal brings the total from `total` down to
/// `target`. Returns an empty list while `total <= cap`. Pure, so the policy is
/// unit-testable without a filesystem.
fn pick_evictions(files: &[(u64, u64)], total: u64, cap: u64, target: u64) -> Vec<usize> {
    if total <= cap {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by_key(|&i| files[i].0); // oldest first
    let mut remaining = total;
    let mut evict = Vec::new();
    for i in order {
        if remaining <= target {
            break;
        }
        remaining = remaining.saturating_sub(files[i].1);
        evict.push(i);
    }
    evict
}

/// Sweep the app cache once, in the background. Best-effort throughout: a
/// missing dir, unreadable metadata, or a failed delete never surfaces — the
/// worst case is the cache stays big until the next launch.
pub(crate) fn sweep_render_caches(cache_root: PathBuf) {
    tauri::async_runtime::spawn_blocking(move || {
        let mut files: Vec<(u64, u64)> = Vec::new();
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut total: u64 = 0;
        for sub in CACHE_SUBDIRS {
            collect_files(&cache_root.join(sub), &mut files, &mut paths, &mut total);
        }
        for i in pick_evictions(&files, total, CAP_BYTES, TARGET_BYTES) {
            let _ = std::fs::remove_file(&paths[i]);
        }
    });
}

fn collect_files(
    dir: &Path,
    files: &mut Vec<(u64, u64)>,
    paths: &mut Vec<PathBuf>,
    total: &mut u64,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        files.push((mtime, meta.len()));
        paths.push(entry.path());
        *total += meta.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_evictions_noop_under_cap() {
        let files = vec![(100, 50), (200, 50)];
        assert!(pick_evictions(&files, 100, 512, 384).is_empty());
        // Exactly at the cap is still fine.
        assert!(pick_evictions(&files, 512, 512, 384).is_empty());
    }

    #[test]
    fn pick_evictions_drops_oldest_first_until_target() {
        // Four files, 100 bytes each, total 400, cap 300, target 200:
        // must evict the two oldest (mtimes 10 and 20), keep 30 and 40.
        let files = vec![(30, 100), (10, 100), (40, 100), (20, 100)];
        let evict = pick_evictions(&files, 400, 300, 200);
        assert_eq!(evict, vec![1, 3], "oldest two by mtime");
    }

    #[test]
    fn pick_evictions_handles_everything_evicted() {
        let files = vec![(1, 500), (2, 500)];
        // total 1000, target 0 → both go, no panic, no infinite loop.
        assert_eq!(pick_evictions(&files, 1000, 100, 0).len(), 2);
        // Empty input with an (inconsistent) huge total: no indices to return.
        assert!(pick_evictions(&[], 1000, 100, 0).is_empty());
    }
}

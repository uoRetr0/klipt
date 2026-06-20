// The single frontend source of truth for which file extensions Klipt treats as
// Clips — drives both the Open-file dialog filter and the drag-drop accept test,
// so the two can't silently diverge. (The Rust scanner keeps its own VIDEO_EXTS
// in src-tauri/src/library.rs; keep the two lists in sync.)

/** Supported Clip container extensions (lower-case, no dot). */
export const VIDEO_EXTS = ["mp4", "mov", "mkv", "avi", "webm", "m4v"];

/**
 * True when `path` ends in a supported video extension (case-insensitive).
 * @param {string} path
 * @returns {boolean}
 */
export function isVideoFile(path) {
  const lower = (path || "").toLowerCase();
  return VIDEO_EXTS.some((e) => lower.endsWith("." + e));
}

// Media-kind classification for the library. The single frontend source of
// truth for which extensions Klipt lists and how each kind opens: videos load
// the trim editor, GIF/WebP ("anim") open the media viewer, audio loads the
// editor in audio-only mode. (The Rust scanner keeps the same lists in
// src-tauri/src/library.rs; keep the two in sync.)

import { VIDEO_EXTS } from "./video.js";

/** Animated-image outputs (Klipt's own GIF/WebP exports). */
export const ANIM_EXTS = ["gif", "webp"];
/** Audio-only files (Klipt's M4A/MP3 exports plus common formats). */
export const AUDIO_EXTS = ["m4a", "mp3", "wav", "ogg", "flac", "opus"];
/** Everything the library lists and the Open dialog / drag-drop accept. */
export const MEDIA_EXTS = [...VIDEO_EXTS, ...ANIM_EXTS, ...AUDIO_EXTS];

/** @param {string} path @returns {string} lower-cased extension, no dot */
function extOf(path) {
  const m = /\.([^./\\]+)$/.exec(path || "");
  return m ? m[1].toLowerCase() : "";
}

/**
 * Classify a path into its media kind, mirroring the backend's `media_kind`.
 * @param {string} path
 * @returns {"video" | "anim" | "audio" | null}
 */
export function mediaKind(path) {
  const e = extOf(path);
  if (VIDEO_EXTS.includes(e)) return "video";
  if (ANIM_EXTS.includes(e)) return "anim";
  if (AUDIO_EXTS.includes(e)) return "audio";
  return null;
}

/**
 * True when `path` is any media file Klipt can list/open.
 * @param {string} path
 */
export function isMediaFile(path) {
  return mediaKind(path) !== null;
}

/**
 * The short format tag shown on non-video cards ("GIF", "WEBP", "M4A", …).
 * Videos return null — the poster thumbnail already says "video".
 * @param {string} path
 * @returns {string | null}
 */
export function kindBadge(path) {
  const kind = mediaKind(path);
  if (kind === null || kind === "video") return null;
  return extOf(path).toUpperCase();
}

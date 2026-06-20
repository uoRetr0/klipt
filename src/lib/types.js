/**
 * Shared JSDoc types for the Tauri command boundary. Field names are snake_case
 * because the Rust structs serialize with serde defaults (no camelCase rename).
 *
 * @typedef {Object} ClipEntry  Library clip from `list_recent_clips`.
 * @property {string} path
 * @property {string} name
 * @property {string} game        Parent folder (ShadowPlay stores per-game).
 * @property {number} modified    mtime, seconds since epoch.
 * @property {number} size_bytes
 *
 * @typedef {Object} ClipInfo  Media metadata from `probe_clip`.
 * @property {number} duration
 * @property {number} width
 * @property {number} height
 * @property {number} fps
 * @property {number} size_bytes
 *
 * @typedef {Object} ThumbResult  Lazy card thumbnail from `clip_thumbnail`.
 * @property {string} path
 * @property {boolean} healthy
 * @property {number} duration  Clip seconds from the banner; 0 when unknown (cache hit).
 *
 * @typedef {Object} TrimResult  Result of a Trim/Compress/Gif export.
 * @property {string} path
 * @property {number} size_bytes
 * @property {string | null} encoder
 *
 * @typedef {Object} Settings  Persisted preferences from `get_settings`.
 * @property {string | null} watched_folder
 * @property {string | null} export_mode
 * @property {string | null} compress_by
 * @property {number | null} target_mb
 * @property {string | null} quality
 * @property {boolean | null} delete_original
 * @property {string | null} output_dir
 * @property {string | null} naming_scheme
 * @property {string | null} accent
 */
export {};

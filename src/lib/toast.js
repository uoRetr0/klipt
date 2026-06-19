// Pure state transitions for the "source Clip moved to the Recycle Bin" toast
// and its Undo. Trashing is reversible (see delete_clip / restore_clip), so the
// toast offers Undo until a restore succeeds. The `undo` field is the state
// machine: "available" → "restoring" → "done", with a failed restore falling
// back to "available" so the user can try again. Kept pure + DOM-free so the
// transitions and the undo-availability window are unit-testable.

/** Toast after a Trim/Compress whose source was trashed. `result` is the
 * TrimResult (path, size_bytes, encoder). `trashedPath` is the source we can
 * restore.
 * @param {import('./types').TrimResult} result
 * @param {string} trashedPath
 */
export function trashedToast(result, trashedPath) {
  return { kind: "ok", ...result, trashed: true, trashedPath, undo: "available" };
}

/** Toast after deleting a Clip straight from the library (no save).
 * @param {string} path
 */
export function deletedToast(path) {
  return { kind: "ok", deleted: true, path, trashedPath: path, undo: "available" };
}

/** A restore is in flight — Undo is temporarily unavailable.
 * @param {Record<string, any>} t  a toast object
 */
export function restoringToast(t) {
  return { ...t, undo: "restoring" };
}

/** The source was restored to its original location — Undo is consumed.
 * @param {Record<string, any>} t  a toast object
 */
export function restoredToast(t) {
  return { ...t, trashed: false, restored: true, undo: "done", restoreError: undefined };
}

/** A restore failed — surface the message and offer Undo again.
 * @param {Record<string, any>} t  a toast object
 * @param {unknown} message
 */
export function restoreFailedToast(t, message) {
  return { ...t, undo: "available", restoreError: String(message) };
}

/** Undo may be offered only while it is "available" (not in flight, not done).
 * @param {Record<string, any> | null | undefined} t  a toast object
 */
export function undoAvailable(t) {
  return !!t && t.undo === "available";
}

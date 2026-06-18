// Pure keyboard-binding resolver for the editor. The view builds a plain event
// object + a context snapshot and asks which Action (if any) a key press maps
// to. Keeping this DOM-free makes every binding unit-testable.
//
// Actions: "trim" | "back" | "playPause" | "setIn" | "setOut"
//          | "shuttleRewind" | "shuttlePause" | "shuttleForward"
//
// context: { hasClip: boolean, isTyping: boolean }

// Letter keys are matched case-insensitively (Shift just uppercases them).
const LETTER_ACTIONS = {
  i: "setIn",
  o: "setOut",
  j: "shuttleRewind",
  k: "shuttlePause",
  l: "shuttleForward",
};

/**
 * @param {{key: string, code?: string, ctrlKey?: boolean, metaKey?: boolean, altKey?: boolean, shiftKey?: boolean}} event
 * @param {{hasClip: boolean, isTyping: boolean}} context
 * @returns {string | null}
 */
export function resolve(event, context) {
  // Never hijack typing, and only act when a Clip is open in the editor.
  if (!context || context.isTyping || !context.hasClip) return null;
  // Leave OS / browser combos alone (Ctrl/Cmd/Alt). Shift is fine.
  if (event.ctrlKey || event.metaKey || event.altKey) return null;

  // Space is matched by physical code so it works regardless of key value.
  if (event.code === "Space") return "playPause";
  if (event.key === "Enter") return "trim";
  if (event.key === "Escape") return "back";

  const letter = typeof event.key === "string" ? event.key.toLowerCase() : "";
  return LETTER_ACTIONS[letter] ?? null;
}

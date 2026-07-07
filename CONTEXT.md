# Klipt

A lightweight, fast desktop tool (Windows) for trimming game clips (primarily
NVIDIA ShadowPlay recordings) down to a single short moment, without re-encoding. Built
to replace heavyweight editors (ClipChamp) and to be friendlier than LosslessCut.

## Language

**Clip**:
A source video recording opened for trimming — typically a multi-minute ShadowPlay
capture. The unedited input, never the result.
_Avoid_: Video, recording, footage (when precision matters)

**Region**:
The single contiguous range of a Clip the user wants to keep, defined by an in-point
and an out-point. Exactly one Region per Clip — there is no multi-segment selection.
_Avoid_: Segment, selection, range, cut

**In-point / Out-point**:
The start and end boundaries of the Region, set by dragging handles on the Timeline.
With lossless trimming these snap to the nearest keyframe.
_Avoid_: Mark in/out, start/end time

**Timeline**:
The scrubbable strip beneath the video showing the full Clip duration, the playhead,
and the draggable in/out handles bounding the Region.

**Trim**:
The act of producing an output file containing only the Region, by stream-copying
(no re-encode) from the Clip.
_Avoid_: Export, render, cut, save (as verbs for this action)

**Watched folder**:
The configurable directory (default: the ShadowPlay output location) the app monitors
to list recent Clips newest-first with thumbnails. A filesystem watcher refreshes the
library automatically when media lands in it (or in the output folder).

**Media kind**:
The classification the library gives every listed file — `video` (opens the trim
editor), `anim` (GIF/WebP, opens the Viewer), or `audio` (opens the editor in
audio-only mode). Exports of every kind show back up in the library.
_Avoid_: file type, format (when meaning this three-way split)

**Viewer**:
The overlay that opens for `anim` media — the webview animates the file natively —
with the card file actions (Copy / Reveal / Rename / Delete). Distinct from the
editor: nothing is trimmed in the Viewer.

## Flagged ambiguities

- **"Single region"** caused confusion during design: dragging one in-handle and one
  out-handle *is* one contiguous Region. "Multi-segment" (selecting several
  disconnected chunks from one Clip) was explicitly rejected for v1.

## Example dialogue

> **Dev:** When the user drags the out-point past a keyframe, does the Region update live?
> **User:** Yeah, the handle moves freely while dragging, but on Trim it snaps the
> in/out to keyframes so the copy stays lossless.
> **Dev:** And there's only ever one Region per Clip?
> **User:** Right. One funny moment per recording. If they want two, that's two Trims.

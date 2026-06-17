# Lossless stream-copy as the default trim

Klipt trims by stream-copying (`ffmpeg -c copy`) rather than re-encoding. This is
near-instant, loses zero quality, and keeps output tiny relative to duration — matching
the "fast and lightweight" goal for big 4K ShadowPlay clips.

The accepted cost: cuts can only land on **keyframes** (every ~1–5s), so in/out points
snap to the nearest keyframe instead of an exact frame. For trimming a clip down to "the
funny moment," ~1s of slack at the boundaries is fine. A frame-accurate re-encode
("precise mode") is a deliberate future option, not the default.

Audio: copy all tracks by default (ShadowPlay may record game + mic as separate streams);
a track selector is deferred until a real need appears.

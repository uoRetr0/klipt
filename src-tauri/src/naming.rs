//! Output-path resolution: sanitizing user-supplied names into bare file stems,
//! applying the naming-scheme template, and resolving collision-free paths for
//! exports and renames. All pure — the filesystem moves happen in the commands.

use std::path::{Path, PathBuf};

/// Drop characters illegal in a Windows filename (also strips path separators,
/// so a stem can never introduce a subdirectory).
fn strip_illegal(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect()
}

/// Sanitize a user-supplied output name into a bare file stem (no path, no ext).
fn clean_stem(requested: Option<&str>, default_stem: &str) -> String {
    let raw = requested.map(|s| s.trim()).unwrap_or("");
    if raw.is_empty() {
        return default_stem.to_string();
    }
    // Drop any directory components and strip a trailing extension the user typed.
    let base = Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(raw);
    let base = Path::new(base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(base);
    let cleaned = strip_illegal(base).trim().to_string();
    if cleaned.is_empty() {
        default_stem.to_string()
    } else {
        cleaned
    }
}

/// Build the default output stem from the user's naming scheme. Tokens: `{name}`
/// → the source Clip's stem, `{action}` → the output action ("trim", "small",
/// "gif", "webp"). Falls back to `{name}_{action}` when the scheme is blank, and
/// the result is sanitized so a template can never inject illegal path chars.
/// Pure — collision-resolution still happens in `resolve_output`.
fn apply_naming_scheme(scheme: Option<&str>, src_stem: &str, action: &str) -> String {
    let tmpl = scheme
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("{name}_{action}");
    let built = tmpl.replace("{name}", src_stem).replace("{action}", action);
    let cleaned = strip_illegal(&built).trim().to_string();
    if cleaned.is_empty() {
        format!("{src_stem}_{action}")
    } else {
        cleaned
    }
}

/// Resolve a collision-free output path next to the source.
fn resolve_output(parent: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut out = parent.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while out.exists() {
        out = parent.join(format!("{stem}_{n}.{ext}"));
        n += 1;
    }
    out
}

/// Validate the Region and resolve a collision-free output path. `action` names
/// the output ("trim", "small", "gif", …) and feeds the naming scheme that
/// builds the default stem when the user gave no name; `out_ext` is the output
/// extension. The output lands in `output_dir` when set (non-blank), otherwise
/// next to the source Clip. `naming_scheme` is the user's stem template.
// The negated comparison is deliberate: `!(end > start)` also rejects a NaN
// endpoint (NaN > x is false), whereas `end <= start` would let NaN through.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_output(
    path: &str,
    start: f64,
    end: f64,
    output_name: Option<&str>,
    action: &str,
    out_ext: &str,
    output_dir: Option<&str>,
    naming_scheme: Option<&str>,
) -> Result<String, String> {
    if !(end > start) {
        return Err("End point must be after the start point.".into());
    }
    let input = PathBuf::from(path);
    let src_stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Could not read the clip's name.")?;
    // Output folder: the override when set and non-blank, else next-to-source.
    let override_dir = output_dir
        .map(|d| d.trim())
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    let parent = match override_dir.as_deref() {
        Some(d) => d,
        None => input
            .parent()
            .ok_or("Could not resolve the clip's folder.")?,
    };
    let default_stem = apply_naming_scheme(naming_scheme, src_stem, action);
    let stem = clean_stem(output_name, &default_stem);
    let out_path = resolve_output(parent, &stem, out_ext);
    Ok(out_path.to_string_lossy().to_string())
}

/// Resolve where a Clip should move when renamed: same folder, same extension,
/// the user's name sanitized (illegal chars dropped), collision-free. Renaming
/// to the current name is a no-op (returns the source path unchanged). Pure —
/// the `rename_clip` command does the actual filesystem move.
pub(crate) fn rename_target(path: &str, new_name: &str) -> Result<String, String> {
    let input = PathBuf::from(path);
    let parent = input
        .parent()
        .ok_or("Could not resolve the clip's folder.")?;
    let ext = input.extension().and_then(|s| s.to_str()).unwrap_or("");
    let cleaned = clean_stem(Some(new_name), "");
    if cleaned.is_empty() {
        return Err("Please enter a valid name.".into());
    }
    let desired = if ext.is_empty() {
        parent.join(&cleaned)
    } else {
        parent.join(format!("{cleaned}.{ext}"))
    };
    if desired == input {
        return Ok(input.to_string_lossy().to_string());
    }
    Ok(resolve_output(parent, &cleaned, ext)
        .to_string_lossy()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // A fresh, collision-free temp dir per call so concurrent / repeated test
    // runs don't share (and clobber) a fixed-name directory.
    fn unique_temp_dir(label: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("klipt_test_{label}_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn clean_stem_falls_back_when_empty_or_blank() {
        assert_eq!(clean_stem(None, "def"), "def");
        assert_eq!(clean_stem(Some("   "), "def"), "def");
        assert_eq!(clean_stem(Some(""), "def"), "def");
    }

    #[test]
    fn clean_stem_strips_path_extension_and_illegal_chars() {
        // directory components dropped
        assert_eq!(clean_stem(Some("C:/evil/clip"), "def"), "clip");
        assert_eq!(clean_stem(Some("../../clip"), "def"), "clip");
        // trailing extension stripped
        assert_eq!(clean_stem(Some("clip.mp4"), "def"), "clip");
        // illegal filename chars removed
        assert_eq!(clean_stem(Some("a<b>c:d"), "def"), "abcd");
        // a name that is only illegal chars collapses to the default
        assert_eq!(clean_stem(Some("///"), "def"), "def");
    }

    #[test]
    fn resolve_output_uses_bare_name_when_free() {
        let dir = unique_temp_dir("resolve_free");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = resolve_output(&dir, "clip", "mp4");
        assert_eq!(out, dir.join("clip.mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_output_avoids_collisions() {
        let dir = unique_temp_dir("resolve_collide");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"x").unwrap();
        std::fs::write(dir.join("clip_2.mp4"), b"x").unwrap();
        let out = resolve_output(&dir, "clip", "mp4");
        assert_eq!(out, dir.join("clip_3.mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prepare_output_rejects_non_positive_region() {
        assert!(prepare_output("/tmp/a.mp4", 5.0, 5.0, None, "trim", "mp4", None, None).is_err());
        assert!(prepare_output("/tmp/a.mp4", 5.0, 4.0, None, "trim", "mp4", None, None).is_err());
    }

    #[test]
    fn prepare_output_builds_default_suffix_path() {
        let dir = unique_temp_dir("prepare");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        let out = prepare_output(
            &src.to_string_lossy(),
            0.0,
            2.0,
            None,
            "trim",
            "mp4",
            None,
            None,
        )
        .unwrap();
        assert!(out.ends_with("clip_trim.mp4"), "got {out}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prepare_output_honours_output_dir_override() {
        // Source lives in one folder; the override sends output to another.
        let src_dir = unique_temp_dir("outdir_src");
        let dst_dir = unique_temp_dir("outdir_dst");
        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&dst_dir);
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&dst_dir).unwrap();
        let src = src_dir.join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        let dst = dst_dir.to_string_lossy().to_string();
        let out = prepare_output(
            &src.to_string_lossy(),
            0.0,
            2.0,
            None,
            "trim",
            "mp4",
            Some(&dst),
            None,
        )
        .unwrap();
        assert_eq!(
            PathBuf::from(&out),
            dst_dir.join("clip_trim.mp4"),
            "got {out}"
        );
        // A blank override falls back to next-to-source.
        let out2 = prepare_output(
            &src.to_string_lossy(),
            0.0,
            2.0,
            None,
            "trim",
            "mp4",
            Some("   "),
            None,
        )
        .unwrap();
        assert_eq!(
            PathBuf::from(&out2),
            src_dir.join("clip_trim.mp4"),
            "got {out2}"
        );
        std::fs::remove_dir_all(&src_dir).unwrap();
        std::fs::remove_dir_all(&dst_dir).unwrap();
    }

    #[test]
    fn prepare_output_uses_naming_scheme_for_default_stem() {
        let dir = unique_temp_dir("scheme");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("raw.mp4");
        std::fs::write(&src, b"x").unwrap();
        // Scheme reorders tokens; an explicit name still wins over the scheme.
        let out = prepare_output(
            &src.to_string_lossy(),
            0.0,
            2.0,
            None,
            "small",
            "mp4",
            None,
            Some("{action}-{name}"),
        )
        .unwrap();
        assert!(out.ends_with("small-raw.mp4"), "got {out}");
        let named = prepare_output(
            &src.to_string_lossy(),
            0.0,
            2.0,
            Some("my clip"),
            "small",
            "mp4",
            None,
            Some("{action}-{name}"),
        )
        .unwrap();
        assert!(named.ends_with("my clip.mp4"), "got {named}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_naming_scheme_substitutes_tokens_and_defaults() {
        assert_eq!(apply_naming_scheme(None, "clip", "trim"), "clip_trim");
        assert_eq!(
            apply_naming_scheme(Some("   "), "clip", "small"),
            "clip_small"
        );
        assert_eq!(
            apply_naming_scheme(Some("{name}-{action}-clip"), "raw", "gif"),
            "raw-gif-clip"
        );
        // A scheme can drop a token entirely.
        assert_eq!(apply_naming_scheme(Some("{name}"), "raw", "trim"), "raw");
    }

    #[test]
    fn apply_naming_scheme_strips_illegal_chars_and_separators() {
        // Path separators and illegal filename chars are stripped so a scheme
        // can never write outside the target folder or produce an invalid name.
        assert_eq!(
            apply_naming_scheme(Some("../{name}/{action}"), "raw", "trim"),
            "..rawtrim"
        );
        assert_eq!(
            apply_naming_scheme(Some("a:b*c{name}"), "raw", "trim"),
            "abcraw"
        );
        // A scheme that collapses to nothing falls back to the default.
        assert_eq!(apply_naming_scheme(Some("///"), "raw", "trim"), "raw_trim");
    }

    #[test]
    fn rename_target_keeps_extension_and_strips_illegal_chars() {
        // Parent need not exist for the no-collision path.
        let out = rename_target("C:/clips/raw.mkv", "My Clip").unwrap();
        assert!(out.ends_with("My Clip.mkv"), "got {out}");
        let out2 = rename_target("C:/clips/raw.mp4", "a<b>c:d").unwrap();
        assert!(out2.ends_with("abcd.mp4"), "got {out2}");
    }

    #[test]
    fn rename_target_rejects_blank_names() {
        assert!(rename_target("C:/clips/raw.mp4", "   ").is_err());
        assert!(rename_target("C:/clips/raw.mp4", "///").is_err());
    }

    #[test]
    fn rename_target_is_noop_when_name_is_unchanged() {
        let dir = unique_temp_dir("rename_noop");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        // Renaming to the same stem returns the same path, not "clip_2.mp4".
        let out = rename_target(&src.to_string_lossy(), "clip").unwrap();
        assert_eq!(out, src.to_string_lossy());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_target_avoids_collisions_with_other_files() {
        let dir = unique_temp_dir("rename_collide");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"x").unwrap();
        std::fs::write(dir.join("taken.mp4"), b"x").unwrap();
        // Renaming clip.mp4 to "taken" must not clobber the existing taken.mp4.
        let out = rename_target(&dir.join("clip.mp4").to_string_lossy(), "taken").unwrap();
        assert!(out.ends_with("taken_2.mp4"), "got {out}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

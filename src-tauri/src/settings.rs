//! Persisted user settings (watched folder + export/output preferences) and the
//! commands that read/write them. Loading is always best-effort and forward-
//! compatible: every preference is `Option` + `#[serde(default)]`, so an older
//! settings.json still deserialises.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) watched_folder: Option<String>,
    // Persisted export preferences (None until the user has saved one). All
    // Option + `#[serde(default)]` so older settings.json files still load.
    pub(crate) export_mode: Option<String>, // "lossless" | "compress"
    pub(crate) compress_by: Option<String>, // "size" | "quality"
    pub(crate) target_mb: Option<u32>,
    pub(crate) quality: Option<String>, // "low" | "medium" | "high"
    pub(crate) delete_original: Option<bool>,
    // Output preferences. `output_dir`: where Trims/Compresses are written when
    // set (defaults to next-to-source when None/blank). `naming_scheme`: a
    // template for the default output stem ({name}, {action} tokens). `accent`:
    // the theme accent colour token (hex), applied by the frontend.
    pub(crate) output_dir: Option<String>,
    pub(crate) naming_scheme: Option<String>,
    pub(crate) accent: Option<String>,
}

/// Resolve the configured output-location override, creating the folder if set.
/// Returns the directory to write into (for `prepare_output`), or None to keep
/// the default next-to-source behaviour.
pub(crate) fn ensure_output_dir(settings: &Settings) -> Result<Option<String>, String> {
    match settings
        .output_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        Some(d) => {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            Ok(Some(d.to_string()))
        }
        None => Ok(None),
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("settings.json"))
}

/// Load persisted settings, best-effort. Returns defaults (with the OS video
/// dir as the watched folder) when there is no settings file, and never errors —
/// commands that need the output prefs call this without failing the whole save.
pub(crate) fn read_settings(app: &AppHandle) -> Settings {
    if let Ok(p) = settings_path(app) {
        if p.exists() {
            if let Ok(raw) = std::fs::read_to_string(&p) {
                if let Ok(s) = serde_json::from_str::<Settings>(&raw) {
                    return s;
                }
            }
        }
    }
    let watched_folder = app
        .path()
        .video_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    Settings {
        watched_folder,
        ..Default::default()
    }
}

#[tauri::command]
pub(crate) fn get_settings(app: AppHandle) -> Result<Settings, String> {
    Ok(read_settings(&app))
}

#[tauri::command]
pub(crate) fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let p = settings_path(&app)?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&p, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trips_export_preferences() {
        let s = Settings {
            watched_folder: Some("C:/clips".into()),
            export_mode: Some("compress".into()),
            compress_by: Some("size".into()),
            target_mb: Some(25),
            quality: Some("high".into()),
            delete_original: Some(true),
            output_dir: Some("D:/exports".into()),
            naming_scheme: Some("{name}_{action}".into()),
            accent: Some("#fafafa".into()),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn settings_loads_legacy_file_with_only_watched_folder() {
        // A settings.json written before export prefs existed must still load,
        // with the new fields defaulting to None rather than erroring.
        let json = r#"{"watched_folder":"C:/clips"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.watched_folder, Some("C:/clips".to_string()));
        assert_eq!(s.export_mode, None);
        assert_eq!(s.compress_by, None);
        assert_eq!(s.target_mb, None);
        assert_eq!(s.quality, None);
        assert_eq!(s.delete_original, None);
        assert_eq!(s.output_dir, None);
        assert_eq!(s.naming_scheme, None);
        assert_eq!(s.accent, None);
    }
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::align::AiModel;
use crate::quran::data_dir;

/// User settings persisted to `<data_dir>/settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where exports (JSON, compressed/split/cleaned audio) are written.
    pub output_dir: PathBuf,
    /// Show a save dialog (starting in `output_dir`) instead of saving directly.
    pub ask_each_time: bool,
    /// DeepFilterNet attenuation limit in dB (100 = no limit).
    pub denoise_atten_db: f32,
    /// Load the cleaned file into the app once clean-up finishes.
    pub load_cleaned: bool,
    /// Folder of per-surah audio files (001.mp3 … 114.mp3) for the "folder" source.
    pub audio_folder: Option<PathBuf>,
    /// Model precision used for AI timing.
    pub ai_model: AiModel,
    /// Show UI numbers as 123 instead of ١٢٣.
    pub western_digits: bool,
    pub dark: bool,
}

pub fn default_output_dir() -> PathBuf {
    dirs::audio_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Ayah Timer")
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            ask_each_time: false,
            denoise_atten_db: 100.0,
            load_cleaned: true,
            audio_folder: None,
            ai_model: AiModel::default(),
            western_digits: false,
            dark: false,
        }
    }
}

fn path() -> PathBuf {
    data_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, json);
        }
    }
}

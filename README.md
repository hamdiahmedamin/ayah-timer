# Ayah Timer · مؤقِّت الآيات

A native pure-Rust desktop application to time the verses of a whole-surah Quran
recitation and export the result as JSON in [mp3quran.net](https://www.mp3quran.net)'s
`ayat_timing` format.

## Features

- **Load any recitation** from mp3quran (with quick Riwayah filter for **Hafs**,
  **Warsh**, and **Qalun**), from a local audio file (MP3, WAV, FLAC, M4A, OGG),
  or from a URL.
- **Multi-hour streaming (`symphonia` + `rodio`):** Decodes multi-hour recordings
  (such as Al-Baqarah) in constant low RAM.
- **Native multi-threaded AI forced alignment (`ort`):** Runs `wav2vec2-base-arabic`
  (`model_q4.onnx`) via ONNX Runtime + 2-bit packed banded CTC Viterbi + pause-onset
  refinement (`🤖 توقيت AI`).
- **Authentic Mushaf texts & fonts:** Hafs (`alquran.cloud`), plus 6,214-ayah Warsh &
  Qalun (`thetruetruth/quran-data-kfgqpc` v10 + `KFGQPC Warsh` / `KFGQPC Qaloon`
  fonts) with both a live **Verse Stage** and full-surah **Mushaf Reader**.
- **Interactive waveform & table:** Click to seek, drag verse markers, nudge ±100 ms,
  play individual verses, and import/export `mp3quran` JSON.

## Build & Run

```bash
cargo run --release
```

## Shortcuts

- <kbd>Space</kbd> — Mark next verse start (with reaction-delay & pause snap)
- <kbd>P</kbd> — Play / Pause
- <kbd>←</kbd> / <kbd>→</kbd> — Seek ±5 s
- <kbd>Z</kbd> — Undo last mark

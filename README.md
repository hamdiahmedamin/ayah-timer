# Ayah Timer · مؤقِّت الآيات

A single-page tool to time the verses of a whole-surah Quran recitation and export
the result as JSON in [mp3quran.net](https://www.mp3quran.net)'s `ayat_timing` format.

**Live:** https://hamdiahmedamin.github.io/ayah-timer/

## What it does

- **Load a recitation** from mp3quran (any reciter and riwayah, picked from its public
  API), from a file on your device, or from a URL.
- **Mark verse starts while listening**: press <kbd>Space</kbd> at the start of each verse.
  A reaction-delay correction and "snap to the nearest pause" place the mark where the
  voice actually starts.
- **Automatic draft**: finds the pauses in the recording and picks, among them, the set
  whose verses best fit their expected lengths (from the Hafs text). Verses you have
  already marked stay where they are and the draft fills the rest between them, which
  keeps long surahs on track.
- **Edit**: drag marks on the waveform, nudge ±100 ms, type exact times, play any verse.
- **Import / export**: mp3quran JSON in and out; fetch mp3quran's own timings for a
  recitation that has them, to check or correct them. Work is saved in the browser.
- Verse counts for the Kufan count (Hafs, Shu'ba) and the Madani count (Warsh, Qalun).

## Output

```json
[
 { "ayah": 0, "polygon": null, "start_time": 0,    "end_time": 5860,  "x": null, "y": null, "page": null },
 { "ayah": 1, "polygon": null, "start_time": 5860, "end_time": 16480, "x": null, "y": null, "page": null }
]
```

Times are in milliseconds. Ayah 0 is everything before verse 1 (the isti'adha and the
basmala). Untick "حقول mp3quran الكاملة" for `{ayah, start_time, end_time}` only.

## Accuracy of the draft

Checked against mp3quran's published timings for Mishary Alafasy (Hafs):
Yasin 80/83 verses within 0.5 s, al-Mulk 28/30 within 1 s. On al-Kahf the draft alone
goes astray (many pauses inside verses); with 6 verses marked by hand the rest land
with a median error of 0.46 s. Always review the draft by ear.

## Built with

One `index.html`, no build step, custom elements (`<ayah-timer>`, `<wave-view>`,
`<ayah-table>`, `<verse-stage>`, `<progress-meter>`, `<mushaf-view>`, `<smart-select>`,
`<file-drop>`). Audio is decoded in the browser with the Web Audio API (with chunked
16 kHz decoding for multi-hour MP3s); nothing is uploaded.

In-browser AI alignment (`🤖 توقيت AI`) runs `wav2vec2-base-arabic` (`model_q4.onnx`,
~86 MB, cached in browser Cache Storage) via `onnxruntime-web` inside a Web Worker.

Data: recitations and timings from mp3quran.net's public API; the Hafs text shown while
listening from alquran.cloud.

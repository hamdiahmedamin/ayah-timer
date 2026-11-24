use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::align::AiProgress;
use crate::timing::TimelineRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Mp3,
    Opus,
    M4a,
    Copy,
}

impl AudioFormat {
    pub fn ext(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Opus => "opus",
            AudioFormat::M4a => "m4a",
            AudioFormat::Copy => "mp3",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "MP3",
            AudioFormat::Opus => "Opus",
            AudioFormat::M4a => "M4A",
            AudioFormat::Copy => "نسخ سريع",
        }
    }

    pub fn codec_args(self, bitrate_kbps: u32, mono: bool, allow_copy: bool) -> Vec<String> {
        if self == AudioFormat::Copy && allow_copy {
            return vec!["-c:a".into(), "copy".into()];
        }
        let eff = if self == AudioFormat::Copy {
            AudioFormat::Mp3
        } else {
            self
        };
        let mut args = match eff {
            AudioFormat::Mp3 => vec!["-c:a".into(), "libmp3lame".into()],
            AudioFormat::Opus => vec!["-c:a".into(), "libopus".into()],
            AudioFormat::M4a => vec!["-c:a".into(), "aac".into()],
            AudioFormat::Copy => unreachable!(),
        };
        args.push("-b:a".into());
        args.push(format!("{bitrate_kbps}k"));
        if mono {
            args.push("-ac".into());
            args.push("1".into());
        }
        args
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingMode {
    SurahAyah,
    AyahOnly,
}

impl NamingMode {
    pub fn format_name(self, surah: usize, ayah: usize, ext: &str) -> String {
        match self {
            NamingMode::SurahAyah => format!("{surah:03}{ayah:03}.{ext}"),
            NamingMode::AyahOnly => format!("{ayah:03}.{ext}"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AudioExportOpts {
    pub format: AudioFormat,
    pub bitrate_kbps: u32,
    pub mono: bool,
    pub include_intro: bool,
    pub naming: NamingMode,
}

impl Default for AudioExportOpts {
    fn default() -> Self {
        Self {
            format: AudioFormat::Mp3,
            bitrate_kbps: 48,
            mono: true,
            include_intro: false,
            naming: NamingMode::SurahAyah,
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn run_ffmpeg_cancellable(args: &[String], cancel: &AtomicBool) -> Result<()> {
    let mut child = Command::new("ffmpeg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("تعذّر تشغيل ffmpeg (تأكد من تثبيته)")?;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("aborted"));
        }
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let out = child.wait_with_output()?;
            let err = String::from_utf8_lossy(&out.stderr);
            let tail = err.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
            return Err(anyhow!("ffmpeg فشل: {tail}"));
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

pub fn compress_audio(
    input: &Path,
    output: &Path,
    opts: &AudioExportOpts,
    cancel: &AtomicBool,
    on_progress: &impl Fn(AiProgress),
) -> Result<(u64, u64)> {
    on_progress(AiProgress {
        text: "جارٍ ضغط الملف الصوتي…".into(),
        pct: Some(25),
        done: false,
        err: None,
    });

    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-vn".to_string(),
    ];
    args.extend(opts.format.codec_args(opts.bitrate_kbps, opts.mono, false));
    args.push(output.display().to_string());

    run_ffmpeg_cancellable(&args, cancel)?;

    let src_size = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    let dst_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    Ok((src_size, dst_size))
}

pub fn split_audio_by_ayahs(
    input: &Path,
    out_dir: &Path,
    surah: usize,
    rows: &[TimelineRow],
    opts: &AudioExportOpts,
    cancel: &AtomicBool,
    on_progress: &impl Fn(AiProgress),
) -> Result<(usize, u64)> {
    let items: Vec<&TimelineRow> = rows
        .iter()
        .filter(|r| match (r.start, r.end) {
            (Some(s), Some(e)) => e > s && (r.ayah > 0 || opts.include_intro),
            _ => false,
        })
        .collect();

    if items.is_empty() {
        return Err(anyhow!("لا توجد آيات معلَّمة لتقسيمها."));
    }

    std::fs::create_dir_all(out_dir)?;
    let ext = if opts.format == AudioFormat::Copy {
        input
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.is_empty())
            .unwrap_or("mp3")
    } else {
        opts.format.ext()
    };

    let total = items.len();
    let mut written = 0usize;
    let mut total_bytes = 0u64;

    for (i, row) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("aborted"));
        }
        let start_ms = row.start.unwrap();
        let end_ms = row.end.unwrap();
        let dur_ms = end_ms.saturating_sub(start_ms);
        let pct = ((i * 100) / total).min(99) as u8;
        on_progress(AiProgress {
            text: format!("تقسيم الآية {} ({} من {total})…", row.ayah, i + 1),
            pct: Some(pct),
            done: false,
            err: None,
        });

        let dest = out_dir.join(opts.naming.format_name(surah, row.ayah, ext));
        let mut args = vec![
            "-y".to_string(),
            "-ss".to_string(),
            format!("{:.3}", start_ms as f64 / 1000.0),
            "-i".to_string(),
            input.display().to_string(),
            "-t".to_string(),
            format!("{:.3}", dur_ms as f64 / 1000.0),
            "-vn".to_string(),
        ];
        args.extend(opts.format.codec_args(opts.bitrate_kbps, opts.mono, true));
        args.push(dest.display().to_string());

        run_ffmpeg_cancellable(&args, cancel)?;
        written += 1;
        total_bytes += std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    }

    Ok((written, total_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_wav(path: &Path, secs: f32) {
        let sr = 16000u32;
        let n = (sr as f32 * secs) as usize;
        let data_bytes = (n * 2) as u32;
        let mut buf = Vec::with_capacity(44 + data_bytes as usize);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        buf.extend_from_slice(b"WAVEfmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&sr.to_le_bytes());
        buf.extend_from_slice(&(sr * 2).to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..n {
            let s = ((i as f32 * 440.0 * std::f32::consts::TAU / sr as f32).sin() * 8000.0) as i16;
            buf.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, buf).unwrap();
    }

    #[test]
    fn split_and_compress_work_with_ffmpeg() {
        let base = std::env::temp_dir().join("ayah_timer_export_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let wav = base.join("in.wav");
        write_test_wav(&wav, 1.0);

        let cancel = AtomicBool::new(false);
        let comp_out = base.join("out.opus");
        let comp_opts = AudioExportOpts {
            format: AudioFormat::Opus,
            bitrate_kbps: 32,
            mono: true,
            include_intro: false,
            naming: NamingMode::SurahAyah,
        };
        let (src_b, dst_b) = compress_audio(&wav, &comp_out, &comp_opts, &cancel, &|_| {}).unwrap();
        assert!(src_b > 0 && dst_b > 0 && comp_out.exists());

        let split_dir = base.join("split");
        let rows = vec![
            TimelineRow { ayah: 1, start: Some(0), end: Some(500), page: None },
            TimelineRow { ayah: 2, start: Some(500), end: Some(1000), page: None },
        ];
        let split_opts = AudioExportOpts {
            format: AudioFormat::Mp3,
            bitrate_kbps: 48,
            mono: true,
            include_intro: false,
            naming: NamingMode::SurahAyah,
        };
        let (cnt, tot) = split_audio_by_ayahs(&wav, &split_dir, 1, &rows, &split_opts, &cancel, &|_| {}).unwrap();
        assert_eq!(cnt, 2);
        assert!(tot > 0);
        assert!(split_dir.join("001001.mp3").exists());
        assert!(split_dir.join("001002.mp3").exists());

        let _ = std::fs::remove_dir_all(&base);
    }
}

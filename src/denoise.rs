//! Speech clean-up with DeepFilterNet3 (embedded model, pure Rust via tract).
//! ffmpeg decodes any input to 48 kHz mono f32 and encodes the result, the
//! model runs in between frame by frame, so RAM stays flat for long files.

use anyhow::{anyhow, Context, Result};
use deep_filter_rt::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{Array2, ArrayView2};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::align::AiProgress;
use crate::export_audio::AudioExportOpts;

/// Encoder args for a full clean-up export (always mono).
pub fn codec_for(opts: &AudioExportOpts) -> Vec<String> {
    opts.format.codec_args(opts.bitrate_kbps, true, false)
}

/// Uncompressed output for previews.
pub fn codec_wav() -> Vec<String> {
    vec!["-c:a".into(), "pcm_s16le".into()]
}

const SR: u64 = 48_000;

fn spawn_ffmpeg(args: &[String], stdin: Stdio, stdout: Stdio) -> Result<Child> {
    Command::new("ffmpeg")
        .args(args)
        .stdin(stdin)
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .context("تعذّر تشغيل ffmpeg (تأكد من تثبيته)")
}

fn ffmpeg_err(child: Child) -> anyhow::Error {
    let out = child.wait_with_output().ok();
    let err = out.map(|o| String::from_utf8_lossy(&o.stderr).trim().to_string()).unwrap_or_default();
    anyhow!("ffmpeg فشل: {}", err.lines().last().unwrap_or("?"))
}

/// Denoise `input` into `output` (mono, encoded with `codec_args`; see [`codec_for`]).
/// `span` = (start_ms, len_ms) limits it to a slice (used for previews).
/// The model's latency is compensated so ayah timings stay aligned with the original.
/// Returns the output size in bytes.
#[allow(clippy::too_many_arguments)]
pub fn denoise_file(
    input: &Path,
    output: &Path,
    codec_args: Vec<String>,
    atten_lim_db: f32,
    span: Option<(u64, u64)>,
    duration_ms: u64,
    cancel: &AtomicBool,
    on_progress: &impl Fn(AiProgress),
) -> Result<u64> {
    let report = |pct: u8| {
        on_progress(AiProgress {
            text: format!("جارٍ تنظيف الصوت (DeepFilterNet)… {pct}%"),
            pct: Some(pct),
            done: false,
            err: None,
        })
    };
    report(0);

    let rp = RuntimeParams::default_with_ch(1).with_atten_lim(atten_lim_db);
    let mut df = DfTract::new(DfParams::default(), &rp).map_err(|e| anyhow!("DeepFilterNet: {e}"))?;
    let hop = df.hop_size;
    let delay = df.delay();

    let mut dec_args: Vec<String> = vec!["-v".into(), "error".into()];
    if let Some((start, len)) = span {
        dec_args.extend(["-ss".into(), format!("{:.3}", start as f64 / 1000.0)]);
        dec_args.extend(["-t".into(), format!("{:.3}", len as f64 / 1000.0)]);
    }
    dec_args.extend(["-i".into(), input.display().to_string()]);
    dec_args.extend(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"].map(String::from));
    let duration_ms = span.map(|(_, len)| len).unwrap_or(duration_ms);
    let mut dec = spawn_ffmpeg(&dec_args, Stdio::null(), Stdio::piped())?;
    let mut enc_args: Vec<String> = ["-y", "-v", "error", "-f", "f32le", "-ar", "48000", "-ac", "1", "-i", "-"]
        .map(String::from)
        .to_vec();
    enc_args.extend(codec_args);
    enc_args.push(output.display().to_string());
    let mut enc = spawn_ffmpeg(&enc_args, Stdio::piped(), Stdio::null())?;

    let result = (|| -> Result<()> {
        let mut src = dec.stdout.take().unwrap();
        let mut sink = enc.stdin.take().unwrap();
        let total = (duration_ms * SR / 1000).max(1);
        let mut raw = vec![0u8; hop * 4];
        let mut noisy = Array2::<f32>::zeros((1, hop));
        let mut enh = Array2::<f32>::zeros((1, hop));
        let mut out_bytes = Vec::with_capacity(hop * 4);
        let (mut read_n, mut written_n, mut skip) = (0u64, 0u64, delay);
        let mut eof = false;
        let mut last_pct = 0u8;

        // Keep pulling frames until EOF, then push zeros until the delayed tail is out.
        while !eof || written_n < read_n {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("aborted"));
            }
            let mut got = 0;
            while !eof && got < raw.len() {
                match src.read(&mut raw[got..])? {
                    0 => eof = true,
                    n => got += n,
                }
            }
            let samples = got / 4;
            read_n += samples as u64;
            for (i, v) in noisy.iter_mut().enumerate() {
                *v = if i < samples {
                    f32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap())
                } else {
                    0.0
                };
            }
            df.process(ArrayView2::from(&noisy), enh.view_mut())
                .map_err(|e| anyhow!("DeepFilterNet: {e}"))?;

            out_bytes.clear();
            for &v in enh.iter().skip(skip) {
                if written_n >= read_n {
                    break;
                }
                out_bytes.extend_from_slice(&v.clamp(-1.0, 1.0).to_le_bytes());
                written_n += 1;
            }
            skip = skip.saturating_sub(hop);
            sink.write_all(&out_bytes).context("ffmpeg (الترميز) توقف")?;

            let pct = ((read_n * 100 / total).min(99)) as u8;
            if pct != last_pct {
                last_pct = pct;
                report(pct);
            }
        }
        Ok(())
    })();

    if let Err(e) = result {
        let _ = dec.kill();
        let _ = enc.kill();
        let _ = dec.wait();
        let _ = enc.wait();
        let _ = std::fs::remove_file(output);
        return Err(e);
    }
    if !dec.wait()?.success() {
        let _ = enc.kill();
        return Err(ffmpeg_err(dec));
    }
    let status = enc.wait()?;
    if !status.success() {
        return Err(ffmpeg_err(enc));
    }
    Ok(std::fs::metadata(output).map(|m| m.len()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export_audio::AudioFormat;

    /// 2 s tone burst + noise → denoised file keeps the same duration (latency compensated).
    #[test]
    fn denoise_keeps_duration() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("ffmpeg missing, skipped");
            return;
        }
        let dir = std::env::temp_dir().join("ayah_denoise_test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.wav");
        let dst = dir.join("out.mp3");
        let ok = Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=f=300:d=2", "-f", "lavfi", "-i",
                   "anoisesrc=d=2:a=0.05", "-filter_complex", "amix=inputs=2", "-ar", "44100"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let opts = AudioExportOpts { format: AudioFormat::Mp3, ..Default::default() };
        let bytes =
            denoise_file(&src, &dst, codec_for(&opts), 100.0, None, 2000, &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(bytes > 0);
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(&dst)
            .output()
            .unwrap();
        let dur: f64 = String::from_utf8_lossy(&probe.stdout).trim().parse().unwrap();
        assert!((dur - 2.0).abs() < 0.1, "duration {dur}");
        std::fs::remove_dir_all(&dir).ok();
    }
}

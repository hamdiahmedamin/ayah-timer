use anyhow::{anyhow, Context, Result};
use rodio::{Decoder, OutputStream, Sink};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphErr;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// File view that starts after a leading ID3v2 tag.
/// ponytail: symphonia 0.5 aborts the whole probe on a malformed ID3v2 frame
/// ("id3v2: invalid text encoding") and we never use tags, so we just skip them.
struct SkipId3 {
    f: File,
    off: u64,
    len: u64,
}

impl SkipId3 {
    fn open(path: &Path) -> io::Result<Self> {
        let mut f = File::open(path)?;
        let total = f.metadata()?.len();
        let mut h = [0u8; 10];
        let off = if f.read_exact(&mut h).is_ok()
            && &h[..3] == b"ID3"
            && h[6..10].iter().all(|b| b & 0x80 == 0)
        {
            let size = h[6..10].iter().fold(0u64, |a, &b| (a << 7) | b as u64);
            let footer = if h[5] & 0x10 != 0 { 10 } else { 0 };
            (10 + size + footer).min(total)
        } else {
            0
        };
        f.seek(SeekFrom::Start(off))?;
        Ok(Self { f, off, len: total - off })
    }
}

impl Read for SkipId3 {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.f.read(buf)
    }
}

impl Seek for SkipId3 {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(n) => self.len.checked_add_signed(n),
            SeekFrom::Current(n) => (self.f.stream_position()? - self.off).checked_add_signed(n),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        self.f.seek(SeekFrom::Start(self.off + target))?;
        Ok(target)
    }
}

impl MediaSource for SkipId3 {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

const AUDIO_EXTS: [&str; 7] = ["mp3", "wav", "m4a", "flac", "ogg", "opus", "aac"];

/// Surah number from a file name: the first run of digits in the stem, if 1..=114
/// ("001.mp3", "Surah_002", "114-An-Nas.mp3").
pub fn surah_from_name(stem: &str) -> Option<usize> {
    let digits: String = stem
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok().filter(|n| (1..=114).contains(n))
}

/// Map surah number → audio file for every audio file directly inside `dir`.
/// On duplicates the alphabetically first file wins.
pub fn scan_surah_folder(dir: &Path) -> std::collections::BTreeMap<usize, PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .collect();
    files.sort();
    let mut map = std::collections::BTreeMap::new();
    for f in files {
        if let Some(n) = f.file_stem().and_then(|s| s.to_str()).and_then(surah_from_name) {
            map.entry(n).or_insert(f);
        }
    }
    map
}

/// Play a file once on its own output stream (denoise previews), independent of
/// the main player. Set the returned flag to stop it early.
pub fn play_once(path: PathBuf) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        let Ok((_stream, handle)) = OutputStream::try_default() else { return };
        let Ok(file) = SkipId3::open(&path) else { return };
        let Ok(dec) = Decoder::new(BufReader::new(file)) else { return };
        let Ok(sink) = Sink::try_new(&handle) else { return };
        sink.append(dec);
        while !sink.empty() && !flag.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(40));
        }
        sink.stop();
        flag.store(true, Ordering::Relaxed);
    });
    stop
}

#[derive(Debug, Clone)]
pub struct AudioAnalysis {
    pub peaks: Vec<f32>,
    pub energy: Vec<f32>,
    pub duration_ms: u64,
}

enum AudioCmd {
    Load(PathBuf, f32),
    #[allow(dead_code)]
    Play,
    #[allow(dead_code)]
    Pause,
    Toggle,
    Seek(u64),
    PlaySpan(u64, Option<u64>),
    SetRate(f32),
    Clear,
}

#[derive(Clone)]
pub struct AudioPlayer {
    tx: Sender<AudioCmd>,
    pos_ms: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    has_audio: Arc<AtomicBool>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<AudioCmd>();
        let pos_ms = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let has_audio = Arc::new(AtomicBool::new(false));

        let p_ms = Arc::clone(&pos_ms);
        let p_play = Arc::clone(&playing);
        let p_has = Arc::clone(&has_audio);

        std::thread::spawn(move || {
            let stream = OutputStream::try_default().ok();
            let mut sink: Option<Sink> = None;
            let mut cur_path: Option<PathBuf> = None;
            let mut cur_rate: f32 = 1.0;
            let mut stop_at: Option<u64> = None;

            let open_sink = |path: &Path, rate: f32| -> Option<Sink> {
                let (_, handle) = stream.as_ref()?;
                let file = SkipId3::open(path).ok()?;
                let dec = Decoder::new(BufReader::new(file)).ok()?;
                let s = Sink::try_new(handle).ok()?;
                s.set_speed(rate);
                s.append(dec);
                s.pause();
                Some(s)
            };

            loop {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        AudioCmd::Load(path, rate) => {
                            if let Some(s) = sink.take() {
                                s.stop();
                            }
                            cur_rate = rate;
                            stop_at = None;
                            p_ms.store(0, Ordering::Relaxed);
                            p_play.store(false, Ordering::Relaxed);
                            if let Some(s) = open_sink(&path, rate) {
                                sink = Some(s);
                                cur_path = Some(path);
                                p_has.store(true, Ordering::Relaxed);
                            } else {
                                cur_path = None;
                                p_has.store(false, Ordering::Relaxed);
                            }
                        }
                        AudioCmd::Play => {
                            if let Some(s) = &sink {
                                s.play();
                                p_play.store(true, Ordering::Relaxed);
                            }
                        }
                        AudioCmd::Pause => {
                            stop_at = None;
                            if let Some(s) = &sink {
                                s.pause();
                            }
                            p_play.store(false, Ordering::Relaxed);
                        }
                        AudioCmd::Toggle => {
                            stop_at = None;
                            if let Some(s) = &sink {
                                if s.is_paused() {
                                    if s.empty() {
                                        if let Some(p) = &cur_path {
                                            if let Some(ns) = open_sink(p, cur_rate) {
                                                ns.play();
                                                sink = Some(ns);
                                                p_play.store(true, Ordering::Relaxed);
                                            }
                                        }
                                    } else {
                                        s.play();
                                        p_play.store(true, Ordering::Relaxed);
                                    }
                                } else {
                                    s.pause();
                                    p_play.store(false, Ordering::Relaxed);
                                }
                            }
                        }
                        AudioCmd::Seek(ms) => {
                            stop_at = None;
                            if let Some(s) = &sink {
                                if s.empty() {
                                    if let Some(p) = &cur_path {
                                        if let Some(ns) = open_sink(p, cur_rate) {
                                            let _ = ns.try_seek(Duration::from_millis(ms));
                                            sink = Some(ns);
                                        }
                                    }
                                } else {
                                    let _ = s.try_seek(Duration::from_millis(ms));
                                }
                                p_ms.store(ms, Ordering::Relaxed);
                            }
                        }
                        AudioCmd::PlaySpan(start, end) => {
                            stop_at = end.filter(|&e| e > start);
                            if let Some(s) = &sink {
                                if s.empty() {
                                    if let Some(p) = &cur_path {
                                        if let Some(ns) = open_sink(p, cur_rate) {
                                            let _ = ns.try_seek(Duration::from_millis(start));
                                            ns.play();
                                            sink = Some(ns);
                                            p_play.store(true, Ordering::Relaxed);
                                        }
                                    }
                                } else {
                                    let _ = s.try_seek(Duration::from_millis(start));
                                    s.play();
                                    p_play.store(true, Ordering::Relaxed);
                                }
                                p_ms.store(start, Ordering::Relaxed);
                            }
                        }
                        AudioCmd::SetRate(rate) => {
                            cur_rate = rate;
                            if let Some(s) = &sink {
                                s.set_speed(rate);
                            }
                        }
                        AudioCmd::Clear => {
                            stop_at = None;
                            if let Some(s) = sink.take() {
                                s.stop();
                            }
                            cur_path = None;
                            p_has.store(false, Ordering::Relaxed);
                            p_play.store(false, Ordering::Relaxed);
                            p_ms.store(0, Ordering::Relaxed);
                        }
                    }
                }

                if let Some(s) = &sink {
                    let pos = s.get_pos().as_millis() as u64;
                    p_ms.store(pos, Ordering::Relaxed);
                    if s.empty() {
                        p_play.store(false, Ordering::Relaxed);
                        stop_at = None;
                    } else if let Some(end) = stop_at {
                        if !s.is_paused() && pos >= end.saturating_sub(25) {
                            s.pause();
                            p_play.store(false, Ordering::Relaxed);
                            stop_at = None;
                        }
                    }
                }

                std::thread::sleep(Duration::from_millis(15));
            }
        });

        Self {
            tx,
            pos_ms,
            playing,
            has_audio,
        }
    }

    pub fn load(&self, path: PathBuf, rate: f32) {
        let _ = self.tx.send(AudioCmd::Load(path, rate));
    }

    pub fn toggle(&self) {
        let _ = self.tx.send(AudioCmd::Toggle);
    }

    #[allow(dead_code)]
    pub fn pause(&self) {
        let _ = self.tx.send(AudioCmd::Pause);
    }

    pub fn seek(&self, ms: u64) {
        let _ = self.tx.send(AudioCmd::Seek(ms));
    }

    pub fn play_span(&self, start_ms: u64, end_ms: Option<u64>) {
        let _ = self.tx.send(AudioCmd::PlaySpan(start_ms, end_ms));
    }

    pub fn set_rate(&self, rate: f32) {
        let _ = self.tx.send(AudioCmd::SetRate(rate));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(AudioCmd::Clear);
    }

    pub fn position_ms(&self) -> u64 {
        self.pos_ms.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn has_audio(&self) -> bool {
        self.has_audio.load(Ordering::Relaxed)
    }
}

fn open_symphonia(path: &Path) -> Result<(Box<dyn symphonia::core::formats::FormatReader>, Box<dyn symphonia::core::codecs::Decoder>, u32, u32, usize, u64)> {
    let file = SkipId3::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("unsupported audio format")?;
    let format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no audio track found"))?;
    let track_id = track.id;
    let sr = track.codec_params.sample_rate.unwrap_or(44100);
    let ch = track.codec_params.channels.map(|c| c.count()).unwrap_or(1).max(1);
    let est_frames = track.codec_params.n_frames.unwrap_or(0);
    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("unsupported audio codec")?;
    Ok((format, decoder, track_id, sr, ch, est_frames))
}

/// Streaming waveform peaks (6000 bins) and 10ms RMS energy in constant low RAM.
pub fn analyze_audio_file(
    path: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u8),
) -> Result<AudioAnalysis> {
    let (mut format, mut decoder, track_id, sr, _ch, est_frames) = open_symphonia(path)?;

    const P: usize = 6000;
    let win10 = ((sr as f64) / 100.0).round().max(1.0) as usize;
    let mut energy = Vec::new();
    let mut rms_acc = 0.0f64;
    let mut rms_cnt = 0usize;

    // Collect fine peak buckets (up to 60,000) and downsample to P=6000 at end so
    // duration doesn't need to be known upfront.
    let fine_step = if est_frames > 0 {
        (est_frames as usize / (P * 8)).max(1)
    } else {
        (sr as usize / 20).max(1)
    };
    let mut fine_peaks = Vec::new();
    let mut fine_max = 0.0f32;
    let mut fine_cnt = 0usize;

    let mut total_samples: u64 = 0;
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut last_pct = 0u8;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("aborted"));
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphErr::IoError(_)) | Err(SymphErr::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphErr::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let frames = decoded.frames();
        if sbuf.as_ref().map(|b| b.capacity() < frames * ch).unwrap_or(true) {
            sbuf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let buf = sbuf.as_mut().unwrap();
        buf.copy_interleaved_ref(decoded);
        let samples = buf.samples();

        for frame in samples.chunks_exact(ch) {
            let v = frame[0];
            let a = v.abs();
            if a > fine_max {
                fine_max = a;
            }
            fine_cnt += 1;
            if fine_cnt >= fine_step {
                fine_peaks.push(fine_max);
                fine_max = 0.0;
                fine_cnt = 0;
            }

            rms_acc += (v as f64) * (v as f64);
            rms_cnt += 1;
            if rms_cnt >= win10 {
                energy.push((rms_acc / rms_cnt as f64).sqrt() as f32);
                rms_acc = 0.0;
                rms_cnt = 0;
            }
        }
        total_samples += frames as u64;

        if est_frames > 0 {
            let pct = ((total_samples * 100) / est_frames).min(100) as u8;
            if pct >= last_pct + 3 {
                last_pct = pct;
                on_progress(pct);
            }
        }
    }
    if fine_cnt > 0 {
        fine_peaks.push(fine_max);
    }

    let mut peaks = vec![0.0f32; P];
    if !fine_peaks.is_empty() {
        let fn_len = fine_peaks.len();
        for (i, p) in peaks.iter_mut().enumerate() {
            let a = (i * fn_len) / P;
            let b = (((i + 1) * fn_len) / P).max(a + 1).min(fn_len);
            let mut m = 0.0f32;
            for &v in &fine_peaks[a..b] {
                if v > m {
                    m = v;
                }
            }
            *p = m;
        }
        let top = peaks.iter().copied().fold(0.0f32, f32::max).max(1e-6);
        for p in &mut peaks {
            *p /= top;
        }
    }

    let duration_ms = ((total_samples as f64 / sr as f64) * 1000.0).round() as u64;
    Ok(AudioAnalysis {
        peaks,
        energy,
        duration_ms,
    })
}

/// Stream 16 kHz mono f32 chunks (with overlap) in constant low RAM for multi-hour AI alignment.
pub fn stream_pcm_16k(
    path: &Path,
    chunk_sec: usize,
    overlap_sec: f32,
    cancel: &AtomicBool,
    mut on_chunk: impl FnMut(&[f32], usize, u64, u64) -> Result<()>,
) -> Result<u64> {
    let (mut format, mut decoder, track_id, sr, _ch, est_frames) = open_symphonia(path)?;
    let target_sr = 16000.0f64;
    let step = sr as f64 / target_sr; // input frames per 16k output sample
    let est_ms = if est_frames > 0 {
        ((est_frames as f64 / sr as f64) * 1000.0).round() as u64
    } else {
        0
    };

    let chunk_samples = chunk_sec * 16000;
    let ov_samples = (overlap_sec * 16000.0).round() as usize;
    let ov_frames = ov_samples / 320;

    let mut mono_buf: Vec<f32> = Vec::with_capacity(65536);
    let mut frac_pos = 0.0f64;
    let mut resampled: Vec<f32> = Vec::with_capacity(chunk_samples + ov_samples + 4096);
    let mut tail: Vec<f32> = Vec::new();
    let mut processed_samples: u64 = 0;
    let mut sbuf: Option<SampleBuffer<f32>> = None;

    let flush = |core: &[f32],
                 tail: &mut Vec<f32>,
                 processed: &mut u64,
                 cb: &mut dyn FnMut(&[f32], usize, u64, u64) -> Result<()>|
     -> Result<()> {
        let mut wav = Vec::with_capacity(tail.len() + core.len());
        wav.extend_from_slice(tail);
        wav.extend_from_slice(core);
        let drop_left = if tail.len() == ov_samples {
            ov_frames
        } else {
            tail.len() / 320
        };
        *processed += core.len() as u64;
        let done_ms = (*processed * 1000) / 16000;
        let dur_ms = est_ms.max(done_ms);
        cb(&wav, drop_left, done_ms, dur_ms)?;
        tail.clear();
        if core.len() >= ov_samples {
            tail.extend_from_slice(&core[core.len() - ov_samples..]);
        }
        Ok(())
    };

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("aborted"));
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphErr::IoError(_)) | Err(SymphErr::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphErr::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let frames = decoded.frames();
        if sbuf.as_ref().map(|b| b.capacity() < frames * ch).unwrap_or(true) {
            sbuf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let buf = sbuf.as_mut().unwrap();
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks_exact(ch) {
            let mut sum = 0.0f32;
            for &s in frame {
                sum += s;
            }
            mono_buf.push(sum / ch as f32);
        }

        // Linear resample to 16 kHz
        if mono_buf.len() >= 2 {
            let limit = (mono_buf.len() - 1) as f64;
            while frac_pos < limit {
                let idx = frac_pos as usize;
                let frac = (frac_pos - idx as f64) as f32;
                let s = mono_buf[idx] + (mono_buf[idx + 1] - mono_buf[idx]) * frac;
                resampled.push(s);
                frac_pos += step;
            }
            let consumed = frac_pos.floor() as usize;
            if consumed > 0 && consumed <= mono_buf.len() {
                mono_buf.drain(..consumed);
                frac_pos -= consumed as f64;
            }
        }

        while resampled.len() >= chunk_samples {
            flush(&resampled[..chunk_samples], &mut tail, &mut processed_samples, &mut on_chunk)?;
            resampled.drain(..chunk_samples);
        }
    }

    if !resampled.is_empty() {
        flush(&resampled, &mut tail, &mut processed_samples, &mut on_chunk)?;
    }

    Ok((processed_samples * 1000) / 16000)
}

#[test]
fn skip_id3_view() {
    let p = std::env::temp_dir().join("ayah_skip_id3_test.bin");
    // ID3v2 header with syncsafe size 5, then 5 tag bytes, then payload "MP3!"
    std::fs::write(&p, [b"ID3\x03\x00\x00\x00\x00\x00\x05".as_slice(), b"xxxxx", b"MP3!"].concat()).unwrap();
    let mut v = SkipId3::open(&p).unwrap();
    assert_eq!(v.byte_len(), Some(4));
    let mut s = String::new();
    v.read_to_string(&mut s).unwrap();
    assert_eq!(s, "MP3!");
    assert_eq!(v.seek(SeekFrom::End(-1)).unwrap(), 3);
    assert!(v.seek(SeekFrom::Current(-10)).is_err());
    std::fs::remove_file(&p).ok();
}


#[test]
fn surah_numbers_from_names() {
    assert_eq!(surah_from_name("001"), Some(1));
    assert_eq!(surah_from_name("Surah_002 Al-Baqara"), Some(2));
    assert_eq!(surah_from_name("114-An-Nas"), Some(114));
    assert_eq!(surah_from_name("115"), None);
    assert_eq!(surah_from_name("000"), None);
    assert_eq!(surah_from_name("cover"), None);
}

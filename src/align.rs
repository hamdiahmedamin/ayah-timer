use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::audio::stream_pcm_16k;
use crate::quran::{cache_dir, fmt_ms, http_client, normalize_for_vocab, strip_basmala_for_align};

const MODEL_BASE: &str =
    "https://huggingface.co/onnx-community/wav2vec2-base-arabic-without-reused-en-token-ids-v1-ONNX/resolve/main";

#[derive(Debug, Clone)]
pub struct AiProgress {
    pub text: String,
    pub pct: Option<u8>,
    pub done: bool,
    pub err: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct Gap {
    len: u64,
    at: u64,
}

fn pauses(energy: &[f32], min_needed: usize) -> (Vec<Gap>, u64) {
    if energy.is_empty() {
        return (vec![Gap { len: 120, at: 0 }], 0);
    }
    let mut sorted = energy.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted[(sorted.len() as f64 * 0.15) as usize];
    let voice = sorted[(sorted.len() as f64 * 0.70) as usize];
    let span = (voice - floor).max(0.0);
    let base_quiet = floor + span * 0.18;

    let mut last_voice = 0usize;
    for (i, &e) in energy.iter().enumerate() {
        if e >= base_quiet {
            last_voice = i;
        }
    }
    let end = if last_voice > 0 {
        last_voice as u64 * 10
    } else {
        energy.len() as u64 * 10
    };

    let scan = |ratio: f32, min_frames: usize| -> Vec<Gap> {
        let quiet = floor + span * ratio;
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < energy.len() {
            if energy[i] < quiet {
                let mut j = i;
                while j < energy.len() && energy[j] < quiet {
                    j += 1;
                }
                if j - i >= min_frames && j < energy.len().saturating_sub(3) {
                    out.push(Gap {
                        len: ((j - i) * 10) as u64,
                        at: (j as u64 * 10).saturating_sub(60),
                    });
                }
                i = j;
            } else {
                i += 1;
            }
        }
        out
    };

    let mut gaps = scan(0.18, 12);
    if gaps.len() < min_needed {
        gaps = scan(0.26, 7);
    }
    if gaps.len() < min_needed {
        gaps = scan(0.36, 4);
    }
    if !gaps.iter().any(|g| g.at <= 150) {
        gaps.insert(0, Gap { len: 120, at: 0 });
    }
    if gaps.len() < min_needed && energy.len() > 10 {
        let bins = (min_needed * 2).max(12);
        let step = ((end / 10) as usize / bins).max(8);
        let mut s = 4usize;
        while s < energy.len().saturating_sub(4) {
            let mut best_i = s;
            let mut low = energy[s];
            for k in s..energy.len().saturating_sub(3).min(s + step) {
                if energy[k] < low {
                    low = energy[k];
                    best_i = k;
                }
            }
            let at = (best_i as u64 * 10).saturating_sub(30);
            if !gaps.iter().any(|g| g.at.abs_diff(at) < 120) {
                gaps.push(Gap { len: 80, at });
            }
            s += step;
        }
        gaps.sort_by_key(|g| g.at);
    }
    (gaps, end)
}

fn verse_lengths(texts: &[String], surah: usize, count: usize) -> Option<Vec<usize>> {
    if texts.len() != count {
        return None;
    }
    Some(
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let s = if i == 0 {
                    strip_basmala_for_align(surah, t)
                } else {
                    t.trim_start_matches('\u{FEFF}').to_string()
                };
                let letters = s
                    .chars()
                    .filter(|&c| ('ء'..='ي').contains(&c) || c == 'ٱ' || c == 'ى' || c == 'ے')
                    .count();
                letters.max(4)
            })
            .collect(),
    )
}

pub fn auto_heuristic(
    energy: &[f32],
    texts: Option<&[String]>,
    surah: usize,
    count: usize,
    current_starts: &[Option<u64>],
) -> Vec<Option<u64>> {
    let (gaps, end) = pauses(energy, count);
    let lens = texts.and_then(|t| verse_lengths(t, surah, count));
    let fixed: Vec<Option<u64>> = (0..=count)
        .map(|a| {
            if a >= 1 {
                current_starts.get(a).copied().flatten()
            } else {
                None
            }
        })
        .collect();

    let snap_local = |ms: u64, lo: u64, hi: u64| -> u64 {
        let at = (ms as f64 / 10.0).round() as usize;
        let a0 = at.saturating_sub(40);
        let a1 = (at + 40).min(energy.len().saturating_sub(1));
        let mut best = at.min(a1);
        let mut low = f32::INFINITY;
        for i in a0..=a1 {
            if energy[i] < low {
                low = energy[i];
                best = i;
            }
        }
        ((best as u64) * 10).clamp(lo, hi)
    };

    let fill_proportional = || -> Vec<u64> {
        let mut anchors = vec![(0usize, 0u64)];
        for a in 1..=count {
            if let Some(ms) = fixed[a] {
                anchors.push((a, ms));
            }
        }
        anchors.push((count + 1, end));
        let mut out = vec![0u64; count + 1];
        for w in anchors.windows(2) {
            let (a0, t0) = w[0];
            let (a1, t1) = w[1];
            if a0 > 0 {
                out[a0] = t0;
            }
            let need = a1.saturating_sub(a0 + 1);
            if need == 0 {
                continue;
            }
            let lo = if a0 == 0 { 0 } else { t0 + 40 };
            let hi = (t1.saturating_sub(40)).max(lo + (need as u64) * 40);
            let mut span_gaps: Vec<Gap> = gaps.iter().copied().filter(|g| g.at >= lo && g.at <= hi).collect();
            if span_gaps.len() >= need {
                span_gaps.sort_by(|a, b| b.len.cmp(&a.len));
                span_gaps.truncate(need);
                span_gaps.sort_by_key(|g| g.at);
                for (i, g) in span_gaps.into_iter().enumerate() {
                    out[a0 + 1 + i] = g.at;
                }
            } else {
                let weights: Vec<usize> = if let Some(ref l) = lens {
                    l[a0..a1].to_vec()
                } else {
                    vec![1; need + 1]
                };
                let sum_w: usize = weights.iter().sum::<usize>().max(need + 1);
                let mut acc = 0usize;
                for i in 0..need {
                    acc += weights[i];
                    let prev = if i == 0 { lo } else { out[a0 + i] + 40 };
                    let max_t = hi.saturating_sub(((need - 1 - i) as u64) * 40);
                    let raw = t0 + ((t1.saturating_sub(t0) as f64) * (acc as f64 / sum_w as f64)) as u64;
                    out[a0 + 1 + i] = snap_local(raw, prev, prev.max(max_t));
                }
            }
        }
        out[1..].to_vec()
    };

    let Some(ref lens) = lens else {
        let mut res = vec![Some(0)];
        res.extend(fill_proportional().into_iter().map(Some));
        return res;
    };

    #[derive(Clone, Copy)]
    struct Pt {
        at: u64,
        reward: f64,
        verse: usize,
    }
    let mut points: Vec<Pt> = gaps
        .iter()
        .map(|g| Pt {
            at: g.at,
            reward: (1.0 + g.len as f64 / 100.0).ln(),
            verse: 0,
        })
        .collect();
    for (a, ms) in fixed.iter().enumerate().skip(1) {
        if let Some(ms) = ms {
            points.push(Pt {
                at: *ms,
                reward: 0.0,
                verse: a,
            });
        }
    }
    points.sort_by_key(|p| p.at);
    let k_len = points.len();
    let total_l: usize = lens.iter().sum();

    let solve_dp = |strict: bool| -> Option<Vec<u64>> {
        if k_len < count {
            return None;
        }
        let mut rate = end as f64 / (total_l + 20) as f64;
        let mut best: Option<Vec<u64>> = None;
        for _pass in 0..2 {
            let exp: Vec<f64> = lens.iter().map(|&l| l as f64 * rate).collect();
            let cost = |dur: u64, e: f64| -> f64 {
                let r = ((dur.max(50) as f64) / e).ln();
                r * r
            };
            let l_wt = 0.45;
            let mut dp = vec![f64::INFINITY; k_len];
            let mut from: Vec<Vec<i32>> = Vec::with_capacity(count);
            let opening = 20.0 * rate;
            for k in 0..k_len {
                let allow = if let Some(_) = fixed[1] {
                    points[k].verse == 1
                } else {
                    points[k].verse == 0
                };
                if allow && (!strict || (points[k].at as f64) < (end as f64 * 0.6) || fixed[1].is_some()) {
                    dp[k] = cost(points[k].at, opening) - l_wt * points[k].reward;
                }
            }
            from.push(vec![]);

            for a in 1..count {
                let mut next = vec![f64::INFINITY; k_len];
                let mut back = vec![-1i32; k_len];
                let e = exp[a - 1];
                let lo = if strict { (e * 0.12).max(120.0) as u64 } else { 40 };
                let hi = if strict {
                    ((e * 8.0 + 4000.0).max(12000.0)) as u64
                } else {
                    u64::MAX
                };
                let free = !strict || fixed[a].is_some() || fixed[a + 1].is_some();
                let mut j0 = 0usize;
                for k in 0..k_len {
                    let allow = if fixed[a + 1].is_some() {
                        points[k].verse == a + 1
                    } else {
                        points[k].verse == 0
                    };
                    if !allow {
                        continue;
                    }
                    while !free && j0 < k && points[k].at.saturating_sub(points[j0].at) > hi {
                        j0 += 1;
                    }
                    let mut m = f64::INFINITY;
                    let mut arg = -1i32;
                    let start_j = if free { 0 } else { j0 };
                    for j in start_j..k {
                        let min_d = if free { 40 } else { lo };
                        if points[k].at.saturating_sub(points[j].at) < min_d {
                            break;
                        }
                        if dp[j].is_infinite() {
                            continue;
                        }
                        let c = dp[j] + cost(points[k].at - points[j].at, e);
                        if c < m {
                            m = c;
                            arg = j as i32;
                        }
                    }
                    if arg >= 0 {
                        next[k] = m - l_wt * points[k].reward;
                        back[k] = arg;
                    }
                }
                dp = next;
                from.push(back);
            }

            let mut m = f64::INFINITY;
            let mut last = -1i32;
            let end_min = if strict { 150 } else { 30 };
            for k in 0..k_len {
                if dp[k].is_infinite() || end.saturating_sub(points[k].at) < end_min {
                    continue;
                }
                let c = dp[k] + cost(end - points[k].at, exp[count - 1]);
                if c < m {
                    m = c;
                    last = k as i32;
                }
            }
            if last < 0 {
                break;
            }
            let mut idx = vec![last as usize];
            for a in (1..count).rev() {
                idx.insert(0, from[a][idx[0]] as usize);
            }
            let b: Vec<u64> = idx.into_iter().map(|k| points[k].at).collect();
            rate = (end.saturating_sub(b[0]).max(1) as f64) / total_l as f64;
            best = Some(b);
        }
        best
    };

    let best = solve_dp(true)
        .or_else(|| solve_dp(false))
        .unwrap_or_else(fill_proportional);
    let mut out = vec![Some(0)];
    out.extend(best.into_iter().map(Some));
    out
}

async fn download_with_progress(
    url: &str,
    dest: &Path,
    label: &str,
    cancel: &AtomicBool,
    on_progress: &impl Fn(AiProgress),
) -> Result<()> {
    if dest.exists() && dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(());
    }
    if let Some(p) = dest.parent() {
        tokio::fs::create_dir_all(p).await?;
    }
    let tmp = dest.with_extension("part");
    let mut resp = http_client()?.get(url).send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);
    let mut out = Vec::with_capacity(total as usize);
    let mut got = 0u64;
    let mut last_ui = std::time::Instant::now();

    while let Some(chunk) = resp.chunk().await? {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("aborted"));
        }
        out.extend_from_slice(&chunk);
        got += chunk.len() as u64;
        if last_ui.elapsed().as_millis() >= 150 {
            last_ui = std::time::Instant::now();
            let pct = if total > 0 {
                Some(((got * 100) / total).min(100) as u8)
            } else {
                None
            };
            on_progress(AiProgress {
                text: format!(
                    "جارٍ تنزيل {label}… {:.1} / {:.1} MB",
                    got as f64 / 1_048_576.0,
                    total as f64 / 1_048_576.0
                ),
                pct,
                done: false,
                err: None,
            });
        }
    }
    tokio::fs::write(&tmp, &out).await?;
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

pub async fn ensure_ai_model(
    cancel: &AtomicBool,
    on_progress: &impl Fn(AiProgress),
) -> Result<(std::path::PathBuf, HashMap<String, usize>)> {
    let dir = cache_dir();
    let vocab_path = dir.join("vocab.json");
    let model_path = dir.join("model_q4.onnx");

    download_with_progress(
        &format!("{MODEL_BASE}/vocab.json"),
        &vocab_path,
        "قاموس الأحرف",
        cancel,
        on_progress,
    )
    .await?;

    download_with_progress(
        &format!("{MODEL_BASE}/onnx/model_q4.onnx"),
        &model_path,
        "نموذج الذكاء الاصطناعي (86 MB)",
        cancel,
        on_progress,
    )
    .await?;

    let vocab_raw = tokio::fs::read_to_string(&vocab_path).await?;
    let vocab: HashMap<String, usize> = serde_json::from_str(&vocab_raw)?;
    Ok((model_path, vocab))
}

fn quantize_log_softmax(logits: &[f32], t_len: usize, v_len: usize, drop_left: usize) -> (Vec<u8>, usize) {
    let keep = t_len.saturating_sub(drop_left);
    let mut out = vec![0u8; keep * v_len];
    for t in 0..keep {
        let src = (t + drop_left) * v_len;
        let dst = t * v_len;
        let row = &logits[src..src + v_len];
        let mx = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let sum: f32 = row.iter().map(|&v| (v - mx).exp()).sum();
        let lse = mx + sum.ln();
        for v in 0..v_len {
            let q = ((lse - row[v]) * 10.0).round();
            out[dst + v] = q.clamp(0.0, 255.0) as u8;
        }
    }
    (out, keep)
}

fn align_ctc(
    em: &[u8],
    n_frames: usize,
    v_len: usize,
    ayah_tokens: &[Vec<usize>],
    blank_id: usize,
    beam: usize,
    cancel: &AtomicBool,
    on_progress: &impl Fn(AiProgress),
) -> Result<Vec<u64>> {
    let mut states: Vec<i32> = vec![-1, blank_id as i32];
    let mut first_state = Vec::with_capacity(ayah_tokens.len());
    for toks in ayah_tokens {
        if toks.is_empty() {
            first_state.push(states.len() - 1);
            continue;
        }
        first_state.push(states.len());
        for &tk in toks {
            states.push(tk as i32);
            states.push(blank_id as i32);
        }
    }
    states.push(-2);

    let s_len = states.len();
    let mut can_skip = vec![false; s_len];
    for s in 2..s_len {
        if s == s_len - 1 || (states[s] != blank_id as i32 && states[s] != states[s - 2]) {
            can_skip[s] = true;
        }
    }

    for &try_beam in &[beam, beam.max(8000).min(s_len), s_len] {
        let w = try_beam.min(s_len);
        let row_bytes = (w + 3) >> 2;
        let mut bp = vec![0u8; n_frames * row_bytes];
        let mut lo_arr = vec![0usize; n_frames];
        let mut dp = vec![-1e30f32; s_len];
        let mut next_dp = vec![-1e30f32; s_len];
        dp[0] = 0.0;
        dp[1] = 0.0;
        if s_len > 2 {
            dp[2] = 0.0;
        }

        let mut lo = 0usize;
        let mut hi = w;

        for t in 0..n_frames {
            if t > 0 && t % 4000 == 0 {
                if cancel.load(Ordering::Relaxed) {
                    return Err(anyhow!("aborted"));
                }
                let pct = 95 + ((t * 4) / n_frames).min(4) as u8;
                on_progress(AiProgress {
                    text: "🤖 جارٍ مطابقة الآيات مع الصوت…".into(),
                    pct: Some(pct),
                    done: false,
                    err: None,
                });
            }
            let e_base = t * v_len;
            let e_row = &em[e_base..e_base + v_len];
            let min_q = e_row.iter().copied().min().unwrap_or(0);
            let star_val = -(min_q as f32) * 0.1 - 0.18;
            lo_arr[t] = lo;
            let bp_off = t * row_bytes;

            let mut best_score = -1e30f32;
            let mut best_s = lo;
            for s in lo..hi {
                let st = states[s];
                let emit = if st < 0 {
                    star_val
                } else {
                    -(e_row[st as usize] as f32) * 0.1
                };
                let mut best = dp[s];
                let mut step = 0u8;
                if s >= 1 && dp[s - 1] > best {
                    best = dp[s - 1];
                    step = 1;
                }
                if s >= 2 && can_skip[s] && dp[s - 2] > best {
                    best = dp[s - 2];
                    step = 2;
                }
                let val = best + emit;
                next_dp[s] = val;
                if val > best_score {
                    best_score = val;
                    best_s = s;
                }
                let k = s - lo;
                bp[bp_off + (k >> 2)] |= step << ((k & 3) * 2);
            }
            dp[lo..hi].fill(-1e30f32);
            std::mem::swap(&mut dp, &mut next_dp);
            if w < s_len {
                lo = best_s.saturating_sub(w >> 2).min(s_len - w);
                hi = lo + w;
            }
        }

        let end_s = if s_len >= 2 && dp[s_len - 2] > dp[s_len - 1] {
            s_len - 2
        } else {
            s_len - 1
        };
        if dp[end_s] > -1e20 || w == s_len {
            let mut first_frame = vec![0usize; s_len];
            let mut s = end_s;
            for t in (0..n_frames).rev() {
                first_frame[s] = t;
                let k = s.saturating_sub(lo_arr[t]);
                if s >= lo_arr[t] && k < w {
                    let step = ((bp[t * row_bytes + (k >> 2)] >> ((k & 3) * 2)) & 3) as usize;
                    s = s.saturating_sub(step);
                }
            }
            return Ok(first_state.into_iter().map(|st| (first_frame[st] as u64) * 20).collect());
        }
    }
    Err(anyhow!("تعذّرت المحاذاة"))
}

fn refine_onset(starts: &[u64], energy: &[f32], total_ms: u64) -> Vec<u64> {
    if energy.len() < 50 {
        return starts.to_vec();
    }
    let mut sorted = energy.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted[(sorted.len() as f64 * 0.15) as usize];
    let voice = sorted[(sorted.len() as f64 * 0.70) as usize];
    let quiet = floor + (voice - floor) * 0.20;

    let mut out = starts.to_vec();
    for i in 0..out.len() {
        let lo = if i == 0 { 0 } else { out[i - 1] + 50 };
        let hi = if i + 1 == out.len() {
            total_ms.saturating_sub(50)
        } else {
            out[i + 1].saturating_sub(50)
        };
        if hi < lo {
            continue;
        }
        let at = (out[i] as f64 / 10.0).round() as usize;
        let a0 = at.saturating_sub(25);
        let a1 = (at + 25).min(energy.len() - 1);
        if a1 <= a0 {
            continue;
        }
        let mut best = a0;
        let mut low = energy[a0];
        for k in (a0 + 1)..=a1 {
            if energy[k] < low {
                low = energy[k];
                best = k;
            }
        }
        if low < quiet {
            let mut j = best;
            let back = low * 3.0 + 0.005;
            while j < energy.len() - 1 && j < best + 40 && energy[j] < back {
                j += 1;
            }
            let cand = (j as u64 * 10).saturating_sub(40);
            if cand >= lo && cand <= hi {
                out[i] = cand;
            }
        }
    }
    out
}

pub fn run_ai_alignment(
    audio_path: &Path,
    model_path: &Path,
    vocab: &HashMap<String, usize>,
    texts: &[String],
    surah: usize,
    energy: &[f32],
    duration_ms: u64,
    cancel: &AtomicBool,
    on_progress: &impl Fn(AiProgress),
) -> Result<Vec<u64>> {
    on_progress(AiProgress {
        text: "جارٍ تهيئة نموذج الذكاء الاصطناعي…".into(),
        pct: None,
        done: false,
        err: None,
    });

    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(2);
    let env = ort::init().build().map_err(|e| anyhow!("{e}"))?;
    let mut sess = ort::session::Session::builder(&env)
        .map_err(|e| anyhow!("{e}"))?
        .with_intra_threads(threads)
        .map_err(|e| anyhow!("{e}"))?
        .commit_from_file(model_path)
        .map_err(|e| anyhow!("{e}"))?;

    let v_len = vocab.values().copied().max().unwrap_or(33) + 3;
    let blank_id = *vocab.get("<pad>").unwrap_or(&33);
    let mut em_all: Vec<u8> = Vec::new();
    let mut total_frames = 0usize;

    let decoded_ms = stream_pcm_16k(audio_path, 15, 1.5, cancel, |wav, drop_left, done_ms, dur_ms| {
        let n = wav.len();
        let mean = wav.iter().map(|&x| x as f64).sum::<f64>() / n.max(1) as f64;
        let var = wav
            .iter()
            .map(|&x| {
                let d = x as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / n.max(1) as f64;
        let inv = (1.0 / (var + 1e-7).sqrt()) as f32;
        let mean_f = mean as f32;
        let normed: Vec<f32> = wav.iter().map(|&x| (x - mean_f) * inv).collect();

        let tensor = ort::value::Tensor::from_array(([1usize, n], normed.into_boxed_slice()))
            .map_err(|e| anyhow!("{e}"))?;
        let outputs = sess
            .run(ort::inputs!["input_values" => tensor])
            .map_err(|e| anyhow!("{e}"))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("{e}"))?;
        let t_len = shape[1] as usize;
        let out_v = shape[2] as usize;
        let (q, frames) = quantize_log_softmax(data, t_len, out_v, drop_left);
        em_all.extend_from_slice(&q);
        total_frames += frames;

        let pct = if dur_ms > 0 {
            Some(((done_ms * 95) / dur_ms).min(95) as u8)
        } else {
            None
        };
        on_progress(AiProgress {
            text: format!("🤖 تحليل الصوت: {} / {}", fmt_ms(done_ms), fmt_ms(dur_ms)),
            pct,
            done: false,
            err: None,
        });
        Ok(())
    })?;

    on_progress(AiProgress {
        text: "🤖 جارٍ مطابقة الآيات مع الصوت…".into(),
        pct: Some(96),
        done: false,
        err: None,
    });

    let ayah_tokens: Vec<Vec<usize>> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let s = if i == 0 {
                strip_basmala_for_align(surah, t)
            } else {
                t.trim_start_matches('\u{FEFF}').to_string()
            };
            normalize_for_vocab(&s, vocab)
        })
        .collect();

    let raw_starts = align_ctc(
        &em_all,
        total_frames,
        v_len,
        &ayah_tokens,
        blank_id,
        2500,
        cancel,
        on_progress,
    )?;

    let final_ms = duration_ms.max(decoded_ms);
    let refined = refine_onset(&raw_starts, energy, final_ms);
    Ok(refined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::analyze_audio_file;
    use crate::quran::{fetch_surah_text, TextEdition};

    #[tokio::test]
    async fn real_audio_hafs_surah1_ai_alignment() {
        let cancel = AtomicBool::new(false);
        let (model_path, vocab) = ensure_ai_model(&cancel, &|_| {}).await.unwrap();
        let st = fetch_surah_text(1, TextEdition::Hafs).await.unwrap();
        let tmp = std::env::temp_dir().join("ayah_timer_test_001.mp3");
        if !tmp.exists() {
            let bytes = http_client()
                .unwrap()
                .get("https://cdn.mp3quran.net/audio/mishary-alafasy/r1/001.mp3")
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            tokio::fs::write(&tmp, &bytes).await.unwrap();
        }
        let analysis = analyze_audio_file(&tmp, &cancel, |_| {}).unwrap();
        let starts = run_ai_alignment(
            &tmp,
            &model_path,
            &vocab,
            &st.texts,
            1,
            &analysis.energy,
            analysis.duration_ms,
            &cancel,
            &|_| {},
        )
        .unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(starts.len(), 7);
        for w in starts.windows(2) {
            assert!(w[1] > w[0], "non-monotonic: {starts:?}");
        }
        assert!(starts[0] > 5000 && starts[0] < 8000, "unexpected v1 start: {}", starts[0]);
    }
}

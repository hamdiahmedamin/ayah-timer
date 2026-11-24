use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::quran::data_dir;

#[derive(Debug, Clone)]
pub struct TimelineRow {
    pub ayah: usize,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mp3QuranRow {
    pub ayah: usize,
    pub polygon: Option<String>,
    pub start_time: u64,
    pub end_time: u64,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactRow {
    pub ayah: usize,
    pub start_time: u64,
    pub end_time: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ImportEntry {
    ayah: usize,
    start_time: Option<f64>,
    start: Option<f64>,
}

pub fn set_start_clamped(
    starts: &mut Vec<Option<u64>>,
    ayah: usize,
    ms: Option<u64>,
    count: usize,
    duration_ms: u64,
) {
    if ayah < 1 || ayah > count {
        return;
    }
    if starts.is_empty() {
        starts.push(Some(0));
    }
    let Some(ms) = ms else {
        if ayah < starts.len() {
            starts[ayah] = None;
        }
        while starts.len() > 1 && starts.last() == Some(&None) {
            starts.pop();
        }
        return;
    };

    let prev = (1..ayah).rev().find_map(|i| starts.get(i).copied().flatten());
    let next = ((ayah + 1)..starts.len()).find_map(|i| starts.get(i).copied().flatten());

    let min_b = prev.map(|p| p + 10).unwrap_or(0);
    let dur_cap = if duration_ms > 0 { duration_ms } else { u64::MAX };
    let max_b = next.map(|n| n.saturating_sub(10).max(min_b)).unwrap_or(dur_cap);

    while starts.len() <= ayah {
        starts.push(None);
    }
    starts[ayah] = Some(ms.clamp(min_b, max_b));
    while starts.len() > 1 && starts.last() == Some(&None) {
        starts.pop();
    }
}

pub fn build_timeline(
    starts: &[Option<u64>],
    count: usize,
    duration_ms: u64,
    pages: Option<&[Option<u32>]>,
) -> Vec<TimelineRow> {
    let dur = if duration_ms > 0 { Some(duration_ms) } else { None };
    (0..=count)
        .map(|a| {
            let start = if a == 0 {
                Some(0)
            } else {
                starts.get(a).copied().flatten()
            };
            let next = ((a + 1)..=count).find_map(|b| starts.get(b).copied().flatten());
            let end = next.or(if a == count { dur } else { None });
            let page = if a == 0 {
                None
            } else {
                pages.and_then(|p| p.get(a - 1).copied().flatten())
            };
            TimelineRow { ayah: a, start, end, page }
        })
        .collect()
}

pub fn current_ayah(starts: &[Option<u64>], now_ms: u64) -> usize {
    let mut cur = 0;
    for (a, st) in starts.iter().enumerate().skip(1) {
        if let Some(ms) = st {
            if *ms <= now_ms {
                cur = a;
            }
        }
    }
    cur
}

pub fn next_unmarked(starts: &[Option<u64>], count: usize) -> Option<usize> {
    (1..=count).find(|&a| starts.get(a).copied().flatten().is_none())
}

pub fn export_json(
    starts: &[Option<u64>],
    count: usize,
    duration_ms: u64,
    pages: Option<&[Option<u32>]>,
    full: bool,
    minified: bool,
) -> String {
    let rows: Vec<TimelineRow> = build_timeline(starts, count, duration_ms, pages)
        .into_iter()
        .filter(|r| match (r.start, r.end) {
            (Some(s), Some(e)) => e >= s && !(r.ayah == 0 && e == 0),
            _ => false,
        })
        .collect();

    if full {
        let out: Vec<Mp3QuranRow> = rows
            .into_iter()
            .map(|r| Mp3QuranRow {
                ayah: r.ayah,
                polygon: None,
                start_time: r.start.unwrap(),
                end_time: r.end.unwrap(),
                x: None,
                y: None,
                page: r.page,
            })
            .collect();
        if minified {
            serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
        } else {
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| "[]".into())
        }
    } else {
        let out: Vec<CompactRow> = rows
            .into_iter()
            .map(|r| CompactRow {
                ayah: r.ayah,
                start_time: r.start.unwrap(),
                end_time: r.end.unwrap(),
            })
            .collect();
        if minified {
            serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
        } else {
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| "[]".into())
        }
    }
}

pub fn import_json(text: &str, count: usize) -> Result<(Vec<Option<u64>>, usize, usize), String> {
    let raw: Vec<ImportEntry> = serde_json::from_str(text).map_err(|e| format!("JSON غير صالح: {e}"))?;
    let mut entries: Vec<(usize, u64)> = raw
        .into_iter()
        .filter_map(|r| {
            let st = r.start_time.or(r.start)?;
            if (1..=count).contains(&r.ayah) && st.is_finite() && st >= 0.0 {
                Some((r.ayah, st.round() as u64))
            } else {
                None
            }
        })
        .collect();
    entries.sort_by_key(|(a, _)| *a);

    let mut s = vec![Some(0)];
    let mut prev = 0u64;
    let mut dropped = 0usize;
    let mut imported = 0usize;
    for (a, st) in entries {
        if st < prev {
            dropped += 1;
            continue;
        }
        while s.len() <= a {
            s.push(None);
        }
        s[a] = Some(st);
        prev = st;
        imported += 1;
    }
    Ok((s, imported, dropped))
}

fn work_path(key: &str) -> PathBuf {
    let safe: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let trimmed = if safe.len() > 140 { &safe[safe.len() - 140..] } else { &safe };
    data_dir().join("work").join(format!("{trimmed}.json"))
}

pub fn save_work(key: &str, starts: &[Option<u64>]) {
    let p = work_path(key);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(starts) {
        let _ = std::fs::write(p, json);
    }
}

pub fn load_work(key: &str, count: usize) -> Vec<Option<u64>> {
    let p = work_path(key);
    if let Ok(raw) = std::fs::read_to_string(p) {
        if let Ok(mut v) = serde_json::from_str::<Vec<Option<u64>>>(&raw) {
            v.truncate(count + 1);
            if v.is_empty() {
                v.push(Some(0));
            }
            return v;
        }
    }
    vec![Some(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_clamping() {
        let mut s = vec![Some(0)];
        set_start_clamped(&mut s, 1, Some(0), 7, 20000);
        assert_eq!(s[1], Some(0));
        set_start_clamped(&mut s, 3, Some(5000), 7, 20000);
        set_start_clamped(&mut s, 2, Some(6000), 7, 20000);
        assert_eq!(s[2], Some(4990));
        set_start_clamped(&mut s, 2, Some(0), 7, 20000);
        assert_eq!(s[2], Some(10));
    }

    #[test]
    fn import_drops_out_of_order() {
        let txt = r#"[{"ayah":1,"start_time":1000},{"ayah":2,"start_time":500},{"ayah":3,"start_time":2000}]"#;
        let (s, imp, drop) = import_json(txt, 7).unwrap();
        assert_eq!(imp, 2);
        assert_eq!(drop, 1);
        assert_eq!(s[1], Some(1000));
        assert_eq!(s[2], None);
        assert_eq!(s[3], Some(2000));
    }

    #[test]
    fn export_json_minified_and_compact() {
        let s = vec![Some(0), Some(100), Some(1200)];
        let pretty = export_json(&s, 2, 2500, None, false, false);
        let mini = export_json(&s, 2, 2500, None, false, true);
        assert!(mini.len() < pretty.len());
        assert!(!mini.contains('\n'));
        let (parsed, imp, dropped) = import_json(&mini, 2).unwrap();
        assert_eq!((imp, dropped), (2, 0));
        assert_eq!(parsed[1], Some(100));
        assert_eq!(parsed[2], Some(1200));
    }
}

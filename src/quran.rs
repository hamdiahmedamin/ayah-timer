use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

pub const API_BASE: &str = "https://www.mp3quran.net/api/v3";

pub const COUNTS_HAFS: [usize; 114] = [
    7, 286, 200, 176, 120, 165, 206, 75, 129, 109, 123, 111, 43, 52, 99, 128, 111, 110, 98, 135,
    112, 78, 118, 64, 77, 227, 93, 88, 69, 60, 34, 30, 73, 54, 45, 83, 182, 88, 75, 85, 54, 53, 89,
    59, 37, 35, 38, 29, 18, 45, 60, 49, 62, 55, 78, 96, 29, 22, 24, 13, 14, 11, 11, 18, 12, 12, 30,
    52, 52, 44, 28, 28, 20, 56, 40, 31, 50, 40, 46, 42, 29, 19, 36, 25, 22, 17, 19, 26, 30, 20, 15,
    21, 11, 8, 8, 19, 5, 8, 8, 11, 11, 8, 3, 9, 5, 4, 7, 3, 6, 3, 5, 4, 5, 6,
];

pub const COUNTS_MADANI: [usize; 114] = [
    7, 285, 200, 175, 122, 167, 206, 76, 130, 109, 121, 111, 44, 54, 99, 128, 110, 105, 99, 134,
    111, 76, 119, 62, 77, 226, 95, 88, 69, 59, 33, 30, 73, 54, 46, 82, 182, 86, 72, 84, 53, 50, 89,
    56, 36, 34, 39, 29, 18, 45, 60, 47, 61, 55, 77, 99, 28, 21, 24, 13, 14, 11, 11, 18, 12, 12, 31,
    52, 52, 44, 30, 28, 18, 55, 39, 31, 50, 40, 45, 42, 29, 19, 36, 25, 22, 17, 19, 26, 32, 20, 15,
    21, 11, 8, 8, 20, 5, 8, 9, 11, 10, 8, 3, 9, 5, 5, 6, 3, 6, 3, 5, 4, 5, 6,
];

pub const SURAHS: [&str; 114] = [
    "الفاتحة", "البقرة", "آل عمران", "النساء", "المائدة", "الأنعام", "الأعراف", "الأنفال",
    "التوبة", "يونس", "هود", "يوسف", "الرعد", "إبراهيم", "الحجر", "النحل", "الإسراء", "الكهف",
    "مريم", "طه", "الأنبياء", "الحج", "المؤمنون", "النور", "الفرقان", "الشعراء", "النمل",
    "القصص", "العنكبوت", "الروم", "لقمان", "السجدة", "الأحزاب", "سبأ", "فاطر", "يس",
    "الصافات", "ص", "الزمر", "غافر", "فصلت", "الشورى", "الزخرف", "الدخان", "الجاثية",
    "الأحقاف", "محمد", "الفتح", "الحجرات", "ق", "الذاريات", "الطور", "النجم", "القمر",
    "الرحمن", "الواقعة", "الحديد", "المجادلة", "الحشر", "الممتحنة", "الصف", "الجمعة",
    "المنافقون", "التغابن", "الطلاق", "التحريم", "الملك", "القلم", "الحاقة", "المعارج",
    "نوح", "الجن", "المزمل", "المدثر", "القيامة", "الإنسان", "المرسلات", "النبأ", "النازعات",
    "عبس", "التكوير", "الانفطار", "المطففين", "الانشقاق", "البروج", "الطارق", "الأعلى",
    "الغاشية", "الفجر", "البلد", "الشمس", "الليل", "الضحى", "الشرح", "التين", "العلق", "القدر",
    "البينة", "الزلزلة", "العاديات", "القارعة", "التكاثر", "العصر", "الهمزة", "الفيل", "قريش",
    "الماعون", "الكوثر", "الكافرون", "النصر", "المسد", "الإخلاص", "الفلق", "الناس",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counting {
    Hafs,
    Madani,
}

impl Counting {
    pub fn count_for(self, surah: usize) -> usize {
        let idx = surah.saturating_sub(1).min(113);
        match self {
            Counting::Hafs => COUNTS_HAFS[idx],
            Counting::Madani => COUNTS_MADANI[idx],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Counting::Hafs => "الكوفي (حفص، شعبة)",
            Counting::Madani => "المدني (ورش، قالون)",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Counting::Hafs => "hafs",
            Counting::Madani => "madani",
        }
    }
}

impl std::fmt::Display for Counting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Riwayah {
    All,
    Hafs,
    Warsh,
    Qalun,
}

impl Riwayah {
    pub fn label(self) -> &'static str {
        match self {
            Riwayah::All => "الكل",
            Riwayah::Hafs => "حفص",
            Riwayah::Warsh => "ورش",
            Riwayah::Qalun => "قالون",
        }
    }

    pub fn matches(self, moshaf_name: &str) -> bool {
        match self {
            Riwayah::All => true,
            Riwayah::Hafs => moshaf_name.contains("حفص"),
            Riwayah::Warsh => moshaf_name.contains("ورش"),
            Riwayah::Qalun => moshaf_name.contains("قالون"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextEdition {
    Hafs,
    Warsh,
    Qalun,
}

impl TextEdition {
    pub fn ar_name(self) -> &'static str {
        match self {
            TextEdition::Hafs => "مصحف حفص",
            TextEdition::Warsh => "مصحف ورش",
            TextEdition::Qalun => "مصحف قالون",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reciter {
    #[allow(dead_code)]
    pub id: u64,
    pub name: String,
    pub moshaf: Vec<Moshaf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Moshaf {
    #[allow(dead_code)]
    pub id: u64,
    pub name: String,
    pub server: String,
    #[serde(default)]
    pub surah_total: usize,
    #[serde(default)]
    pub surah_list: String,
}

impl Moshaf {
    pub fn available_surahs(&self) -> Vec<usize> {
        self.surah_list
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .filter(|&n| (1..=114).contains(&n))
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimedRead {
    pub id: u64,
    pub folder_url: String,
}

#[derive(Debug, Clone)]
pub struct SurahText {
    pub texts: Vec<String>,
    pub pages: Vec<Option<u32>>,
}

#[derive(Debug, Clone, Default)]
pub struct LoadedFonts {
    pub ui_family: Option<String>,
    pub quran_hafs_family: Option<String>,
    pub quran_warsh_family: Option<String>,
    pub quran_qalun_family: Option<String>,
    pub font_bytes: Vec<Vec<u8>>,
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("ayah-timer")
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("ayah-timer")
}

pub fn norm_url(u: &str) -> String {
    u.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string()
}

pub fn fmt_ms(ms: u64) -> String {
    let s = ms as f64 / 1000.0;
    let m = (s / 60.0).floor() as u64;
    let rem = s - (m as f64) * 60.0;
    format!("{m}:{rem:05.2}")
}

pub fn ar_digits(n: usize) -> String {
    const DIGS: [char; 10] = ['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'];
    n.to_string()
        .chars()
        .map(|c| c.to_digit(10).map(|d| DIGS[d as usize]).unwrap_or(c))
        .collect()
}

pub fn strip_basmala_for_align(surah: usize, verse1: &str) -> String {
    let clean = verse1.trim_start_matches('\u{FEFF}');
    if surah != 1 && surah != 9 && clean.starts_with("بِسْمِ") {
        let parts: Vec<&str> = clean.splitn(5, ' ').collect();
        if parts.len() == 5 {
            return parts[4].to_string();
        }
    }
    clean.to_string()
}

pub fn normalize_for_vocab(text: &str, vocab: &HashMap<String, usize>) -> Vec<usize> {
    let mut out = Vec::new();
    let space_tok = *vocab.get("|").unwrap_or(&0);
    let mut prev_space = true;

    for ch in text.chars() {
        let code = ch as u32;
        // Strip harakat, Quranic annotation marks, tatweel
        if (0x0610..=0x061A).contains(&code)
            || (0x064B..=0x065F).contains(&code)
            || code == 0x0670
            || (0x06D6..=0x06ED).contains(&code)
            || code == 0x0640
        {
            continue;
        }
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(space_tok);
                prev_space = true;
            }
            continue;
        }
        let mapped = match ch {
            'ٱ' | 'أ' | 'إ' | 'آ' => 'ا',
            'ى' | 'ے' => 'ي',
            'ة' => 'ه',
            other => other,
        };
        let mut buf = [0u8; 4];
        let key = mapped.encode_utf8(&mut buf);
        if let Some(&id) = vocab.get(key) {
            out.push(id);
            prev_space = false;
        }
    }
    while out.last() == Some(&space_tok) {
        out.pop();
    }
    out
}

pub fn normalize_search(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let code = ch as u32;
        if matches!(ch, '\u{200E}' | '\u{2067}' | '\u{2069}')
            || (0x0610..=0x061A).contains(&code)
            || (0x064B..=0x065F).contains(&code)
            || code == 0x0670
            || (0x06D6..=0x06ED).contains(&code)
            || code == 0x0640
        {
            continue;
        }
        let mapped = match ch {
            'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
            'ة' => 'ه',
            'ى' | 'ئ' => 'ي',
            'ؤ' => 'و',
            '٠'..='٩' => char::from_u32('0' as u32 + (code - '٠' as u32)).unwrap_or(ch),
            _ => ch,
        };
        for lc in mapped.to_lowercase() {
            out.push(lc);
        }
    }
    out
}

pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("ayah-timer-rust/0.1")
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}

pub async fn fetch_reciters_and_timed_reads() -> Result<(Vec<Reciter>, HashMap<String, u64>)> {
    #[derive(Deserialize)]
    struct RecWrap {
        reciters: Vec<Reciter>,
    }
    let client = http_client()?;
    let (rec_res, reads_res) = tokio::join!(
        client.get(format!("{API_BASE}/reciters?language=ar")).send(),
        client.get(format!("{API_BASE}/ayat_timing/reads")).send(),
    );

    let mut reciters: Vec<Reciter> = rec_res?.error_for_status()?.json::<RecWrap>().await?.reciters;
    reciters.sort_by(|a, b| a.name.cmp(&b.name));

    let mut timed = HashMap::new();
    if let Ok(resp) = reads_res {
        if let Ok(reads) = resp.json::<Vec<TimedRead>>().await {
            for r in reads {
                timed.insert(norm_url(&r.folder_url), r.id);
            }
        }
    }
    Ok((reciters, timed))
}

pub async fn fetch_surah_text(surah: usize, edition: TextEdition) -> Result<SurahText> {
    let dir = cache_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let client = http_client()?;

    match edition {
        TextEdition::Hafs => {
            #[derive(Deserialize)]
            struct ApiResp {
                data: ApiData,
            }
            #[derive(Deserialize)]
            struct ApiData {
                ayahs: Vec<ApiAyah>,
            }
            #[derive(Deserialize)]
            struct ApiAyah {
                text: String,
                page: Option<u32>,
            }

            let cache_path = dir.join(format!("hafs_{surah:03}.json"));
            let bytes = if cache_path.exists() {
                tokio::fs::read(&cache_path).await?
            } else {
                let url = format!("https://api.alquran.cloud/v1/surah/{surah}/quran-uthmani");
                let b = client.get(url).send().await?.error_for_status()?.bytes().await?.to_vec();
                let _ = tokio::fs::write(&cache_path, &b).await;
                b
            };
            let parsed: ApiResp = serde_json::from_slice(&bytes).context("invalid alquran.cloud JSON")?;
            let texts = parsed
                .data
                .ayahs
                .iter()
                .map(|a| a.text.trim_start_matches('\u{FEFF}').to_string())
                .collect();
            let pages = parsed.data.ayahs.iter().map(|a| a.page).collect();
            Ok(SurahText { texts, pages })
        }
        TextEdition::Warsh | TextEdition::Qalun => {
            #[derive(Deserialize)]
            struct KfgRow {
                sura_no: usize,
                aya_text: String,
                #[serde(default)]
                page: Option<serde_json::Value>,
            }
            let (name, url) = match edition {
                TextEdition::Warsh => (
                    "warsh_v10.json",
                    "https://cdn.jsdelivr.net/gh/thetruetruth/quran-data-kfgqpc@main/warsh/data/warshData_v10.json",
                ),
                TextEdition::Qalun => (
                    "qalun_v10.json",
                    "https://cdn.jsdelivr.net/gh/thetruetruth/quran-data-kfgqpc@main/qaloon/data/QaloonData_v10.json",
                ),
                TextEdition::Hafs => unreachable!(),
            };
            let cache_path = dir.join(name);
            let bytes = if cache_path.exists() {
                tokio::fs::read(&cache_path).await?
            } else {
                let b = client.get(url).send().await?.error_for_status()?.bytes().await?.to_vec();
                let _ = tokio::fs::write(&cache_path, &b).await;
                b
            };
            let rows: Vec<KfgRow> = serde_json::from_slice(&bytes).context("invalid KFGQPC JSON")?;
            let mut texts = Vec::new();
            let mut pages = Vec::new();
            for r in rows {
                if r.sura_no != surah {
                    continue;
                }
                let cleaned = r
                    .aya_text
                    .trim_end_matches(|c: char| c.is_whitespace() || c == '\u{00A0}' || ('٠'..='٩').contains(&c))
                    .to_string();
                let pg = match r.page {
                    Some(serde_json::Value::Number(n)) => n.as_u64().map(|v| v as u32),
                    Some(serde_json::Value::String(s)) => s.trim().parse::<u32>().ok(),
                    _ => None,
                };
                texts.push(cleaned);
                pages.push(pg);
            }
            Ok(SurahText { texts, pages })
        }
    }
}

pub async fn fetch_published_timings(surah: usize, read_id: u64) -> Result<String> {
    let url = format!("{API_BASE}/ayat_timing?surah={surah}&read={read_id}");
    let text = http_client()?.get(url).send().await?.error_for_status()?.text().await?;
    Ok(text)
}

/// Parse the font family name (nameID 1) from a TTF/OTF `name` table.
fn parse_ttf_family(data: &[u8]) -> Option<String> {
    if data.len() < 12 {
        return None;
    }
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    for i in 0..num_tables {
        let off = 12 + i * 16;
        if off + 16 > data.len() {
            break;
        }
        if &data[off..off + 4] == b"name" {
            let t_off = u32::from_be_bytes([data[off + 8], data[off + 9], data[off + 10], data[off + 11]]) as usize;
            if t_off + 6 > data.len() {
                return None;
            }
            let count = u16::from_be_bytes([data[t_off + 2], data[t_off + 3]]) as usize;
            let str_off = t_off + u16::from_be_bytes([data[t_off + 4], data[t_off + 5]]) as usize;
            for j in 0..count {
                let rec = t_off + 6 + j * 12;
                if rec + 12 > data.len() {
                    break;
                }
                let platform = u16::from_be_bytes([data[rec], data[rec + 1]]);
                let name_id = u16::from_be_bytes([data[rec + 6], data[rec + 7]]);
                let len = u16::from_be_bytes([data[rec + 8], data[rec + 9]]) as usize;
                let s_off = str_off + u16::from_be_bytes([data[rec + 10], data[rec + 11]]) as usize;
                if name_id == 1 && s_off + len <= data.len() {
                    let slice = &data[s_off..s_off + len];
                    if platform == 3 || platform == 0 {
                        let u16s: Vec<u16> = slice
                            .chunks_exact(2)
                            .map(|c| u16::from_be_bytes([c[0], c[1]]))
                            .collect();
                        if let Ok(s) = String::from_utf16(&u16s) {
                            return Some(s.trim().to_string());
                        }
                    } else if let Ok(s) = std::str::from_utf8(slice) {
                        return Some(s.trim().to_string());
                    }
                }
            }
        }
    }
    None
}

async fn download_cached_font(client: &reqwest::Client, filename: &str, url: &str) -> Option<(Vec<u8>, Option<String>)> {
    let dir = cache_dir().join("fonts");
    let _ = tokio::fs::create_dir_all(&dir).await;
    let path = dir.join(filename);
    let bytes = if path.exists() {
        tokio::fs::read(&path).await.ok()?
    } else {
        let resp = client.get(url).send().await.ok()?.error_for_status().ok()?;
        let b = resp.bytes().await.ok()?.to_vec();
        let _ = tokio::fs::write(&path, &b).await;
        b
    };
    let fam = parse_ttf_family(&bytes);
    Some((bytes, fam))
}

pub fn load_cached_fonts_sync() -> LoadedFonts {
    let mut out = LoadedFonts::default();
    let dir = cache_dir().join("fonts");
    let read_cached = |filename: &str| -> Option<(Vec<u8>, Option<String>)> {
        let bytes = std::fs::read(dir.join(filename)).ok()?;
        let fam = parse_ttf_family(&bytes);
        Some((bytes, fam))
    };

    if let Some((b, fam)) = read_cached("IBMPlexSansArabic-Regular.ttf") {
        out.ui_family = fam;
        out.font_bytes.push(b);
    }
    if let Some((b, _)) = read_cached("IBMPlexSansArabic-SemiBold.ttf") {
        out.font_bytes.push(b);
    }
    if let Some((b, fam)) = read_cached("AmiriQuran-Regular.ttf") {
        out.quran_hafs_family = fam;
        out.font_bytes.push(b);
    }
    if let Some((b, fam)) = read_cached("warsh.10.ttf") {
        out.quran_warsh_family = fam;
        out.font_bytes.push(b);
    }
    if let Some((b, fam)) = read_cached("qaloon.10.ttf") {
        out.quran_qalun_family = fam;
        out.font_bytes.push(b);
    }
    out
}

pub async fn ensure_fonts() -> LoadedFonts {
    let mut out = LoadedFonts::default();
    let Ok(client) = http_client() else {
        return out;
    };

    let (ui_reg, ui_bold, amiri, warsh, qalun) = tokio::join!(
        download_cached_font(
            &client,
            "IBMPlexSansArabic-Regular.ttf",
            "https://github.com/google/fonts/raw/main/ofl/ibmplexsansarabic/IBMPlexSansArabic-Regular.ttf",
        ),
        download_cached_font(
            &client,
            "IBMPlexSansArabic-SemiBold.ttf",
            "https://github.com/google/fonts/raw/main/ofl/ibmplexsansarabic/IBMPlexSansArabic-SemiBold.ttf",
        ),
        download_cached_font(
            &client,
            "AmiriQuran-Regular.ttf",
            "https://github.com/google/fonts/raw/main/ofl/amiriquran/AmiriQuran-Regular.ttf",
        ),
        download_cached_font(
            &client,
            "warsh.10.ttf",
            "https://cdn.jsdelivr.net/gh/thetruetruth/quran-data-kfgqpc@main/warsh/font/warsh.10.ttf",
        ),
        download_cached_font(
            &client,
            "qaloon.10.ttf",
            "https://cdn.jsdelivr.net/gh/thetruetruth/quran-data-kfgqpc@main/qaloon/font/qaloon.10.ttf",
        ),
    );

    if let Some((b, fam)) = ui_reg {
        out.ui_family = fam;
        out.font_bytes.push(b);
    }
    if let Some((b, _)) = ui_bold {
        out.font_bytes.push(b);
    }
    if let Some((b, fam)) = amiri {
        out.quran_hafs_family = fam;
        out.font_bytes.push(b);
    }
    if let Some((b, fam)) = warsh {
        out.quran_warsh_family = fam;
        out.font_bytes.push(b);
    }
    if let Some((b, fam)) = qalun {
        out.quran_qalun_family = fam;
        out.font_bytes.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_match_114() {
        assert_eq!(COUNTS_HAFS[0], 7);
        assert_eq!(COUNTS_HAFS[1], 286);
        assert_eq!(COUNTS_MADANI[1], 285);
        assert_eq!(COUNTS_MADANI[17], 105);
    }

    #[test]
    fn arabic_normalization_works() {
        let mut vocab = HashMap::new();
        vocab.insert("|".into(), 0);
        vocab.insert("ا".into(), 4);
        vocab.insert("ل".into(), 26);
        vocab.insert("ح".into(), 9);
        vocab.insert("م".into(), 27);
        vocab.insert("د".into(), 11);
        let toks = normalize_for_vocab("ٱلْحَمْدُ", &vocab);
        assert_eq!(toks, vec![4, 26, 9, 27, 11]);
    }

    #[test]
    fn search_normalization_matches_variants() {
        assert_eq!(normalize_search("إبراهيم"), normalize_search("ابراهيم"));
        assert_eq!(normalize_search("آل عمران"), normalize_search("ال عمران"));
        assert_eq!(normalize_search("البقرة"), normalize_search("البقره"));
        assert_eq!(normalize_search("١٨. الكهف"), "18. الكهف");
    }
}

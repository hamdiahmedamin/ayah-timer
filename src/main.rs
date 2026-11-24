mod align;
mod audio;
mod export_audio;
mod quran;
mod timing;
mod waveform;

use iced::event::{self, Event};
use iced::keyboard::{self, key::Named, Key};
use iced::overlay::menu;
use iced::widget::{
    button, checkbox, column, container, mouse_area, pick_list, progress_bar, row, scrollable,
    slider, stack, text, text_editor, text_input, Space,
};
use iced::{
    alignment, border, Color, Element, Font, Length, Padding, Shadow, Subscription, Task, Theme,
    Vector,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use align::{auto_heuristic, ensure_ai_model, run_ai_alignment, AiProgress};
use audio::{analyze_audio_file, AudioAnalysis, AudioPlayer};
use export_audio::{
    compress_audio, format_bytes, split_audio_by_ayahs, AudioExportOpts, AudioFormat, NamingMode,
};
use quran::{
    ar_digits, cache_dir, ensure_fonts, fetch_published_timings, fetch_reciters_and_timed_reads,
    fetch_surah_text, fmt_ms, http_client, load_cached_fonts_sync, norm_url, normalize_search,
    Counting, LoadedFonts, Moshaf, Reciter, Riwayah, SurahText, TextEdition, SURAHS,
};
use timing::{
    build_timeline, current_ayah, export_json, import_json, load_work, next_unmarked, save_work,
    set_start_clamped,
};
use waveform::{view_waveform, WaveColors, WaveEvent, WaveformProgram};

fn main() -> iced::Result {
    let preloaded = load_cached_fonts_sync();
    let mut app_builder = iced::application("Ayah Timer", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size((1340.0, 860.0));
    for bytes in &preloaded.font_bytes {
        app_builder = app_builder.font(bytes.clone());
    }
    if let Some(fam) = preloaded.ui_family.as_deref() {
        app_builder = app_builder.default_font(intern_font(fam));
    }
    app_builder.run_with(move || App::new(preloaded))
}

fn intern_font(name: &str) -> Font {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().unwrap();
    let leaked: &'static str = if let Some(&s) = g.get(name) {
        s
    } else {
        let s: &'static str = Box::leak(name.to_string().into_boxed_str());
        g.insert(name.to_string(), s);
        s
    };
    Font::with_name(leaked)
}

/// Wrap Arabic text in LRM + RLI..PDI so cosmic-text keeps paragraph level LTR
/// (preventing `start_x = line_width` displacement in iced 0.13) while shaping
/// the interior Arabic run RTL via HarfBuzz/rustybuzz.
fn rtl(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else {
        format!("\u{200E}\u{2067}{s}\u{2069}")
    }
}

fn ar<'a>(s: impl AsRef<str>, font: Font) -> text::Text<'a> {
    text(rtl(s.as_ref()))
        .font(font)
        .shaping(text::Shaping::Advanced)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorKind {
    Reciter,
    Moshaf,
    Surah,
    Counting,
}

fn search_input_id() -> text_input::Id {
    text_input::Id::new("selector_search")
}

fn mushaf_scroll_id() -> scrollable::Id {
    scrollable::Id::new("mushaf_scroll")
}

fn table_scroll_id() -> scrollable::Id {
    scrollable::Id::new("table_scroll")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArOption {
    idx: usize,
    raw_label: String,
    search_key: String,
}

impl ArOption {
    fn new(idx: usize, raw: &str) -> Self {
        Self {
            idx,
            raw_label: raw.to_string(),
            search_key: normalize_search(raw),
        }
    }
}

impl std::fmt::Display for ArOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&rtl(&self.raw_label))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceTab {
    Mp3Quran,
    File,
    Url,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectorTab {
    Table,
    Mushaf,
    Io,
    Audio,
}

#[derive(Debug, Clone, Copy)]
struct Palette {
    bg: Color,
    panel: Color,
    panel2: Color,
    ink: Color,
    muted: Color,
    line: Color,
    accent: Color,
    accent_hover: Color,
    accent_ink: Color,
    soft: Color,
    wave: Color,
    wave_played: Color,
    mark: Color,
    now: Color,
    verse: Color,
    danger: Color,
    danger_soft: Color,
}

impl Palette {
    fn light() -> Self {
        Self {
            bg: Color::from_rgb8(0xf4, 0xf1, 0xe8),
            panel: Color::from_rgb8(0xff, 0xfd, 0xf8),
            panel2: Color::from_rgb8(0xf7, 0xf3, 0xe8),
            ink: Color::from_rgb8(0x1a, 0x22, 0x20),
            muted: Color::from_rgb8(0x5d, 0x6b, 0x67),
            line: Color::from_rgb8(0xdf, 0xd8, 0xc7),
            accent: Color::from_rgb8(0x1f, 0x64, 0x54),
            accent_hover: Color::from_rgb8(0x17, 0x4f, 0x42),
            accent_ink: Color::WHITE,
            soft: Color::from_rgb8(0xe2, 0xef, 0xe9),
            wave: Color::from_rgb8(0x9c, 0xb9, 0xaf),
            wave_played: Color::from_rgb8(0x1f, 0x64, 0x54),
            mark: Color::from_rgb8(0xc8, 0x4b, 0x14),
            now: Color::from_rgb8(0xc9, 0x92, 0x0e),
            verse: Color::from_rgba8(0x1f, 0x64, 0x54, 0.11),
            danger: Color::from_rgb8(0xb4, 0x23, 0x18),
            danger_soft: Color::from_rgb8(0xfc, 0xec, 0xea),
        }
    }

    fn dark() -> Self {
        Self {
            bg: Color::from_rgb8(0x0f, 0x14, 0x13),
            panel: Color::from_rgb8(0x16, 0x1e, 0x1d),
            panel2: Color::from_rgb8(0x1c, 0x26, 0x24),
            ink: Color::from_rgb8(0xe7, 0xec, 0xe9),
            muted: Color::from_rgb8(0x93, 0xa3, 0x9e),
            line: Color::from_rgb8(0x27, 0x33, 0x31),
            accent: Color::from_rgb8(0x6f, 0xd1, 0xb6),
            accent_hover: Color::from_rgb8(0x88, 0xdd, 0xc6),
            accent_ink: Color::from_rgb8(0x0b, 0x14, 0x12),
            soft: Color::from_rgb8(0x1b, 0x2c, 0x28),
            wave: Color::from_rgb8(0x38, 0x52, 0x4d),
            wave_played: Color::from_rgb8(0x6f, 0xd1, 0xb6),
            mark: Color::from_rgb8(0xf0, 0x8a, 0x5d),
            now: Color::from_rgb8(0xec, 0xc4, 0x5a),
            verse: Color::from_rgba8(0x6f, 0xd1, 0xb6, 0.12),
            danger: Color::from_rgb8(0xf9, 0x70, 0x66),
            danger_soft: Color::from_rgb8(0x33, 0x18, 0x18),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    FontsLoaded(LoadedFonts),
    FontRegistered(Result<(), iced::font::Error>),
    RecitersLoaded(Result<(Vec<Reciter>, HashMap<String, u64>), String>),
    SurahTextLoaded(usize, TextEdition, Result<SurahText, String>),
    SourceTabChanged(SourceTab),
    RiwayahChanged(Riwayah),
    OpenSelector(SelectorKind),
    CloseSelector,
    SelectorQueryChanged(String),
    ClearSelectorQuery,
    SelectorSubmit,
    SelectorPick(SelectorKind, usize),
    Noop,
    PickAudioFile,
    AudioFilePicked(Option<PathBuf>),
    UrlChanged(String),
    LoadAudio,
    AudioAnalyzed(u64, PathBuf, String, Result<AudioAnalysis, String>),
    Tick,
    TogglePlay,
    Seek(u64),
    RatePicked(String),
    ReactionChanged(String),
    SnapToggled(bool),
    MarkNext,
    Undo,
    ResetMarks,
    AutoHeuristic,
    ToggleAiAlign,
    DismissAiProgress,
    AiFinished(Result<Vec<u64>, String>),
    Wave(WaveEvent),
    ZoomChanged(f32),
    ZoomStep(f32),
    ScrollChanged(f32),
    FollowToggled(bool),
    InspectorChanged(InspectorTab),
    PlayAyah(usize),
    SetAyahToNow(usize),
    NudgeAyah(usize, i64),
    ClearAyah(usize),
    TableInputChanged(usize, String),
    TableInputSubmit(usize),
    FullFieldsToggled(bool),
    JsonMinifiedToggled(bool),
    MushafFontDelta(f32),
    JumpToCurrentAyah,
    StepAyah(i32),
    ExportFormatPicked(AudioFormat),
    ExportBitratePicked(u32),
    ExportMonoToggled(bool),
    ExportIntroToggled(bool),
    ExportNamingPicked(NamingMode),
    StartSplitAudio,
    SplitDirPicked(Option<PathBuf>),
    StartCompressAudio,
    CompressDestPicked(Option<PathBuf>),
    CancelExport,
    ExportFinished(Result<String, String>),
    JsonAction(text_editor::Action),
    ImportFromText,
    ImportFromFile,
    ImportFileLoaded(Option<String>),
    DownloadJson,
    CopyJson,
    FetchPublishedTiming,
    PublishedTimingLoaded(Result<String, String>),
    ToggleTheme,
    KeyPressed(Key, keyboard::Modifiers),
}

struct App {
    dark: bool,
    fonts: LoadedFonts,
    ui_font: Font,
    hafs_font: Font,
    warsh_font: Font,
    qalun_font: Font,

    player: AudioPlayer,
    source_tab: SourceTab,
    riwayah: Riwayah,
    inspector_tab: InspectorTab,

    all_reciters: Vec<Reciter>,
    reciters: Vec<Reciter>,
    timed_reads: HashMap<String, u64>,
    selected_reciter: usize,
    selected_moshaf: usize,

    reciter_opts: Vec<ArOption>,
    moshaf_opts: Vec<ArOption>,
    surah_opts: Vec<ArOption>,
    counting_opts: Vec<ArOption>,
    rate_opts: Vec<String>,
    open_selector: Option<SelectorKind>,
    selector_query: String,
    selector_highlight: usize,

    surah: usize,
    counting: Counting,
    file_path: Option<PathBuf>,
    url_input: String,
    audio_path: Option<PathBuf>,
    audio_label: String,
    load_seq: u64,
    load_cancel: Arc<AtomicBool>,

    surah_text: Option<SurahText>,
    starts: Vec<Option<u64>>,
    duration_ms: u64,
    peaks: Vec<f32>,
    energy: Vec<f32>,

    zoom: f32,
    scroll_frac: f32,
    follow: bool,
    now_ms: u64,
    last_cur: usize,

    rate_str: String,
    reaction_str: String,
    reaction_ms: u64,
    snap: bool,
    full_fields: bool,
    json_minified: bool,
    mushaf_font_size: f32,
    export_opts: AudioExportOpts,
    export_running: bool,

    table_inputs: HashMap<usize, String>,
    json_content: text_editor::Content,
    status: String,
    status_err: bool,

    ai_running: bool,
    ai_cancel: Arc<AtomicBool>,
    ai_progress_slot: Arc<Mutex<Option<AiProgress>>>,
    ai_progress: Option<AiProgress>,
}

impl App {
    fn new(preloaded: LoadedFonts) -> (Self, Task<Message>) {
        let need_fonts = preloaded.ui_family.is_none()
            || preloaded.quran_hafs_family.is_none()
            || preloaded.quran_warsh_family.is_none()
            || preloaded.quran_qalun_family.is_none();

        let mut app = Self {
            dark: false,
            fonts: preloaded,
            ui_font: Font::DEFAULT,
            hafs_font: Font::DEFAULT,
            warsh_font: Font::DEFAULT,
            qalun_font: Font::DEFAULT,
            player: AudioPlayer::new(),
            source_tab: SourceTab::Mp3Quran,
            riwayah: Riwayah::All,
            inspector_tab: InspectorTab::Table,
            all_reciters: Vec::new(),
            reciters: Vec::new(),
            timed_reads: HashMap::new(),
            selected_reciter: 0,
            selected_moshaf: 0,
            reciter_opts: vec![ArOption::new(0, "جارٍ التحميل…")],
            moshaf_opts: vec![],
            surah_opts: Self::surah_options(None),
            counting_opts: vec![
                ArOption::new(0, Counting::Hafs.label()),
                ArOption::new(1, Counting::Madani.label()),
            ],
            rate_opts: ["0.75", "1", "1.25", "1.5", "2"]
                .into_iter()
                .map(String::from)
                .collect(),
            open_selector: None,
            selector_query: String::new(),
            selector_highlight: 0,
            surah: 1,
            counting: Counting::Hafs,
            file_path: None,
            url_input: String::new(),
            audio_path: None,
            audio_label: String::new(),
            load_seq: 0,
            load_cancel: Arc::new(AtomicBool::new(false)),
            surah_text: None,
            starts: vec![Some(0)],
            duration_ms: 0,
            peaks: Vec::new(),
            energy: Vec::new(),
            zoom: 1.0,
            scroll_frac: 0.0,
            follow: true,
            now_ms: 0,
            last_cur: 0,
            rate_str: "1".into(),
            reaction_str: "250".into(),
            reaction_ms: 250,
            snap: true,
            full_fields: true,
            json_minified: false,
            mushaf_font_size: 26.0,
            export_opts: AudioExportOpts::default(),
            export_running: false,
            table_inputs: HashMap::new(),
            json_content: text_editor::Content::new(),
            status: String::new(),
            status_err: false,
            ai_running: false,
            ai_cancel: Arc::new(AtomicBool::new(false)),
            ai_progress_slot: Arc::new(Mutex::new(None)),
            ai_progress: None,
        };
        app.sync_cached_fonts();
        app.restore_work();

        let mut tasks = vec![
            Task::perform(
                async { fetch_reciters_and_timed_reads().await.map_err(|e| e.to_string()) },
                Message::RecitersLoaded,
            ),
            app.load_text_task(),
        ];
        if need_fonts {
            tasks.push(Task::perform(ensure_fonts(), Message::FontsLoaded));
        }
        (app, Task::batch(tasks))
    }

    fn sync_cached_fonts(&mut self) {
        self.ui_font = self
            .fonts
            .ui_family
            .as_deref()
            .map(intern_font)
            .unwrap_or(Font::DEFAULT);
        self.hafs_font = self
            .fonts
            .quran_hafs_family
            .as_deref()
            .map(intern_font)
            .unwrap_or(self.ui_font);
        self.warsh_font = self
            .fonts
            .quran_warsh_family
            .as_deref()
            .map(intern_font)
            .unwrap_or(self.hafs_font);
        self.qalun_font = self
            .fonts
            .quran_qalun_family
            .as_deref()
            .map(intern_font)
            .unwrap_or(self.hafs_font);
    }

    fn surah_options(allowed: Option<&[usize]>) -> Vec<ArOption> {
        SURAHS
            .iter()
            .enumerate()
            .filter(|(i, _)| allowed.map(|a| a.contains(&(i + 1))).unwrap_or(true))
            .map(|(i, name)| ArOption::new(i + 1, &format!("{}. {name}", i + 1)))
            .collect()
    }

    fn pal(&self) -> Palette {
        if self.dark {
            Palette::dark()
        } else {
            Palette::light()
        }
    }

    fn theme(&self) -> Theme {
        let p = self.pal();
        Theme::custom(
            if self.dark { "AyahDark" } else { "AyahLight" }.into(),
            iced::theme::Palette {
                background: p.bg,
                text: p.ink,
                primary: p.accent,
                success: Color::from_rgb8(0x16, 0xa3, 0x4a),
                danger: p.danger,
            },
        )
    }

    fn quran_font(&self) -> Font {
        match self.text_edition() {
            TextEdition::Warsh => self.warsh_font,
            TextEdition::Qalun => self.qalun_font,
            TextEdition::Hafs => self.hafs_font,
        }
    }

    fn count(&self) -> usize {
        self.counting.count_for(self.surah)
    }

    fn current_moshaf(&self) -> Option<&Moshaf> {
        self.reciters
            .get(self.selected_reciter)
            .and_then(|r| r.moshaf.get(self.selected_moshaf))
    }

    fn text_edition(&self) -> TextEdition {
        if self.counting == Counting::Hafs {
            return TextEdition::Hafs;
        }
        let m_name = self.current_moshaf().map(|m| m.name.as_str()).unwrap_or("");
        if m_name.contains("قالون") || self.riwayah == Riwayah::Qalun {
            TextEdition::Qalun
        } else {
            TextEdition::Warsh
        }
    }

    fn audio_url(&self) -> Option<String> {
        let s = format!("{:03}", self.surah);
        match self.source_tab {
            SourceTab::Mp3Quran => self.current_moshaf().map(|m| format!("{}{s}.mp3", m.server)),
            SourceTab::Url => {
                let u = self.url_input.trim();
                if u.is_empty() {
                    None
                } else {
                    Some(u.to_string())
                }
            }
            SourceTab::File => None,
        }
    }

    fn store_key(&self) -> String {
        let src = match self.source_tab {
            SourceTab::File => self
                .file_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            _ => self.audio_url().unwrap_or_default(),
        };
        format!("ayah-timer:{src}:{}:{}", self.surah, self.counting.as_str())
    }

    fn restore_work(&mut self) {
        let c = self.count();
        self.starts = load_work(&self.store_key(), c);
        self.sync_json_and_inputs();
    }

    fn save_and_sync(&mut self) {
        save_work(&self.store_key(), &self.starts);
        self.sync_json_and_inputs();
    }

    fn sync_json_and_inputs(&mut self) {
        let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
        let j = export_json(
            &self.starts,
            self.count(),
            self.duration_ms,
            pages,
            self.full_fields,
            self.json_minified,
        );
        self.json_content = text_editor::Content::with_text(&j);
        self.table_inputs.clear();
        for a in 1..=self.count() {
            let v = self
                .starts
                .get(a)
                .copied()
                .flatten()
                .map(|ms| ms.to_string())
                .unwrap_or_default();
            self.table_inputs.insert(a, v);
        }
    }

    fn load_text_task(&self) -> Task<Message> {
        let surah = self.surah;
        let ed = self.text_edition();
        Task::perform(
            async move { fetch_surah_text(surah, ed).await.map_err(|e| e.to_string()) },
            move |res| Message::SurahTextLoaded(surah, ed, res),
        )
    }

    fn clear_audio(&mut self) {
        self.load_seq += 1;
        self.load_cancel.store(true, Ordering::Relaxed);
        self.ai_cancel.store(true, Ordering::Relaxed);
        self.ai_running = false;
        self.ai_progress = None;
        self.player.clear();
        self.audio_path = None;
        self.audio_label.clear();
        self.peaks.clear();
        self.energy.clear();
        self.duration_ms = 0;
        self.now_ms = 0;
        self.zoom = 1.0;
        self.scroll_frac = 0.0;
    }

    fn fill_reciters(&mut self) {
        self.reciters = self
            .all_reciters
            .iter()
            .filter(|r| r.moshaf.iter().any(|m| self.riwayah.matches(&m.name)))
            .cloned()
            .collect();
        self.reciter_opts = self
            .reciters
            .iter()
            .enumerate()
            .map(|(i, r)| ArOption::new(i, &r.name))
            .collect();

        let pick = match self.riwayah {
            Riwayah::Warsh => self
                .reciters
                .iter()
                .position(|r| r.name.contains("الكوشي"))
                .or_else(|| {
                    self.reciters
                        .iter()
                        .position(|r| r.moshaf.iter().any(|m| m.name.contains("ورش") && m.surah_total == 114))
                }),
            Riwayah::Qalun => self.reciters.iter().position(|r| r.name.contains("الحصري")),
            _ => self.reciters.iter().position(|r| r.name.contains("العفاسي")),
        }
        .unwrap_or(0);

        self.selected_reciter = pick.min(self.reciters.len().saturating_sub(1));
        let _ = self.fill_moshaf();
    }

    fn fill_moshaf(&mut self) -> Task<Message> {
        let Some(r) = self.reciters.get(self.selected_reciter) else {
            self.moshaf_opts = vec![];
            return Task::none();
        };
        self.moshaf_opts = r
            .moshaf
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let timed = self.timed_reads.contains_key(&norm_url(&m.server));
                let raw = if timed {
                    format!("{} · له توقيت", m.name)
                } else {
                    m.name.clone()
                };
                ArOption::new(i, &raw)
            })
            .collect();

        let idx = r
            .moshaf
            .iter()
            .position(|m| {
                if self.riwayah == Riwayah::All {
                    m.name.contains("حفص")
                } else {
                    self.riwayah.matches(&m.name)
                }
            })
            .unwrap_or(0);
        self.selected_moshaf = idx;
        self.picked_moshaf()
    }

    fn picked_moshaf(&mut self) -> Task<Message> {
        let Some(m) = self.current_moshaf().cloned() else {
            return Task::none();
        };
        if self.source_tab == SourceTab::Mp3Quran {
            self.clear_audio();
            self.status.clear();
            self.status_err = false;
        }
        let madani = m.name.contains("ورش") || m.name.contains("قالون");
        self.counting = if madani { Counting::Madani } else { Counting::Hafs };

        let avail = m.available_surahs();
        if self.source_tab == SourceTab::Mp3Quran && !avail.is_empty() {
            self.surah_opts = Self::surah_options(Some(&avail));
            if !avail.contains(&self.surah) {
                self.surah = avail[0];
            }
        } else {
            self.surah_opts = Self::surah_options(None);
        }

        self.surah_text = None;
        self.restore_work();
        self.load_text_task()
    }

    fn reveal_now(&mut self) {
        if self.duration_ms == 0 || self.zoom <= 1.0 {
            self.scroll_frac = 0.0;
            return;
        }
        let frac = (self.now_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0);
        let view_frac = 1.0 / self.zoom;
        let cur_left = self.scroll_frac * (1.0 - view_frac);
        if frac < cur_left + view_frac * 0.1 || frac > cur_left + view_frac * 0.85 {
            let target_left = (frac - view_frac * 0.35).clamp(0.0, 1.0 - view_frac);
            self.scroll_frac = if 1.0 - view_frac > 1e-5 {
                target_left / (1.0 - view_frac)
            } else {
                0.0
            };
        }
    }

    fn say(&mut self, msg: impl Into<String>, err: bool) {
        self.status = msg.into();
        self.status_err = err;
    }

    fn scroll_inspector_to(&self, ayah: usize) -> Task<Message> {
        match self.inspector_tab {
            InspectorTab::Table => {
                let y = (ayah as f32 * 39.0 - 140.0).max(0.0);
                scrollable::scroll_to(table_scroll_id(), scrollable::AbsoluteOffset { x: 0.0, y })
            }
            InspectorTab::Mushaf => {
                let Some(st) = &self.surah_text else {
                    return Task::none();
                };
                let ed = self.text_edition();
                let mut y = 0.0f32;
                if self.surah != 9 && !(self.surah == 1 && ed == TextEdition::Hafs) {
                    y += 44.0;
                }
                let line_px = self.mushaf_font_size * 1.9;
                let chars_per_line = (380.0 / (self.mushaf_font_size * 0.52)).max(16.0);
                let mut prev_page: Option<u32> = None;
                for (i, t) in st.texts.iter().enumerate() {
                    let a = i + 1;
                    let pg = st.pages.get(i).copied().flatten();
                    if pg.is_some() && pg != prev_page {
                        prev_page = pg;
                        y += 28.0;
                    }
                    if a >= ayah {
                        break;
                    }
                    let lines = ((t.chars().count() as f32 + 6.0) / chars_per_line).ceil().max(1.0);
                    y += lines * line_px + 28.0;
                }
                scrollable::scroll_to(
                    mushaf_scroll_id(),
                    scrollable::AbsoluteOffset {
                        x: 0.0,
                        y: (y - 110.0).max(0.0),
                    },
                )
            }
            _ => Task::none(),
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::FontsLoaded(loaded) => {
                let mut tasks = Vec::new();
                if loaded.font_bytes.len() > self.fonts.font_bytes.len() {
                    for bytes in loaded.font_bytes.iter().cloned() {
                        tasks.push(iced::font::load(bytes).map(Message::FontRegistered));
                    }
                }
                self.fonts = loaded;
                self.sync_cached_fonts();
                return Task::batch(tasks);
            }
            Message::FontRegistered(_) => {}
            Message::RecitersLoaded(Ok((recs, reads))) => {
                self.all_reciters = recs;
                self.timed_reads = reads;
                self.fill_reciters();
                return self.load_text_task();
            }
            Message::RecitersLoaded(Err(_)) => {
                self.reciter_opts = vec![ArOption::new(0, "تعذّر تحميل القرّاء")];
                self.source_tab = SourceTab::File;
                self.surah_opts = Self::surah_options(None);
            }
            Message::SurahTextLoaded(surah, ed, Ok(st)) => {
                if surah == self.surah && ed == self.text_edition() {
                    self.surah_text = Some(st);
                    self.sync_json_and_inputs();
                }
            }
            Message::SurahTextLoaded(_, _, Err(_)) => {}
            Message::SourceTabChanged(tab) => {
                if tab != self.source_tab {
                    self.clear_audio();
                    self.say("", false);
                }
                self.source_tab = tab;
                if tab != SourceTab::Mp3Quran {
                    self.surah_opts = Self::surah_options(None);
                } else {
                    return self.picked_moshaf();
                }
                self.restore_work();
            }
            Message::RiwayahChanged(r) => {
                self.riwayah = r;
                self.fill_reciters();
                return self.load_text_task();
            }
            Message::OpenSelector(kind) => {
                self.open_selector = Some(kind);
                self.selector_query.clear();
                self.selector_highlight = 0;
                return text_input::focus(search_input_id());
            }
            Message::CloseSelector => {
                self.open_selector = None;
                self.selector_query.clear();
                self.selector_highlight = 0;
            }
            Message::SelectorQueryChanged(raw) => {
                let clean: String = raw
                    .chars()
                    .filter(|&c| !matches!(c, '\u{200E}' | '\u{2067}' | '\u{2069}'))
                    .collect();
                self.selector_highlight = 0;
                if clean.is_empty() {
                    self.selector_query.clear();
                } else if raw.starts_with('\u{200E}') {
                    self.selector_query = format!("\u{200E}{clean}");
                } else {
                    self.selector_query = format!("\u{200E}{clean}");
                    return Task::batch([
                        text_input::focus(search_input_id()),
                        text_input::move_cursor_to_end(search_input_id()),
                    ]);
                }
            }
            Message::ClearSelectorQuery => {
                self.selector_query.clear();
                self.selector_highlight = 0;
                return text_input::focus(search_input_id());
            }
            Message::SelectorSubmit => {
                if let Some(kind) = self.open_selector {
                    let filtered = self.filtered_selector_options(kind);
                    if let Some(opt) = filtered
                        .get(self.selector_highlight)
                        .or_else(|| filtered.first())
                    {
                        let idx = opt.idx;
                        return self.update(Message::SelectorPick(kind, idx));
                    }
                }
            }
            Message::SelectorPick(kind, idx) => {
                self.open_selector = None;
                self.selector_query.clear();
                self.selector_highlight = 0;
                match kind {
                    SelectorKind::Reciter => {
                        if idx < self.reciters.len() {
                            self.selected_reciter = idx;
                            return self.fill_moshaf();
                        }
                    }
                    SelectorKind::Moshaf => {
                        if let Some(r) = self.reciters.get(self.selected_reciter) {
                            if idx < r.moshaf.len() {
                                self.selected_moshaf = idx;
                                return self.picked_moshaf();
                            }
                        }
                    }
                    SelectorKind::Surah => {
                        if (1..=114).contains(&idx) {
                            if self.source_tab == SourceTab::Mp3Quran && idx != self.surah {
                                self.clear_audio();
                                self.say("", false);
                            }
                            self.surah = idx;
                            self.surah_text = None;
                            self.restore_work();
                            return self.load_text_task();
                        }
                    }
                    SelectorKind::Counting => {
                        self.counting = if idx == 0 { Counting::Hafs } else { Counting::Madani };
                        self.surah_text = None;
                        self.restore_work();
                        return self.load_text_task();
                    }
                }
            }
            Message::Noop => {}
            Message::PickAudioFile => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("Audio", &["mp3", "wav", "m4a", "flac", "ogg", "aac"])
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::AudioFilePicked,
                );
            }
            Message::AudioFilePicked(Some(p)) => {
                self.file_path = Some(p);
                return self.update(Message::LoadAudio);
            }
            Message::AudioFilePicked(None) => {}
            Message::UrlChanged(u) => self.url_input = u,
            Message::LoadAudio => {
                self.clear_audio();
                let seq = self.load_seq;
                let cancel = Arc::new(AtomicBool::new(false));
                self.load_cancel = Arc::clone(&cancel);

                let (local_path, remote_url, label) = match self.source_tab {
                    SourceTab::File => match self.file_path.clone() {
                        Some(p) => (Some(p.clone()), None, p.display().to_string()),
                        None => {
                            self.say("اختر ملفًا صوتيًا.", true);
                            return Task::none();
                        }
                    },
                    SourceTab::Mp3Quran | SourceTab::Url => match self.audio_url() {
                        Some(u) => (None, Some(u.clone()), u),
                        None => {
                            self.say("لا يوجد رابط.", true);
                            return Task::none();
                        }
                    },
                };

                self.say("جارٍ تحميل وتحليل التلاوة…", false);
                return Task::perform(
                    async move {
                        let res: Result<(PathBuf, AudioAnalysis), String> = async {
                            let path = if let Some(p) = local_path {
                                p
                            } else {
                                let url = remote_url.unwrap();
                                let dir = cache_dir().join("audio");
                                tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
                                let hash = format!(
                                    "{:x}.mp3",
                                    url.bytes().fold(14695981039346656037u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211))
                                );
                                let dest = dir.join(hash);
                                if !dest.exists() {
                                    let bytes = http_client()
                                        .map_err(|e| e.to_string())?
                                        .get(&url)
                                        .send()
                                        .await
                                        .map_err(|e| e.to_string())?
                                        .error_for_status()
                                        .map_err(|e| e.to_string())?
                                        .bytes()
                                        .await
                                        .map_err(|e| e.to_string())?;
                                    tokio::fs::write(&dest, &bytes).await.map_err(|e| e.to_string())?;
                                }
                                dest
                            };
                            let p_clone = path.clone();
                            let c_clone = Arc::clone(&cancel);
                            let analysis = tokio::task::spawn_blocking(move || {
                                analyze_audio_file(&p_clone, &c_clone, |_| {})
                            })
                            .await
                            .map_err(|e| e.to_string())?
                            .map_err(|e| e.to_string())?;
                            Ok((path, analysis))
                        }
                        .await;
                        match res {
                            Ok((p, a)) => Message::AudioAnalyzed(seq, p, label, Ok(a)),
                            Err(e) => Message::AudioAnalyzed(seq, PathBuf::new(), label, Err(e)),
                        }
                    },
                    |m| m,
                );
            }
            Message::AudioAnalyzed(seq, path, label, Ok(analysis)) => {
                if seq != self.load_seq {
                    return Task::none();
                }
                let rate = self.rate_str.parse::<f32>().unwrap_or(1.0);
                self.player.load(path.clone(), rate);
                self.audio_path = Some(path);
                self.audio_label = label;
                self.peaks = analysis.peaks;
                self.energy = analysis.energy;
                self.duration_ms = analysis.duration_ms;
                self.zoom = ((self.duration_ms as f32 / 45000.0).round()).clamp(1.0, 60.0);
                self.scroll_frac = 0.0;
                self.restore_work();
                self.say(format!("جاهز · {}", fmt_ms(self.duration_ms)), false);
            }
            Message::AudioAnalyzed(seq, _, _, Err(e)) => {
                if seq == self.load_seq && e != "aborted" {
                    self.say(format!("تعذّر تحميل التلاوة: {e}"), true);
                }
            }
            Message::Tick => {
                let mut scroll_task = Task::none();
                if self.player.has_audio() {
                    let pos = self.player.position_ms();
                    self.now_ms = pos.min(self.duration_ms.max(pos));
                    if self.player.is_playing() && self.follow {
                        self.reveal_now();
                    }
                    let cur = current_ayah(&self.starts, self.now_ms);
                    if cur != self.last_cur {
                        self.last_cur = cur;
                        if self.follow {
                            scroll_task = self.scroll_inspector_to(cur);
                        }
                    }
                }
                if self.ai_running || self.export_running {
                    let prog = self.ai_progress_slot.lock().ok().and_then(|g| g.clone());
                    if let Some(p) = prog {
                        let txt = p.text.clone();
                        let err = p.err.is_some();
                        self.ai_progress = Some(p);
                        self.say(txt, err);
                    }
                }
                return scroll_task;
            }
            Message::TogglePlay => {
                if self.player.has_audio() {
                    self.player.toggle();
                }
            }
            Message::Seek(ms) => {
                if self.duration_ms > 0 {
                    let clamped = ms.min(self.duration_ms);
                    self.now_ms = clamped;
                    self.player.seek(clamped);
                    let cur = current_ayah(&self.starts, self.now_ms);
                    self.last_cur = cur;
                    if self.follow {
                        self.reveal_now();
                        return self.scroll_inspector_to(cur);
                    }
                }
            }
            Message::RatePicked(r) => {
                if let Ok(v) = r.parse::<f32>() {
                    self.rate_str = r;
                    self.player.set_rate(v);
                }
            }
            Message::ReactionChanged(s) => {
                self.reaction_str = s.clone();
                if let Ok(v) = s.trim().parse::<u64>() {
                    self.reaction_ms = v;
                }
            }
            Message::SnapToggled(b) => self.snap = b,
            Message::MarkNext => {
                if self.duration_ms == 0 {
                    return Task::none();
                }
                let Some(a) = next_unmarked(&self.starts, self.count()) else {
                    self.say("عُلِّمت كل الآيات.", false);
                    return Task::none();
                };
                let mut ms = self.now_ms.saturating_sub(self.reaction_ms);
                if self.snap && !self.energy.is_empty() {
                    let at = (ms as f64 / 10.0).round() as usize;
                    let a0 = at.saturating_sub(60);
                    let a1 = (at + 60).min(self.energy.len() - 1);
                    let mut best = at.min(a1);
                    let mut low = f32::INFINITY;
                    for i in a0..=a1 {
                        if self.energy[i] < low {
                            low = self.energy[i];
                            best = i;
                        }
                    }
                    let mut j = best;
                    let back = low * 3.0 + 0.01;
                    while j < self.energy.len() - 1 && j < best + 80 && self.energy[j] < back {
                        j += 1;
                    }
                    ms = (j as u64 * 10).saturating_sub(60);
                }
                let prev = (0..a).rev().find_map(|i| self.starts.get(i).copied().flatten()).unwrap_or(0);
                let c = self.count();
                let dur = self.duration_ms;
                set_start_clamped(&mut self.starts, a, Some(ms.max(prev + 50)), c, dur);
                self.save_and_sync();
            }
            Message::Undo => {
                for a in (1..self.starts.len()).rev() {
                    if self.starts[a].is_some() {
                        self.starts[a] = None;
                        break;
                    }
                }
                while self.starts.len() > 1 && self.starts.last() == Some(&None) {
                    self.starts.pop();
                }
                self.save_and_sync();
            }
            Message::ResetMarks => {
                self.starts = vec![Some(0)];
                self.save_and_sync();
            }
            Message::AutoHeuristic => {
                if self.energy.is_empty() {
                    self.say("حمّل التلاوة أولًا.", true);
                    return Task::none();
                }
                let texts = self.surah_text.as_ref().map(|t| t.texts.as_slice());
                self.starts = auto_heuristic(&self.energy, texts, self.surah, self.count(), &self.starts);
                self.save_and_sync();
                self.say("اقتراح أولي: راجِع العلامات واسحبها إن لزم.", false);
            }
            Message::ToggleAiAlign => {
                if self.ai_running {
                    self.ai_cancel.store(true, Ordering::Relaxed);
                    self.ai_running = false;
                    self.say("أُوقف التوقيت بالذكاء الاصطناعي.", false);
                    return Task::none();
                }
                if self.export_running {
                    self.say("عملية تصدير الصوت قيد التشغيل…", true);
                    return Task::none();
                }
                let Some(audio_path) = self.audio_path.clone() else {
                    self.say("حمّل التلاوة أولًا.", true);
                    return Task::none();
                };
                let Some(st) = self.surah_text.clone() else {
                    self.say("نص السورة قيد التحميل… انتظر لحظة.", true);
                    return Task::none();
                };
                self.ai_running = true;
                let cancel = Arc::new(AtomicBool::new(false));
                self.ai_cancel = Arc::clone(&cancel);
                let slot = Arc::clone(&self.ai_progress_slot);
                let init_prog = AiProgress {
                    text: "جارٍ تحضير نموذج الذكاء الاصطناعي…".into(),
                    pct: Some(0),
                    done: false,
                    err: None,
                };
                self.ai_progress = Some(init_prog.clone());
                if let Ok(mut g) = slot.lock() {
                    *g = Some(init_prog);
                }

                let surah = self.surah;
                let energy = self.energy.clone();
                let dur = self.duration_ms;

                return Task::perform(
                    async move {
                        let slot_a = Arc::clone(&slot);
                        let report_a = move |p: AiProgress| {
                            if let Ok(mut g) = slot_a.lock() {
                                *g = Some(p);
                            }
                        };
                        let (model_path, vocab) = ensure_ai_model(&cancel, &report_a)
                            .await
                            .map_err(|e| e.to_string())?;
                        let texts = st.texts;
                        let slot_b = Arc::clone(&slot);
                        tokio::task::spawn_blocking(move || {
                            let report_b = |p: AiProgress| {
                                if let Ok(mut g) = slot_b.lock() {
                                    *g = Some(p);
                                }
                            };
                            run_ai_alignment(
                                &audio_path,
                                &model_path,
                                &vocab,
                                &texts,
                                surah,
                                &energy,
                                dur,
                                &cancel,
                                &report_b,
                            )
                            .map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::AiFinished,
                );
            }
            Message::DismissAiProgress => {
                if !self.ai_running && !self.export_running {
                    self.ai_progress = None;
                }
            }
            Message::AiFinished(Ok(starts)) => {
                self.ai_running = false;
                let mut v = vec![Some(0)];
                v.extend(starts.into_iter().map(Some));
                self.starts = v;
                self.save_and_sync();
                let done_prog = AiProgress {
                    text: "تم التوقيت بالذكاء الاصطناعي ✓ راجِع العلامات إن لزم.".into(),
                    pct: Some(100),
                    done: true,
                    err: None,
                };
                self.ai_progress = Some(done_prog.clone());
                self.say(done_prog.text, false);
            }
            Message::AiFinished(Err(e)) => {
                self.ai_running = false;
                if e == "aborted" {
                    self.ai_progress = None;
                    self.say("أُوقف التوقيت بالذكاء الاصطناعي.", false);
                } else {
                    let msg = format!("تعذّر التوقيت بالذكاء الاصطناعي: {e}");
                    self.ai_progress = Some(AiProgress {
                        text: msg.clone(),
                        pct: Some(0),
                        done: false,
                        err: Some(e),
                    });
                    self.say(msg, true);
                }
            }
            Message::Wave(ev) => match ev {
                WaveEvent::Seek(ms) => return self.update(Message::Seek(ms)),
                WaveEvent::MoveMark(a, ms) => {
                    let c = self.count();
                    let dur = self.duration_ms;
                    set_start_clamped(&mut self.starts, a, Some(ms), c, dur);
                    self.save_and_sync();
                }
                WaveEvent::WheelZoom(dy) => {
                    let factor = if dy > 0.0 { 1.15 } else { 0.87 };
                    self.zoom = (self.zoom * factor).clamp(1.0, 60.0);
                    if self.follow {
                        self.reveal_now();
                    }
                }
                WaveEvent::WheelScroll(dx) => {
                    self.scroll_frac = (self.scroll_frac + dx * 0.002).clamp(0.0, 1.0);
                }
            },
            Message::ZoomChanged(z) => {
                self.zoom = z.clamp(1.0, 60.0);
                if self.follow {
                    self.reveal_now();
                }
            }
            Message::ZoomStep(dir) => {
                let step = (self.zoom / 6.0).round().max(1.0);
                self.zoom = (self.zoom + dir * step).clamp(1.0, 60.0);
                if self.follow {
                    self.reveal_now();
                }
            }
            Message::ScrollChanged(f) => self.scroll_frac = f.clamp(0.0, 1.0),
            Message::FollowToggled(f) => self.follow = f,
            Message::InspectorChanged(tab) => {
                self.inspector_tab = tab;
                let cur = current_ayah(&self.starts, self.now_ms);
                return self.scroll_inspector_to(cur);
            }
            Message::PlayAyah(a) => {
                if self.player.has_audio() {
                    let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
                    let tl = build_timeline(&self.starts, self.count(), self.duration_ms, pages);
                    if let Some(row) = tl.get(a) {
                        if let Some(st) = row.start {
                            self.now_ms = st;
                            self.last_cur = a;
                            self.player.play_span(st, row.end);
                            if self.follow {
                                self.reveal_now();
                            }
                            return self.scroll_inspector_to(a);
                        }
                    }
                }
            }
            Message::SetAyahToNow(a) => {
                let c = self.count();
                let dur = self.duration_ms;
                let mut s = self.starts.clone();
                set_start_clamped(&mut s, a, Some(self.now_ms), c, dur);
                self.starts = s;
                self.save_and_sync();
            }
            Message::NudgeAyah(a, by) => {
                let cur = self.starts.get(a).copied().flatten().unwrap_or(0) as i64;
                let next = (cur + by).max(0) as u64;
                let c = self.count();
                let dur = self.duration_ms;
                set_start_clamped(&mut self.starts, a, Some(next), c, dur);
                self.save_and_sync();
            }
            Message::ClearAyah(a) => {
                let c = self.count();
                let dur = self.duration_ms;
                set_start_clamped(&mut self.starts, a, None, c, dur);
                self.save_and_sync();
            }
            Message::TableInputChanged(a, v) => {
                self.table_inputs.insert(a, v);
            }
            Message::TableInputSubmit(a) => {
                if let Some(raw) = self.table_inputs.get(&a).cloned() {
                    let t = raw.trim();
                    let ms = if t.is_empty() { None } else { t.parse::<u64>().ok() };
                    if t.is_empty() || ms.is_some() {
                        let c = self.count();
                        let dur = self.duration_ms;
                        set_start_clamped(&mut self.starts, a, ms, c, dur);
                        self.save_and_sync();
                    }
                }
            }
            Message::FullFieldsToggled(f) => {
                self.full_fields = f;
                self.sync_json_and_inputs();
            }
            Message::JsonMinifiedToggled(m) => {
                self.json_minified = m;
                self.sync_json_and_inputs();
            }
            Message::MushafFontDelta(d) => {
                self.mushaf_font_size = (self.mushaf_font_size + d).clamp(18.0, 38.0);
            }
            Message::JumpToCurrentAyah => {
                let cur = current_ayah(&self.starts, self.now_ms);
                self.last_cur = cur;
                self.reveal_now();
                return self.scroll_inspector_to(cur);
            }
            Message::StepAyah(delta) => {
                let cur = current_ayah(&self.starts, self.now_ms) as i32;
                let target = (cur + delta).clamp(0, self.count() as i32) as usize;
                if let Some(st) = self.starts.get(target).copied().flatten() {
                    self.now_ms = st;
                    self.last_cur = target;
                    self.player.seek(st);
                    self.reveal_now();
                    return self.scroll_inspector_to(target);
                }
            }
            Message::ExportFormatPicked(fmt) => self.export_opts.format = fmt,
            Message::ExportBitratePicked(br) => self.export_opts.bitrate_kbps = br,
            Message::ExportMonoToggled(m) => self.export_opts.mono = m,
            Message::ExportIntroToggled(inc) => self.export_opts.include_intro = inc,
            Message::ExportNamingPicked(n) => self.export_opts.naming = n,
            Message::StartSplitAudio => {
                if self.ai_running || self.export_running {
                    return Task::none();
                }
                if self.audio_path.is_none() {
                    self.say("حمّل التلاوة أولًا.", true);
                    return Task::none();
                }
                return Task::perform(
                    async { rfd::AsyncFileDialog::new().pick_folder().await.map(|h| h.path().to_path_buf()) },
                    Message::SplitDirPicked,
                );
            }
            Message::SplitDirPicked(Some(dir)) => {
                let Some(audio_path) = self.audio_path.clone() else {
                    return Task::none();
                };
                let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
                let rows = build_timeline(&self.starts, self.count(), self.duration_ms, pages);
                let surah = self.surah;
                let opts = self.export_opts;
                let cancel = Arc::new(AtomicBool::new(false));
                self.ai_cancel = Arc::clone(&cancel);
                self.export_running = true;
                let slot = Arc::clone(&self.ai_progress_slot);
                let init_prog = AiProgress {
                    text: "جارٍ تحضير تقسيم الآيات…".into(),
                    pct: Some(0),
                    done: false,
                    err: None,
                };
                self.ai_progress = Some(init_prog.clone());
                if let Ok(mut g) = slot.lock() {
                    *g = Some(init_prog);
                }
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let report = |p: AiProgress| {
                                if let Ok(mut g) = slot.lock() {
                                    *g = Some(p);
                                }
                            };
                            let (cnt, bytes) =
                                split_audio_by_ayahs(&audio_path, &dir, surah, &rows, &opts, &cancel, &report)
                                    .map_err(|e| e.to_string())?;
                            Ok(format!(
                                "تم تقسيم {cnt} آية ({}) في {}",
                                format_bytes(bytes),
                                dir.display()
                            ))
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::ExportFinished,
                );
            }
            Message::SplitDirPicked(None) => {}
            Message::StartCompressAudio => {
                if self.ai_running || self.export_running {
                    return Task::none();
                }
                if self.audio_path.is_none() {
                    self.say("حمّل التلاوة أولًا.", true);
                    return Task::none();
                }
                let ext = if self.export_opts.format == AudioFormat::Copy {
                    "mp3"
                } else {
                    self.export_opts.format.ext()
                };
                let def_name = format!("{:03}_compressed.{ext}", self.surah);
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_file_name(&def_name)
                            .save_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::CompressDestPicked,
                );
            }
            Message::CompressDestPicked(Some(dest)) => {
                let Some(audio_path) = self.audio_path.clone() else {
                    return Task::none();
                };
                let opts = self.export_opts;
                let cancel = Arc::new(AtomicBool::new(false));
                self.ai_cancel = Arc::clone(&cancel);
                self.export_running = true;
                let slot = Arc::clone(&self.ai_progress_slot);
                let init_prog = AiProgress {
                    text: "جارٍ ضغط الملف الصوتي…".into(),
                    pct: Some(20),
                    done: false,
                    err: None,
                };
                self.ai_progress = Some(init_prog.clone());
                if let Ok(mut g) = slot.lock() {
                    *g = Some(init_prog);
                }
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let report = |p: AiProgress| {
                                if let Ok(mut g) = slot.lock() {
                                    *g = Some(p);
                                }
                            };
                            let (src_b, dst_b) = compress_audio(&audio_path, &dest, &opts, &cancel, &report)
                                .map_err(|e| e.to_string())?;
                            Ok(format!(
                                "تم الضغط: {} ← {}",
                                format_bytes(dst_b),
                                format_bytes(src_b)
                            ))
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::ExportFinished,
                );
            }
            Message::CompressDestPicked(None) => {}
            Message::CancelExport => {
                if self.export_running {
                    self.ai_cancel.store(true, Ordering::Relaxed);
                    self.export_running = false;
                    self.ai_progress = None;
                    self.say("أُوقفت عملية التصدير.", false);
                }
            }
            Message::ExportFinished(Ok(msg)) => {
                self.export_running = false;
                self.ai_progress = Some(AiProgress {
                    text: msg.clone(),
                    pct: Some(100),
                    done: true,
                    err: None,
                });
                self.say(msg, false);
            }
            Message::ExportFinished(Err(e)) => {
                self.export_running = false;
                if e == "aborted" {
                    self.ai_progress = None;
                    self.say("أُوقفت عملية التصدير.", false);
                } else {
                    self.ai_progress = Some(AiProgress {
                        text: e.clone(),
                        pct: Some(0),
                        done: false,
                        err: Some(e.clone()),
                    });
                    self.say(e, true);
                }
            }
            Message::JsonAction(act) => self.json_content.perform(act),
            Message::ImportFromText => {
                let txt = self.json_content.text();
                match import_json(&txt, self.count()) {
                    Ok((s, imp, dropped)) => {
                        self.starts = s;
                        self.save_and_sync();
                        if dropped > 0 {
                            self.say(format!("استُورد {imp} آية (تُجاهِل {dropped} غير مرتّب)."), false);
                        } else {
                            self.say(format!("استُورد {imp} آية."), false);
                        }
                    }
                    Err(e) => self.say(e, true),
                }
            }
            Message::ImportFromFile => {
                return Task::perform(
                    async {
                        let h = rfd::AsyncFileDialog::new()
                            .add_filter("JSON", &["json"])
                            .pick_file()
                            .await?;
                        std::fs::read_to_string(h.path()).ok()
                    },
                    Message::ImportFileLoaded,
                );
            }
            Message::ImportFileLoaded(Some(txt)) => {
                self.json_content = text_editor::Content::with_text(&txt);
                return self.update(Message::ImportFromText);
            }
            Message::ImportFileLoaded(None) => {}
            Message::DownloadJson => {
                let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
                let json = export_json(
                    &self.starts,
                    self.count(),
                    self.duration_ms,
                    pages,
                    self.full_fields,
                    self.json_minified,
                );
                let default_name = format!("{:03}.json", self.surah);
                return Task::perform(
                    async move {
                        if let Some(h) = rfd::AsyncFileDialog::new()
                            .set_file_name(&default_name)
                            .add_filter("JSON", &["json"])
                            .save_file()
                            .await
                        {
                            let _ = std::fs::write(h.path(), json);
                        }
                    },
                    |_| Message::Tick,
                );
            }
            Message::CopyJson => {
                let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
                let json = export_json(
                    &self.starts,
                    self.count(),
                    self.duration_ms,
                    pages,
                    self.full_fields,
                    self.json_minified,
                );
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(json);
                    self.say("نُسخ.", false);
                }
            }
            Message::FetchPublishedTiming => {
                let Some(m) = self.current_moshaf() else {
                    self.say("لا يوجد توقيت منشور لهذه التلاوة في mp3quran.", true);
                    return Task::none();
                };
                let Some(&read_id) = self.timed_reads.get(&norm_url(&m.server)) else {
                    self.say("لا يوجد توقيت منشور لهذه التلاوة في mp3quran.", true);
                    return Task::none();
                };
                let surah = self.surah;
                return Task::perform(
                    async move { fetch_published_timings(surah, read_id).await.map_err(|e| e.to_string()) },
                    Message::PublishedTimingLoaded,
                );
            }
            Message::PublishedTimingLoaded(Ok(txt)) => {
                if txt.trim() == "[]" {
                    self.say("لا يوجد توقيت لهذه السورة.", true);
                } else {
                    self.json_content = text_editor::Content::with_text(&txt);
                    return self.update(Message::ImportFromText);
                }
            }
            Message::PublishedTimingLoaded(Err(e)) => self.say(format!("تعذّر الجلب: {e}"), true),
            Message::ToggleTheme => self.dark = !self.dark,
            Message::KeyPressed(key, mods) => {
                if let Some(kind) = self.open_selector {
                    match key.as_ref() {
                        Key::Named(Named::Escape) => return self.update(Message::CloseSelector),
                        Key::Named(Named::ArrowDown) => {
                            let len = self.filtered_selector_options(kind).len();
                            if self.selector_highlight + 1 < len {
                                self.selector_highlight += 1;
                            }
                        }
                        Key::Named(Named::ArrowUp) => {
                            self.selector_highlight = self.selector_highlight.saturating_sub(1);
                        }
                        _ => {}
                    }
                    return Task::none();
                }
                if mods.control() || mods.alt() || mods.logo() || self.inspector_tab == InspectorTab::Io {
                    return Task::none();
                }
                match key.as_ref() {
                    Key::Named(Named::Space) => return self.update(Message::MarkNext),
                    Key::Character("p") | Key::Character("P") => return self.update(Message::TogglePlay),
                    Key::Character("z") | Key::Character("Z") => return self.update(Message::Undo),
                    Key::Named(Named::ArrowLeft) => {
                        return self.update(Message::Seek(self.now_ms.saturating_sub(5000)))
                    }
                    Key::Named(Named::ArrowRight) => {
                        return self.update(Message::Seek(self.now_ms + 5000))
                    }
                    Key::Character("[") => return self.update(Message::StepAyah(-1)),
                    Key::Character("]") => return self.update(Message::StepAyah(1)),
                    _ => {}
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick);
        let keys = event::listen_with(|ev, status, _| {
            if status == event::Status::Ignored {
                if let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = ev {
                    return Some(Message::KeyPressed(key, modifiers));
                }
            }
            None
        });
        Subscription::batch([tick, keys])
    }

    fn card<'a>(&self, content: impl Into<Element<'a, Message>>, pad: u16) -> container::Container<'a, Message> {
        let p = self.pal();
        container(content).padding(pad).style(move |_| container::Style {
            background: Some(p.panel.into()),
            border: border::Border {
                color: p.line,
                width: 1.0,
                radius: 12.0.into(),
            },
            ..Default::default()
        })
    }

    fn btn_primary<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Option<Message>,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        let b = button(label).padding(pad).style(move |_, status| {
            let bg = match status {
                button::Status::Hovered | button::Status::Pressed => p.accent_hover,
                button::Status::Disabled => p.line,
                button::Status::Active => p.accent,
            };
            let fg = if status == button::Status::Disabled {
                p.muted
            } else {
                p.accent_ink
            };
            button::Style {
                background: Some(bg.into()),
                text_color: fg,
                border: border::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });
        if let Some(m) = msg {
            b.on_press(m)
        } else {
            b
        }
    }

    fn btn_soft<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Message,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        button(label).padding(pad).on_press(msg).style(move |_, status| button::Style {
            background: Some(if status == button::Status::Hovered { p.panel2 } else { p.soft }.into()),
            text_color: p.accent,
            border: border::Border {
                color: p.accent,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
    }

    fn btn_ghost<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Message,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        button(label).padding(pad).on_press(msg).style(move |_, status| button::Style {
            background: Some(if status == button::Status::Hovered { p.soft } else { p.panel2 }.into()),
            text_color: p.ink,
            border: border::Border {
                color: p.line,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
    }

    fn btn_danger<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Message,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        button(label).padding(pad).on_press(msg).style(move |_, status| button::Style {
            background: Some(if status == button::Status::Hovered { p.danger_soft } else { p.panel2 }.into()),
            text_color: p.danger,
            border: border::Border {
                color: p.danger,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
    }

    fn styled_input<'a>(
        &self,
        placeholder: &str,
        value: &str,
        on_input: impl Fn(String) -> Message + 'a,
        size: f32,
        pad: impl Into<Padding>,
        width: Length,
    ) -> text_input::TextInput<'a, Message> {
        let p = self.pal();
        text_input(placeholder, value)
            .on_input(on_input)
            .font(self.ui_font)
            .size(size)
            .padding(pad)
            .width(width)
            .style(move |_, status| text_input::Style {
                background: p.panel2.into(),
                border: border::Border {
                    color: if status == text_input::Status::Focused {
                        p.accent
                    } else {
                        p.line
                    },
                    width: 1.0,
                    radius: 7.0.into(),
                },
                icon: p.muted,
                placeholder: p.muted,
                value: p.ink,
                selection: p.soft,
            })
    }

    fn seg_btn<'a>(&self, label: &'a str, active: bool, msg: Message) -> Element<'a, Message> {
        let p = self.pal();
        button(ar(label, self.ui_font).size(12.5))
            .padding([5, 11])
            .on_press(msg)
            .style(move |_, status| {
                let bg = if active {
                    p.accent
                } else if status == button::Status::Hovered {
                    p.soft
                } else {
                    p.panel2
                };
                button::Style {
                    background: Some(bg.into()),
                    text_color: if active { p.accent_ink } else { p.muted },
                    border: border::Border {
                        color: p.line,
                        width: if active { 0.0 } else { 1.0 },
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    fn styled_pick<'a, T>(
        &self,
        options: &'a [T],
        selected: Option<T>,
        on_selected: impl Fn(T) -> Message + 'a,
        placeholder: &str,
        width: Length,
    ) -> pick_list::PickList<'a, T, &'a [T], T, Message>
    where
        T: ToString + PartialEq + Clone + 'a,
    {
        let p = self.pal();
        pick_list(options, selected, on_selected)
            .placeholder(rtl(placeholder))
            .font(self.ui_font)
            .text_shaping(text::Shaping::Advanced)
            .text_size(12.5)
            .padding([6, 10])
            .width(width)
            .style(move |_, status| pick_list::Style {
                text_color: p.ink,
                placeholder_color: p.muted,
                handle_color: p.muted,
                background: p.panel2.into(),
                border: border::Border {
                    color: if status == pick_list::Status::Hovered || status == pick_list::Status::Opened {
                        p.accent
                    } else {
                        p.line
                    },
                    width: 1.0,
                    radius: 8.0.into(),
                },
            })
            .menu_style(move |_| menu::Style {
                background: p.panel.into(),
                border: border::Border {
                    color: p.line,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                text_color: p.ink,
                selected_text_color: p.accent_ink,
                selected_background: p.accent.into(),
            })
    }

    fn filtered_selector_options(&self, kind: SelectorKind) -> Vec<&ArOption> {
        let opts = match kind {
            SelectorKind::Reciter => &self.reciter_opts,
            SelectorKind::Moshaf => &self.moshaf_opts,
            SelectorKind::Surah => &self.surah_opts,
            SelectorKind::Counting => &self.counting_opts,
        };
        let q = normalize_search(&self.selector_query);
        let tokens: Vec<&str> = q.split_whitespace().collect();
        if tokens.is_empty() {
            return opts.iter().collect();
        }
        opts.iter()
            .filter(|o| tokens.iter().all(|t| o.search_key.contains(t)))
            .collect()
    }

    fn selector_trigger<'a>(
        &self,
        label: &str,
        kind: SelectorKind,
        width: Length,
    ) -> Element<'a, Message> {
        let p = self.pal();
        let active = self.open_selector == Some(kind);
        button(
            row![
                text("▾").size(12).color(p.muted).shaping(text::Shaping::Advanced),
                Space::with_width(Length::Fill),
                ar(label, self.ui_font).size(12.5),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center),
        )
        .width(width)
        .padding([6, 10])
        .on_press(Message::OpenSelector(kind))
        .style(move |_, status| button::Style {
            background: Some(p.panel2.into()),
            text_color: p.ink,
            border: border::Border {
                color: if active || status == button::Status::Hovered {
                    p.accent
                } else {
                    p.line
                },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
        .into()
    }

    fn rtl_checkbox<'a>(
        &self,
        label: &'a str,
        checked: bool,
        on_toggle: impl Fn(bool) -> Message + 'a,
    ) -> Element<'a, Message> {
        row![
            ar(label, self.ui_font).size(12),
            checkbox("", checked).on_toggle(on_toggle).size(15),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .into()
    }

    fn styled_scrollable<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> scrollable::Scrollable<'a, Message> {
        let p = self.pal();
        scrollable(content).style(move |_, status| {
            let scroller_col = match status {
                scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. } => p.accent,
                scrollable::Status::Active => p.line,
            };
            let rail = scrollable::Rail {
                background: Some(Color::TRANSPARENT.into()),
                border: border::Border::default(),
                scroller: scrollable::Scroller {
                    color: scroller_col,
                    border: border::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                },
            };
            scrollable::Style {
                container: container::Style::default(),
                vertical_rail: rail,
                horizontal_rail: rail,
                gap: None,
            }
        })
    }

    fn view(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let qfont = self.quran_font();
        let n_ayahs = self.count();
        let done_count = (1..=n_ayahs)
            .filter(|&a| self.starts.get(a).copied().flatten().is_some())
            .count();
        let pct = if n_ayahs > 0 {
            ((done_count * 100) / n_ayahs) as f32
        } else {
            0.0
        };

        // Header (RTL: controls on Left, title on Right)
        let header = row![
            self.btn_ghost(
                ar(if self.dark { "فاتح" } else { "داكن" }, ui).size(12.5),
                [7, 12],
                Message::ToggleTheme,
            ),
            container(
                column![
                    row![
                        text(format!("{}%", pct.round() as u32)).size(12).color(p.accent),
                        Space::with_width(Length::Fill),
                        ar(format!("مُعلَّم {done_count} من {n_ayahs}"), ui).size(12),
                    ]
                    .align_y(alignment::Vertical::Center),
                    progress_bar(0.0..=100.0, pct).height(5),
                ]
                .spacing(4)
                .width(Length::Fixed(180.0))
            )
            .padding([6, 12])
            .style(move |_| container::Style {
                background: Some(p.panel.into()),
                border: border::Border {
                    color: p.line,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..Default::default()
            }),
            Space::with_width(Length::Fill),
            column![
                ar("مؤقِّت الآيات", ui).size(20),
                ar("توقيت آيات القرآن الكريم بالذكاء الاصطناعي", ui)
                    .size(11.5)
                    .color(p.muted),
            ]
            .align_x(alignment::Horizontal::Right)
            .spacing(2),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center);

        // Source panel (RTL)
        let source_tabs = row![
            self.seg_btn("رابط", self.source_tab == SourceTab::Url, Message::SourceTabChanged(SourceTab::Url)),
            self.seg_btn("ملف من جهازك", self.source_tab == SourceTab::File, Message::SourceTabChanged(SourceTab::File)),
            self.seg_btn("من mp3quran", self.source_tab == SourceTab::Mp3Quran, Message::SourceTabChanged(SourceTab::Mp3Quran)),
        ]
        .spacing(4);

        let status_txt = ar(&self.status, ui)
            .size(12)
            .color(if self.status_err { p.danger } else { p.muted });

        let riw_bar = row![
            self.seg_btn(Riwayah::Qalun.label(), self.riwayah == Riwayah::Qalun, Message::RiwayahChanged(Riwayah::Qalun)),
            self.seg_btn(Riwayah::Warsh.label(), self.riwayah == Riwayah::Warsh, Message::RiwayahChanged(Riwayah::Warsh)),
            self.seg_btn(Riwayah::Hafs.label(), self.riwayah == Riwayah::Hafs, Message::RiwayahChanged(Riwayah::Hafs)),
            self.seg_btn(Riwayah::All.label(), self.riwayah == Riwayah::All, Message::RiwayahChanged(Riwayah::All)),
        ]
        .spacing(4);

        let cur_rec = self
            .reciter_opts
            .get(self.selected_reciter)
            .map(|o| o.raw_label.as_str())
            .unwrap_or("اختر القارئ…");
        let cur_moshaf = self
            .moshaf_opts
            .get(self.selected_moshaf)
            .map(|o| o.raw_label.as_str())
            .unwrap_or("اختر الرواية…");
        let cur_surah = self
            .surah_opts
            .iter()
            .find(|o| o.idx == self.surah)
            .map(|o| o.raw_label.as_str())
            .unwrap_or("اختر سورة…");

        let src_inputs: Element<'_, Message> = match self.source_tab {
            SourceTab::Mp3Quran => row![
                column![
                    ar("المصحف/الرواية", ui).size(11).color(p.muted),
                    self.selector_trigger(cur_moshaf, SelectorKind::Moshaf, Length::Fill),
                ]
                .align_x(alignment::Horizontal::Right)
                .spacing(3)
                .width(Length::FillPortion(2)),
                column![
                    ar("القارئ", ui).size(11).color(p.muted),
                    self.selector_trigger(cur_rec, SelectorKind::Reciter, Length::Fill),
                ]
                .align_x(alignment::Horizontal::Right)
                .spacing(3)
                .width(Length::FillPortion(2)),
                column![
                    ar("الرواية", ui).size(11).color(p.muted),
                    riw_bar,
                ]
                .align_x(alignment::Horizontal::Right)
                .spacing(3),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Bottom)
            .width(Length::Fill)
            .into(),
            SourceTab::File => row![
                ar(
                    self.file_path
                        .as_ref()
                        .and_then(|fp| fp.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "لم يُختر ملف".into()),
                    ui,
                )
                .size(12)
                .color(p.muted),
                Space::with_width(Length::Fill),
                self.btn_ghost(
                    ar("اختر ملفًا صوتيًا (MP3 / WAV / FLAC / M4A)…", ui).size(12.5),
                    [7, 14],
                    Message::PickAudioFile,
                ),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center)
            .width(Length::Fill)
            .into(),
            SourceTab::Url => row![
                self.styled_input("https://…/001.mp3", &self.url_input, Message::UrlChanged, 13.0, [6, 10], Length::Fill),
            ]
            .width(Length::Fill)
            .into(),
        };

        let surah_row = row![
            self.btn_primary(
                ar("تحميل التلاوة", ui).size(13),
                [8, 16],
                Some(Message::LoadAudio),
            ),
            container(ar(format!("{n_ayahs} آية"), ui).size(12).color(p.muted))
                .padding([7, 11])
                .style(move |_| container::Style {
                    background: Some(p.panel2.into()),
                    border: border::Border {
                        color: p.line,
                        width: 1.0,
                        radius: 999.0.into(),
                    },
                    ..Default::default()
                }),
            column![
                ar("العدّ", ui).size(11).color(p.muted),
                self.selector_trigger(self.counting.label(), SelectorKind::Counting, Length::Fixed(165.0)),
            ]
            .align_x(alignment::Horizontal::Right)
            .spacing(3),
            column![
                ar("السورة", ui).size(11).color(p.muted),
                self.selector_trigger(cur_surah, SelectorKind::Surah, Length::Fixed(155.0)),
            ]
            .align_x(alignment::Horizontal::Right)
            .spacing(3),
            src_inputs,
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Bottom);

        let source_panel = self.card(
            column![
                row![
                    status_txt,
                    Space::with_width(Length::Fill),
                    source_tabs,
                    ar("١. التلاوة والسورة", ui).size(13.5),
                ]
                .spacing(10)
                .align_y(alignment::Vertical::Center),
                surah_row,
            ]
            .spacing(8),
            10,
        );

        // Verse stage
        let cur = current_ayah(&self.starts, self.now_ms);
        let next = next_unmarked(&self.starts, n_ayahs);
        let ed = self.text_edition();
        let page = self
            .surah_text
            .as_ref()
            .and_then(|st| st.pages.get(cur.saturating_sub(1)).copied().flatten());
        let riw_meta = match page {
            Some(pg) => format!("{} · ص {}", ed.ar_name(), ar_digits(pg as usize)),
            None => ed.ar_name().to_string(),
        };
        let cur_text = if cur == 0 {
            if next == Some(1) {
                "الاستعاذة والبسملة".to_string()
            } else {
                "—".to_string()
            }
        } else if let Some(st) = &self.surah_text {
            if let Some(txt) = st.texts.get(cur - 1) {
                format!("{txt} ﴿{}﴾", ar_digits(cur))
            } else {
                format!("الآية {cur}")
            }
        } else {
            format!("الآية {cur}")
        };

        let next_banner: Element<'_, Message> = if let Some(nx) = next {
            if let Some(txt) = self.surah_text.as_ref().and_then(|s| s.texts.get(nx - 1)) {
                container(
                    row![
                        Space::with_width(Length::Fill),
                        text(rtl(txt))
                            .size(19)
                            .line_height(1.55)
                            .font(qfont)
                            .shaping(text::Shaping::Advanced),
                        ar(format!("التالي (آية {nx}):"), ui).size(12.5).color(p.accent),
                    ]
                    .spacing(10)
                    .align_y(alignment::Vertical::Center),
                )
                .padding([6, 12])
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(p.soft.into()),
                    border: border::Border {
                        radius: 9.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
            } else {
                Space::with_height(0).into()
            }
        } else {
            container(ar("اكتمل تعليم جميع الآيات ✓", ui).size(12.5).color(p.accent))
                .padding([6, 12])
                .width(Length::Fill)
                .center_x(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(p.soft.into()),
                    border: border::Border {
                        radius: 9.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
        };

        let verse_stage = self.card(
            column![
                row![
                    ar(riw_meta, ui).size(11.5).color(p.muted),
                    Space::with_width(Length::Fill),
                    self.btn_ghost(
                        text("‹").size(13).shaping(text::Shaping::Advanced),
                        [2, 8],
                        Message::StepAyah(-1),
                    ),
                    container(
                        ar(
                            if cur == 0 {
                                "الافتتاح".into()
                            } else {
                                format!("الآية {cur}")
                            },
                            ui,
                        )
                        .size(11.5)
                        .color(p.accent)
                    )
                    .padding([3, 10])
                    .style(move |_| container::Style {
                        background: Some(p.soft.into()),
                        border: border::Border {
                            radius: 999.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    self.btn_ghost(
                        text("›").size(13).shaping(text::Shaping::Advanced),
                        [2, 8],
                        Message::StepAyah(1),
                    ),
                ]
                .spacing(5)
                .align_y(alignment::Vertical::Center),
                container(
                    self.styled_scrollable(
                        container(
                            text(rtl(&cur_text))
                                .size(25)
                                .line_height(1.85)
                                .font(qfont)
                                .shaping(text::Shaping::Advanced)
                                .align_x(alignment::Horizontal::Center)
                        )
                        .width(Length::Fill)
                        .center_x(Length::Fill)
                        .padding([4, 8])
                    )
                    .height(Length::Fixed(68.0))
                )
                .padding([4, 8])
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(p.panel2.into()),
                    border: border::Border {
                        radius: 10.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                next_banner,
            ]
            .spacing(7),
            10,
        );

        // Waveform & transport studio
        let wave_h = if self.ai_progress.is_some() { 82.0 } else { 114.0 };
        let wave_el = view_waveform(
            WaveformProgram {
                peaks: &self.peaks,
                duration_ms: self.duration_ms,
                starts: &self.starts,
                now_ms: self.now_ms,
                current_ayah: cur,
                zoom: self.zoom,
                scroll_frac: self.scroll_frac,
                font: ui,
                colors: WaveColors {
                    bg: p.bg,
                    line: p.line,
                    muted: p.muted,
                    wave: p.wave,
                    wave_played: p.wave_played,
                    verse: p.verse,
                    mark: p.mark,
                    now: p.now,
                },
            },
            wave_h,
        )
        .map(Message::Wave);

        let zoom_bar = row![
            self.rtl_checkbox("متابعة رأس التشغيل", self.follow, Message::FollowToggled),
            slider(0.0..=1.0, self.scroll_frac, Message::ScrollChanged)
                .step(0.005)
                .width(Length::Fill),
            ar("تمرير", ui).size(11.5).color(p.muted),
            Space::with_width(8),
            text(format!("×{}", self.zoom.round() as u32)).size(12),
            self.btn_ghost(text("+").size(12), [2, 8], Message::ZoomStep(1.0)),
            slider(1.0..=60.0, self.zoom, Message::ZoomChanged)
                .step(1.0)
                .width(Length::Fixed(130.0)),
            self.btn_ghost(text("−").size(12), [2, 8], Message::ZoomStep(-1.0)),
            ar("تكبير", ui).size(11.5).color(p.muted),
        ]
        .spacing(7)
        .align_y(alignment::Vertical::Center);

        let seek_bar = row![
            text(fmt_ms(self.now_ms)).size(12).color(p.muted),
            slider(
                0.0..=(self.duration_ms.max(1) as f32),
                self.now_ms as f32,
                |v| Message::Seek(v.round() as u64)
            )
            .step(20.0)
            .width(Length::Fill),
            text(fmt_ms(self.duration_ms)).size(12).color(p.muted),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center);

        let mark_label = match next {
            Some(a) => format!("علِّم بداية الآية {a}  [Space]"),
            None => "اكتمل التعليم ✓".into(),
        };
        let mark_msg = if self.duration_ms > 0 && next.is_some() {
            Some(Message::MarkNext)
        } else {
            None
        };
        let mark_btn = self
            .btn_primary(
                ar(mark_label, ui)
                    .size(14)
                    .width(Length::Fill)
                    .align_x(alignment::Horizontal::Center),
                [10, 16],
                mark_msg,
            )
            .width(Length::Fill);

        let transport = row![
            container(text(format!("{} / {}", fmt_ms(self.now_ms), fmt_ms(self.duration_ms))).size(12.5))
                .padding([7, 10])
                .style(move |_| container::Style {
                    background: Some(p.bg.into()),
                    border: border::Border {
                        color: p.line,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }),
            self.btn_ghost(
                ar("+٥ث", ui).size(12.5),
                [8, 12],
                Message::Seek(self.now_ms + 5000),
            ),
            self.btn_ghost(
                ar(
                    if self.player.is_playing() {
                        "إيقاف مؤقت"
                    } else {
                        "▶ تشغيل"
                    },
                    ui,
                )
                .size(12.5),
                [8, 14],
                Message::TogglePlay,
            ),
            self.btn_ghost(
                ar("−٥ث", ui).size(12.5),
                [8, 12],
                Message::Seek(self.now_ms.saturating_sub(5000)),
            ),
            mark_btn,
        ]
        .spacing(7)
        .align_y(alignment::Vertical::Center);

        let ai_btn = if self.ai_running {
            self.btn_danger(
                ar("■ إيقاف AI", ui).size(12.5),
                [6, 12],
                Message::ToggleAiAlign,
            )
        } else {
            self.btn_soft(
                ar("توقيت بالذكاء الاصطناعي", ui).size(12.5),
                [6, 12],
                Message::ToggleAiAlign,
            )
        };

        let tools_actions = row![
            self.btn_danger(ar("مسح الكل", ui).size(12.5), [6, 11], Message::ResetMarks),
            self.btn_ghost(ar("تراجع", ui).size(12.5), [6, 11], Message::Undo),
            Space::with_width(Length::Fill),
            self.btn_soft(ar("اقتراح تلقائي", ui).size(12.5), [6, 12], Message::AutoHeuristic),
            ai_btn,
        ]
        .spacing(7)
        .align_y(alignment::Vertical::Center);

        let tools_settings = row![
            self.rtl_checkbox("التصاق بالسكتة", self.snap, Message::SnapToggled),
            Space::with_width(Length::Fill),
            self.styled_input("250", &self.reaction_str, Message::ReactionChanged, 12.5, [5, 8], Length::Fixed(58.0))
                .align_x(alignment::Horizontal::Center),
            ar("تأخّر(ms)", ui).size(11.5).color(p.muted),
            Space::with_width(8),
            self.styled_pick(
                &self.rate_opts,
                Some(self.rate_str.clone()),
                Message::RatePicked,
                "1",
                Length::Fixed(68.0),
            ),
            ar("السرعة", ui).size(11.5).color(p.muted),
        ]
        .spacing(7)
        .align_y(alignment::Vertical::Center);

        let studio_col = column![
            verse_stage,
            self.card(
                column![
                    row![
                        ar("Space: علِّم · P: تشغيل · ←/→: ٥ث · Z: تراجع", ui)
                            .size(11)
                            .color(p.muted),
                        Space::with_width(Length::Fill),
                        ar("٢. الاستماع والتعليم", ui).size(13.5),
                    ]
                    .align_y(alignment::Vertical::Center),
                    wave_el,
                    zoom_bar,
                    seek_bar,
                    transport,
                    tools_actions,
                    tools_settings,
                ]
                .spacing(7),
                10,
            )
        ]
        .spacing(10)
        .width(Length::FillPortion(6));

        // Inspector column (on LEFT in RTL layout)
        let insp_tabs = row![
            self.btn_ghost(ar("تنزيل JSON", ui).size(11.5), [5, 9], Message::DownloadJson),
            Space::with_width(Length::Fill),
            self.seg_btn("الصوت", self.inspector_tab == InspectorTab::Audio, Message::InspectorChanged(InspectorTab::Audio)),
            self.seg_btn("JSON", self.inspector_tab == InspectorTab::Io, Message::InspectorChanged(InspectorTab::Io)),
            self.seg_btn("المصحف", self.inspector_tab == InspectorTab::Mushaf, Message::InspectorChanged(InspectorTab::Mushaf)),
            self.seg_btn("الآيات", self.inspector_tab == InspectorTab::Table, Message::InspectorChanged(InspectorTab::Table)),
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let pages_slice = self.surah_text.as_ref().map(|t| t.pages.as_slice());
        let tl = build_timeline(&self.starts, n_ayahs, self.duration_ms, pages_slice);

        let insp_body: Element<'_, Message> = match self.inspector_tab {
            InspectorTab::Table => {
                let header_row = container(
                    row![
                        ar("تحكم", ui).size(11).color(p.muted),
                        Space::with_width(Length::Fill),
                        ar("المدة", ui)
                            .size(11)
                            .color(p.muted)
                            .width(Length::Fixed(44.0))
                            .align_x(alignment::Horizontal::Center),
                        ar("النهاية", ui)
                            .size(11)
                            .color(p.muted)
                            .width(Length::Fixed(58.0))
                            .align_x(alignment::Horizontal::Center),
                        ar("البداية (ms)", ui)
                            .size(11)
                            .color(p.muted)
                            .width(Length::Fixed(72.0))
                            .align_x(alignment::Horizontal::Center),
                        ar("الآية", ui)
                            .size(11)
                            .color(p.muted)
                            .width(Length::Fixed(46.0))
                            .align_x(alignment::Horizontal::Right),
                    ]
                    .spacing(6)
                    .align_y(alignment::Vertical::Center),
                )
                .padding([3, 8]);

                let mut rows_col = column![].spacing(4);
                for r in tl {
                    let a = r.ayah;
                    let is_now = a == cur;
                    let start_el: Element<'_, Message> = if a == 0 {
                        text("0")
                            .size(12)
                            .width(Length::Fixed(72.0))
                            .align_x(alignment::Horizontal::Center)
                            .into()
                    } else {
                        let val = self.table_inputs.get(&a).cloned().unwrap_or_default();
                        self.styled_input("—", &val, move |s| Message::TableInputChanged(a, s), 12.0, [3, 6], Length::Fixed(72.0))
                            .align_x(alignment::Horizontal::Center)
                            .on_submit(Message::TableInputSubmit(a))
                            .into()
                    };
                    let end_txt = r.end.map(fmt_ms).unwrap_or_else(|| "–".into());
                    let dur_txt = match (r.start, r.end) {
                        (Some(s), Some(e)) => format!("{:.1}s", (e.saturating_sub(s)) as f64 / 1000.0),
                        _ => "–".into(),
                    };

                    let mut acts = row![].spacing(3);
                    if a > 0 {
                        acts = acts
                            .push(self.btn_danger(text("×").size(11), [3, 6], Message::ClearAyah(a)))
                            .push(self.btn_ghost(text("+").size(11), [3, 6], Message::NudgeAyah(a, 100)))
                            .push(self.btn_ghost(text("−").size(11), [3, 6], Message::NudgeAyah(a, -100)))
                            .push(self.btn_soft(ar("الآن", ui).size(10.5), [3, 6], Message::SetAyahToNow(a)));
                    }
                    acts = acts.push(self.btn_ghost(
                        text("▶").size(11).shaping(text::Shaping::Advanced),
                        [3, 6],
                        Message::PlayAyah(a),
                    ));

                    let ayah_cell: Element<'_, Message> = if a == 0 {
                        ar("الافتتاح", ui)
                            .size(11.5)
                            .width(Length::Fixed(46.0))
                            .align_x(alignment::Horizontal::Right)
                            .into()
                    } else {
                        text(a.to_string())
                            .size(12)
                            .width(Length::Fixed(46.0))
                            .align_x(alignment::Horizontal::Right)
                            .into()
                    };

                    let row_box = container(
                        row![
                            acts,
                            Space::with_width(Length::Fill),
                            text(dur_txt)
                                .size(12)
                                .color(p.muted)
                                .width(Length::Fixed(44.0))
                                .align_x(alignment::Horizontal::Center),
                            text(end_txt)
                                .size(12)
                                .color(p.muted)
                                .width(Length::Fixed(58.0))
                                .align_x(alignment::Horizontal::Center),
                            start_el,
                            ayah_cell,
                        ]
                        .spacing(6)
                        .align_y(alignment::Vertical::Center),
                    )
                    .padding([4, 6])
                    .style(move |_| container::Style {
                        background: Some(if is_now { p.verse } else { Color::TRANSPARENT }.into()),
                        border: border::Border {
                            color: if is_now { p.accent } else { p.line },
                            width: if is_now { 1.0 } else { 0.5 },
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    });
                    rows_col = rows_col.push(row_box);
                }
                column![
                    header_row,
                    self.styled_scrollable(
                        container(rows_col).padding(Padding {
                            top: 2.0,
                            right: 14.0,
                            bottom: 6.0,
                            left: 2.0,
                        })
                    )
                    .id(table_scroll_id())
                    .height(Length::Fill),
                ]
                .spacing(4)
                .height(Length::Fill)
                .into()
            }
            InspectorTab::Mushaf => {
                let mushaf_bar = row![
                    self.btn_ghost(ar("A−", ui).size(11.5), [3, 8], Message::MushafFontDelta(-2.0)),
                    self.btn_ghost(ar("A+", ui).size(11.5), [3, 8], Message::MushafFontDelta(2.0)),
                    self.btn_soft(ar("الآية الحالية", ui).size(11.5), [3, 9], Message::JumpToCurrentAyah),
                    Space::with_width(Length::Fill),
                    ar(
                        page.map(|pg| format!("صفحة {}", ar_digits(pg as usize)))
                            .unwrap_or_else(|| ed.ar_name().to_string()),
                        ui,
                    )
                    .size(11.5)
                    .color(p.muted),
                ]
                .spacing(5)
                .align_y(alignment::Vertical::Center);

                let mut m_col = column![].spacing(8);
                if let Some(st) = &self.surah_text {
                    if self.surah != 9 && !(self.surah == 1 && ed == TextEdition::Hafs) {
                        m_col = m_col.push(
                            container(
                                text(rtl("بِسْمِ اللَّهِ الرَّحْمَٰنِ الرَّحِيمِ"))
                                    .size(self.mushaf_font_size)
                                    .line_height(1.8)
                                    .font(qfont)
                                    .shaping(text::Shaping::Advanced),
                            )
                            .padding([4, 0])
                            .width(Length::Fill)
                            .center_x(Length::Fill),
                        );
                    }
                    let mut prev_pg: Option<u32> = None;
                    for (i, t) in st.texts.iter().enumerate() {
                        let a = i + 1;
                        let pg = st.pages.get(i).copied().flatten();
                        if let Some(pg_num) = pg {
                            if Some(pg_num) != prev_pg {
                                prev_pg = Some(pg_num);
                                m_col = m_col.push(
                                    container(
                                        ar(format!("— صفحة {} —", ar_digits(pg_num as usize)), ui)
                                            .size(11)
                                            .color(p.muted),
                                    )
                                    .width(Length::Fill)
                                    .center_x(Length::Fill)
                                    .padding([2, 0]),
                                );
                            }
                        }
                        let is_now = a == cur;
                        let marked = self.starts.get(a).copied().flatten().is_some();
                        let time_badge = self
                            .starts
                            .get(a)
                            .copied()
                            .flatten()
                            .map(fmt_ms)
                            .unwrap_or_default();
                        let line_txt = format!("{t} ﴿{}﴾", ar_digits(a));
                        let item = button(
                            column![
                                text(rtl(&line_txt))
                                    .size(self.mushaf_font_size)
                                    .line_height(1.9)
                                    .font(qfont)
                                    .shaping(text::Shaping::Advanced)
                                    .width(Length::Fill)
                                    .align_x(alignment::Horizontal::Right)
                                    .color(if marked { p.ink } else { p.muted }),
                                row![
                                    text(time_badge).size(10.5).color(if is_now { p.accent } else { p.muted }),
                                    Space::with_width(Length::Fill),
                                ],
                            ]
                            .spacing(2),
                        )
                        .width(Length::Fill)
                        .padding([10, 14])
                        .on_press(Message::PlayAyah(a))
                        .style(move |_, status| {
                            let bg = if is_now {
                                p.verse
                            } else if status == button::Status::Hovered {
                                p.soft
                            } else {
                                p.panel2
                            };
                            button::Style {
                                background: Some(bg.into()),
                                text_color: p.ink,
                                border: border::Border {
                                    color: if is_now { p.accent } else { p.line },
                                    width: if is_now { 1.5 } else { 0.5 },
                                    radius: 10.0.into(),
                                },
                                ..Default::default()
                            }
                        });
                        m_col = m_col.push(item);
                    }
                } else {
                    m_col = m_col.push(ar("جارٍ تحميل نص المصحف…", ui).size(13).color(p.muted));
                }
                column![
                    mushaf_bar,
                    self.styled_scrollable(
                        container(m_col).padding(Padding {
                            top: 2.0,
                            right: 14.0,
                            bottom: 8.0,
                            left: 2.0,
                        })
                    )
                    .id(mushaf_scroll_id())
                    .height(Length::Fill),
                ]
                .spacing(6)
                .height(Length::Fill)
                .into()
            }
            InspectorTab::Io => {
                let json_bytes = self.json_content.text().len() as u64;
                column![
                    row![
                        self.rtl_checkbox("ضغط JSON", self.json_minified, Message::JsonMinifiedToggled),
                        self.rtl_checkbox("حقول mp3quran", self.full_fields, Message::FullFieldsToggled),
                        Space::with_width(Length::Fill),
                        text(format_bytes(json_bytes)).size(11.5).color(p.muted),
                    ]
                    .spacing(8)
                    .align_y(alignment::Vertical::Center),
                    text_editor(&self.json_content)
                        .on_action(Message::JsonAction)
                        .height(Length::Fill),
                    row![
                        self.btn_soft(ar("جلب توقيت mp3quran", ui).size(11.5), [6, 9], Message::FetchPublishedTiming),
                        self.btn_ghost(ar("استيراد ملف", ui).size(11.5), [6, 9], Message::ImportFromFile),
                        self.btn_ghost(ar("استيراد نص", ui).size(11.5), [6, 9], Message::ImportFromText),
                        Space::with_width(Length::Fill),
                        self.btn_ghost(ar("نسخ", ui).size(11.5), [6, 9], Message::CopyJson),
                        self.btn_primary(ar("تنزيل", ui).size(11.5), [6, 9], Some(Message::DownloadJson)),
                    ]
                    .spacing(5)
                    .align_y(alignment::Vertical::Center),
                ]
                .spacing(8)
                .height(Length::Fill)
                .into()
            }
            InspectorTab::Audio => {
                let ready_count = tl
                    .iter()
                    .filter(|r| match (r.start, r.end) {
                        (Some(s), Some(e)) => e > s && (r.ayah > 0 || self.export_opts.include_intro),
                        _ => false,
                    })
                    .count();
                let fmt_row = row![
                    self.seg_btn(AudioFormat::Copy.label(), self.export_opts.format == AudioFormat::Copy, Message::ExportFormatPicked(AudioFormat::Copy)),
                    self.seg_btn(AudioFormat::M4a.label(), self.export_opts.format == AudioFormat::M4a, Message::ExportFormatPicked(AudioFormat::M4a)),
                    self.seg_btn(AudioFormat::Opus.label(), self.export_opts.format == AudioFormat::Opus, Message::ExportFormatPicked(AudioFormat::Opus)),
                    self.seg_btn(AudioFormat::Mp3.label(), self.export_opts.format == AudioFormat::Mp3, Message::ExportFormatPicked(AudioFormat::Mp3)),
                    Space::with_width(Length::Fill),
                    ar("الصيغة", ui).size(12).color(p.muted),
                ]
                .spacing(4)
                .align_y(alignment::Vertical::Center);

                let br_row = row![
                    self.seg_btn("128k", self.export_opts.bitrate_kbps == 128, Message::ExportBitratePicked(128)),
                    self.seg_btn("64k", self.export_opts.bitrate_kbps == 64, Message::ExportBitratePicked(64)),
                    self.seg_btn("48k", self.export_opts.bitrate_kbps == 48, Message::ExportBitratePicked(48)),
                    self.seg_btn("32k", self.export_opts.bitrate_kbps == 32, Message::ExportBitratePicked(32)),
                    self.seg_btn("24k", self.export_opts.bitrate_kbps == 24, Message::ExportBitratePicked(24)),
                    Space::with_width(Length::Fill),
                    ar("الجودة", ui).size(12).color(p.muted),
                ]
                .spacing(4)
                .align_y(alignment::Vertical::Center);

                let naming_row = row![
                    self.seg_btn("001.ext", self.export_opts.naming == NamingMode::AyahOnly, Message::ExportNamingPicked(NamingMode::AyahOnly)),
                    self.seg_btn("020001.ext", self.export_opts.naming == NamingMode::SurahAyah, Message::ExportNamingPicked(NamingMode::SurahAyah)),
                    Space::with_width(Length::Fill),
                    ar("تسمية الآيات", ui).size(12).color(p.muted),
                ]
                .spacing(5)
                .align_y(alignment::Vertical::Center);

                let opts_row = row![
                    self.rtl_checkbox("تضمين الافتتاح (000)", self.export_opts.include_intro, Message::ExportIntroToggled),
                    Space::with_width(Length::Fill),
                    self.rtl_checkbox("صوت أحادي (Mono)", self.export_opts.mono, Message::ExportMonoToggled),
                ]
                .align_y(alignment::Vertical::Center);

                let busy = self.ai_running || self.export_running || self.audio_path.is_none();
                let split_btn = self
                    .btn_primary(
                        ar(format!("تقسيم السورة ({ready_count} مقطع)"), ui).size(13),
                        [10, 14],
                        if busy || ready_count == 0 {
                            None
                        } else {
                            Some(Message::StartSplitAudio)
                        },
                    )
                    .width(Length::Fill);

                let compress_btn = if busy {
                    self.btn_primary(ar("ضغط ملف السورة كاملًا", ui).size(12.5), [9, 14], None)
                        .width(Length::Fill)
                } else {
                    self.btn_soft(
                        ar("ضغط ملف السورة كاملًا", ui).size(12.5),
                        [9, 14],
                        Message::StartCompressAudio,
                    )
                    .width(Length::Fill)
                };

                column![
                    container(
                        column![
                            ar("تقسيم القرآن بالآية وضغط الصوت (ffmpeg)", ui).size(13),
                            ar(
                                if self.audio_path.is_some() {
                                    format!("السورة {} · {} آية جاهزة للتصدير", self.surah, ready_count)
                                } else {
                                    "حمّل التلاوة أولًا لتفعيل التصدير والضغط.".into()
                                },
                                ui,
                            )
                            .size(11.5)
                            .color(p.muted),
                        ]
                        .align_x(alignment::Horizontal::Right)
                        .spacing(4)
                        .width(Length::Fill)
                    )
                    .padding(10)
                    .style(move |_| container::Style {
                        background: Some(p.panel2.into()),
                        border: border::Border {
                            color: p.line,
                            width: 1.0,
                            radius: 9.0.into(),
                        },
                        ..Default::default()
                    }),
                    fmt_row,
                    br_row,
                    naming_row,
                    opts_row,
                    Space::with_height(Length::Fill),
                    split_btn,
                    compress_btn,
                ]
                .spacing(10)
                .height(Length::Fill)
                .into()
            }
        };

        let insp_col = self
            .card(column![insp_tabs, insp_body].spacing(8).height(Length::Fill), 10)
            .width(Length::FillPortion(4))
            .height(Length::Fill);

        let workspace = row![insp_col, studio_col]
            .spacing(12)
            .height(Length::Fill)
            .align_y(alignment::Vertical::Top);

        let mut main_col = column![header, source_panel].spacing(8).height(Length::Fill);

        if let Some(ap) = &self.ai_progress {
            let is_err = ap.err.is_some();
            let bar_pct = ap.pct.unwrap_or(35) as f32;
            let action_btn: Element<'_, Message> = if self.ai_running {
                self.btn_danger(ar("■ إيقاف", ui).size(11.5), [3, 9], Message::ToggleAiAlign)
                    .into()
            } else if self.export_running {
                self.btn_danger(ar("■ إيقاف", ui).size(11.5), [3, 9], Message::CancelExport)
                    .into()
            } else {
                self.btn_ghost(text("×").size(12), [3, 8], Message::DismissAiProgress)
                    .into()
            };
            let ai_banner = container(
                column![
                    row![
                        action_btn,
                        text(ap.pct.map(|v| format!("{v}%")).unwrap_or_else(|| "…".into()))
                            .size(12.5)
                            .color(if is_err { p.danger } else { p.accent }),
                        Space::with_width(Length::Fill),
                        ar(&ap.text, ui)
                            .size(12.5)
                            .color(if is_err { p.danger } else { p.ink }),
                    ]
                    .spacing(8)
                    .align_y(alignment::Vertical::Center),
                    progress_bar(0.0..=100.0, bar_pct).height(7),
                ]
                .spacing(5),
            )
            .width(Length::Fill)
            .padding([8, 12])
            .style(move |_| container::Style {
                background: Some(if is_err { p.danger_soft } else { p.soft }.into()),
                border: border::Border {
                    color: if is_err { p.danger } else { p.accent },
                    width: 1.5,
                    radius: 10.0.into(),
                },
                ..Default::default()
            });
            main_col = main_col.push(ai_banner);
        }

        main_col = main_col.push(workspace);

        let base_ui: Element<'_, Message> = container(main_col)
            .padding([10, 14])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(p.bg.into()),
                text_color: Some(p.ink),
                ..Default::default()
            })
            .into();

        if let Some(kind) = self.open_selector {
            let (title, total, current_idx) = match kind {
                SelectorKind::Reciter => ("اختر القارئ", self.reciter_opts.len(), self.selected_reciter),
                SelectorKind::Moshaf => ("اختر المصحف / الرواية", self.moshaf_opts.len(), self.selected_moshaf),
                SelectorKind::Surah => ("اختر السورة", self.surah_opts.len(), self.surah),
                SelectorKind::Counting => (
                    "اختر العدّ",
                    self.counting_opts.len(),
                    if self.counting == Counting::Hafs { 0 } else { 1 },
                ),
            };
            let filtered = self.filtered_selector_options(kind);
            let shown = filtered.len();

            let mut list_col = column![].spacing(4);
            if filtered.is_empty() {
                list_col = list_col.push(
                    container(ar("لا توجد نتائج مطابقة", ui).size(13).color(p.muted))
                        .padding(20)
                        .width(Length::Fill)
                        .center_x(Length::Fill),
                );
            } else {
                for (i, opt) in filtered.into_iter().enumerate() {
                    let is_current = opt.idx == current_idx;
                    let is_hl = i == self.selector_highlight;
                    let idx = opt.idx;
                    let row_btn = button(
                        row![
                            text(if is_current { "✓" } else { "" })
                                .size(13)
                                .shaping(text::Shaping::Advanced)
                                .color(if is_hl { p.accent_ink } else { p.accent }),
                            Space::with_width(Length::Fill),
                            ar(&opt.raw_label, ui).size(13),
                        ]
                        .align_y(alignment::Vertical::Center),
                    )
                    .width(Length::Fill)
                    .padding([8, 12])
                    .on_press(Message::SelectorPick(kind, idx))
                    .style(move |_, status| {
                        let bg = if is_hl {
                            p.accent
                        } else if status == button::Status::Hovered || is_current {
                            p.soft
                        } else {
                            p.panel2
                        };
                        let fg = if is_hl { p.accent_ink } else { p.ink };
                        button::Style {
                            background: Some(bg.into()),
                            text_color: fg,
                            border: border::Border {
                                color: if is_current && !is_hl { p.accent } else { p.line },
                                width: if is_current && !is_hl { 1.0 } else { 0.5 },
                                radius: 8.0.into(),
                            },
                            ..Default::default()
                        }
                    });
                    list_col = list_col.push(row_btn);
                }
            }

            let mut clear_btn = button(ar("مسح", ui).size(12))
                .padding([7, 10])
                .style(move |_, status| button::Style {
                    background: Some(if status == button::Status::Hovered { p.soft } else { p.panel2 }.into()),
                    text_color: if status == button::Status::Disabled { p.muted } else { p.ink },
                    border: border::Border {
                        color: p.line,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                });
            if !self.selector_query.is_empty() {
                clear_btn = clear_btn.on_press(Message::ClearSelectorQuery);
            }
            let search_row = row![
                clear_btn,
                self.styled_input(
                    &rtl("ابحث بالاسم أو الرقم…"),
                    &self.selector_query,
                    Message::SelectorQueryChanged,
                    13.0,
                    [8, 12],
                    Length::Fill,
                )
                .align_x(alignment::Horizontal::Right)
                .id(search_input_id())
                .on_submit(Message::SelectorSubmit),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center);

            let popup_card = container(
                column![
                    row![
                        self.btn_ghost(text("×").size(13), [4, 9], Message::CloseSelector),
                        container(ar(format!("{shown} من {total}"), ui).size(11.5).color(p.muted))
                            .padding([4, 10])
                            .style(move |_| container::Style {
                                background: Some(p.panel2.into()),
                                border: border::Border {
                                    color: p.line,
                                    width: 1.0,
                                    radius: 999.0.into(),
                                },
                                ..Default::default()
                            }),
                        Space::with_width(Length::Fill),
                        ar(title, ui).size(14.5),
                    ]
                    .spacing(8)
                    .align_y(alignment::Vertical::Center),
                    search_row,
                    scrollable(list_col).height(Length::Fixed(330.0)),
                ]
                .spacing(10),
            )
            .width(Length::Fixed(520.0))
            .padding(14)
            .style(move |_| container::Style {
                background: Some(p.panel.into()),
                border: border::Border {
                    color: p.accent,
                    width: 1.5,
                    radius: 14.0.into(),
                },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
                    offset: Vector::new(0.0, 10.0),
                    blur_radius: 26.0,
                },
                ..Default::default()
            });

            let backdrop = mouse_area(
                container(Space::new(Length::Fill, Length::Fill)).style(|_| container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.28).into()),
                    ..Default::default()
                }),
            )
            .on_press(Message::CloseSelector);

            let popup_layer = container(mouse_area(popup_card).on_press(Message::Noop))
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .padding(Padding {
                    top: 138.0,
                    right: 0.0,
                    bottom: 24.0,
                    left: 0.0,
                });

            stack![base_ui, backdrop, popup_layer].into()
        } else {
            base_ui
        }
    }
}

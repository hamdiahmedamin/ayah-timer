mod align;
mod audio;
mod denoise;
mod export_audio;
mod quran;
mod settings;
mod icons;
mod view;
mod timing;
mod waveform;

use iced::event::{self, Event};
use iced::keyboard::{self, key::Named, Key};
use iced::widget::{scrollable, text, text_editor, text_input};
use iced::{Color, Font, Subscription, Task, Theme};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use align::{auto_heuristic, ensure_ai_model, run_ai_alignment, AiModel, AiProgress};
use audio::{analyze_audio_file, play_once, scan_surah_folder, AudioAnalysis, AudioPlayer};
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
use settings::{default_output_dir, Settings};
use waveform::WaveEvent;

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon-128.png");
pub(crate) const APP_ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");

fn main() -> iced::Result {
    let preloaded = load_cached_fonts_sync();
    let mut app_builder = iced::application("Ayah Timer", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1340.0, 860.0),
            icon: iced::window::icon::from_file_data(APP_ICON_PNG, None).ok(),
            #[cfg(target_os = "linux")]
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "ayah-timer".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        // ponytail: global UI scale instead of bumping ~100 hardcoded text sizes
        .scale_factor(|_| 1.15)
        .antialiasing(true);
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

/// RTL paragraph (RLM-led) for multi-line Arabic: cosmic-text right-aligns every
/// wrapped line itself. Use with Length::Fill and the default (Left) align_x —
/// iced 0.13 would shift an RTL buffer a second time for Center/Right.
fn rtl_para(s: &str) -> String {
    format!("\u{200F}{s}")
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
    Folder,
    Url,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drawer {
    Source,
    Tools,
    Settings,
}

#[derive(Debug, Clone, Copy)]
struct Palette {
    bg: Color,
    panel: Color,
    panel2: Color,
    ink: Color,
    muted: Color,
    line: Color,
    /// Background of segmented-control tracks.
    track: Color,
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
            muted: Color::from_rgb8(0x4f, 0x5c, 0x58),
            line: Color::from_rgb8(0xdc, 0xd4, 0xc1),
            track: Color::from_rgb8(0xef, 0xea, 0xdc),
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
            line: Color::from_rgb8(0x2c, 0x39, 0x37),
            track: Color::from_rgb8(0x12, 0x19, 0x18),
            accent: Color::from_rgb8(0x6f, 0xd1, 0xb6),
            accent_hover: Color::from_rgb8(0x88, 0xdd, 0xc6),
            accent_ink: Color::from_rgb8(0x0b, 0x14, 0x12),
            soft: Color::from_rgb8(0x1b, 0x2c, 0x28),
            wave: Color::from_rgb8(0x5a, 0x86, 0x7c),
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
    ZoomStep(f32),
    FollowToggled(bool),
    OpenDrawer(Drawer),
    CloseDrawer,
    SelectAyah(usize),
    ToggleJsonEditor,
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
    JsonSaved(Option<Result<PathBuf, String>>),
    PickOutputDir,
    OutputDirPicked(Option<PathBuf>),
    ResetOutputDir,
    OpenOutputDir,
    AskEachTimeToggled(bool),
    DenoiseStrengthPicked(u32),
    LoadCleanedToggled(bool),
    StartDenoise,
    DenoiseDestPicked(Option<PathBuf>),
    DenoiseFinished(Result<(PathBuf, u64), String>),
    PickAudioFolder,
    AudioFolderPicked(Option<PathBuf>),
    /// Denoise ~10 s from the playhead and play it.
    PreviewDenoise,
    PreviewReady(Result<PathBuf, String>),
    /// Play the same original span for A/B.
    PreviewOriginal,
    StopPreview,
    AiModelPicked(AiModel),
    WesternDigitsToggled(bool),
}

const PREVIEW_MS: u64 = 10_000;

/// Output file destination: straight into the output folder, or a save dialog
/// (opened there) when "ask each time" is on or the folder can't be created.
async fn dest_file(dir: PathBuf, name: String, ask: bool) -> Option<PathBuf> {
    if !ask && tokio::fs::create_dir_all(&dir).await.is_ok() {
        return Some(dir.join(name));
    }
    rfd::AsyncFileDialog::new()
        .set_directory(&dir)
        .set_file_name(&name)
        .save_file()
        .await
        .map(|h| h.path().to_path_buf())
}

/// Same as [`dest_file`] for a folder (`dir/sub` unless asking).
async fn dest_dir(dir: PathBuf, sub: String, ask: bool) -> Option<PathBuf> {
    let target = dir.join(sub);
    if !ask && tokio::fs::create_dir_all(&target).await.is_ok() {
        return Some(target);
    }
    rfd::AsyncFileDialog::new()
        .set_directory(&dir)
        .pick_folder()
        .await
        .map(|h| h.path().to_path_buf())
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
    drawer: Option<Drawer>,
    /// Row expanded in the ayah list; None = follow the current ayah.
    selected_ayah: Option<usize>,
    show_json_editor: bool,

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
    settings: Settings,
    /// Surah → file for the "folder" source.
    folder_files: BTreeMap<usize, PathBuf>,
    preview_busy: bool,
    preview_stop: Option<Arc<AtomicBool>>,
    preview_start: u64,
}

impl App {
    fn new(preloaded: LoadedFonts) -> (Self, Task<Message>) {
        let need_fonts = preloaded.ui_family.is_none()
            || preloaded.quran_hafs_family.is_none()
            || preloaded.quran_warsh_family.is_none()
            || preloaded.quran_qalun_family.is_none();

        let settings = Settings::load();
        let folder_files = settings.audio_folder.as_deref().map(scan_surah_folder).unwrap_or_default();
        let mut app = Self {
            dark: settings.dark,
            fonts: preloaded,
            ui_font: Font::DEFAULT,
            hafs_font: Font::DEFAULT,
            warsh_font: Font::DEFAULT,
            qalun_font: Font::DEFAULT,
            player: AudioPlayer::new(),
            source_tab: SourceTab::Mp3Quran,
            riwayah: Riwayah::All,
            drawer: None,
            selected_ayah: None,
            show_json_editor: false,
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
            mushaf_font_size: 22.0,
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
            settings,
            folder_files,
            preview_busy: false,
            preview_stop: None,
            preview_start: 0,
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
            SourceTab::File | SourceTab::Folder => None,
        }
    }

    fn folder_surahs(&self) -> Vec<usize> {
        self.folder_files.keys().copied().collect()
    }

    fn store_key(&self) -> String {
        let src = match self.source_tab {
            SourceTab::File => self
                .file_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            SourceTab::Folder => self
                .folder_files
                .get(&self.surah)
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

    /// A UI number in the chosen digit style.
    fn num(&self, n: usize) -> String {
        if self.settings.western_digits {
            n.to_string()
        } else {
            ar_digits(n)
        }
    }

    fn stop_preview(&mut self) {
        if let Some(f) = self.preview_stop.take() {
            f.store(true, Ordering::Relaxed);
        }
    }

    /// True while a denoise preview is playing.
    fn preview_playing(&self) -> bool {
        self.preview_stop.as_ref().is_some_and(|f| !f.load(Ordering::Relaxed))
    }

    fn say(&mut self, msg: impl Into<String>, err: bool) {
        self.status = msg.into();
        self.status_err = err;
    }

    /// Scroll the ayah list so `ayah` is near the top.
    /// ponytail: row height ~ text length, so a char-weighted fraction is close enough;
    /// switch to measured offsets if long surahs drift.
    fn scroll_inspector_to(&self, ayah: usize) -> Task<Message> {
        let weights: Vec<f32> = match &self.surah_text {
            Some(st) => st.texts.iter().map(|t| 60.0 + t.chars().count() as f32).collect(),
            None => vec![1.0; self.count()],
        };
        let total: f32 = weights.iter().sum();
        if total <= 0.0 {
            return Task::none();
        }
        let before: f32 = weights.iter().take(ayah.saturating_sub(2)).sum();
        scrollable::snap_to(
            table_scroll_id(),
            scrollable::RelativeOffset { x: 0.0, y: (before / total).clamp(0.0, 1.0) },
        )
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
                if tab == SourceTab::Folder {
                    self.surah_opts = Self::surah_options(Some(&self.folder_surahs()));
                } else if tab != SourceTab::Mp3Quran {
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
                            let changed = idx != self.surah;
                            if matches!(self.source_tab, SourceTab::Mp3Quran | SourceTab::Folder) && changed {
                                self.clear_audio();
                                self.say("", false);
                            }
                            self.surah = idx;
                            self.surah_text = None;
                            self.restore_work();
                            let text = self.load_text_task();
                            if self.source_tab == SourceTab::Folder && changed {
                                return Task::batch([text, self.update(Message::LoadAudio)]);
                            }
                            return text;
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
                if self.drawer == Some(Drawer::Source) {
                    self.drawer = None;
                }
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
                    SourceTab::Folder => match self.folder_files.get(&self.surah).cloned() {
                        Some(p) => (Some(p.clone()), None, p.display().to_string()),
                        None => {
                            self.say("لا يوجد ملف لهذه السورة في المجلد.", true);
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
                        self.selected_ayah = None;
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
                let model = self.settings.ai_model;

                return Task::perform(
                    async move {
                        let slot_a = Arc::clone(&slot);
                        let report_a = move |p: AiProgress| {
                            if let Ok(mut g) = slot_a.lock() {
                                *g = Some(p);
                            }
                        };
                        let (model_path, vocab) = ensure_ai_model(model, &cancel, &report_a)
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
            Message::ZoomStep(dir) => {
                let step = (self.zoom / 6.0).round().max(1.0);
                self.zoom = (self.zoom + dir * step).clamp(1.0, 60.0);
                if self.follow {
                    self.reveal_now();
                }
            }
            Message::FollowToggled(f) => self.follow = f,
            Message::OpenDrawer(d) => {
                self.drawer = if self.drawer == Some(d) { None } else { Some(d) };
            }
            Message::CloseDrawer => self.drawer = None,
            Message::ToggleJsonEditor => self.show_json_editor = !self.show_json_editor,
            Message::SelectAyah(a) => {
                self.selected_ayah = Some(a);
                if let Some(st) = self.starts.get(a).copied().flatten() {
                    return self.update(Message::Seek(st));
                }
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
                let (dir, ask) = (self.settings.output_dir.clone(), self.settings.ask_each_time);
                let sub = format!("{:03}", self.surah);
                return Task::perform(dest_dir(dir, sub, ask), Message::SplitDirPicked);
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
                let (dir, ask) = (self.settings.output_dir.clone(), self.settings.ask_each_time);
                return Task::perform(dest_file(dir, def_name, ask), Message::CompressDestPicked);
            }
            Message::CompressDestPicked(Some(dest)) => {
                let Some(audio_path) = self.audio_path.clone() else {
                    return Task::none();
                };
                if dest == audio_path {
                    self.say("لا يمكن الكتابة فوق الملف الأصلي.", true);
                    return Task::none();
                }
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
                let (dir, ask) = (self.settings.output_dir.clone(), self.settings.ask_each_time);
                return Task::perform(
                    async move {
                        let dest = dest_file(dir, default_name, ask).await?;
                        Some(tokio::fs::write(&dest, json).await.map(|_| dest).map_err(|e| e.to_string()))
                    },
                    Message::JsonSaved,
                );
            }
            Message::JsonSaved(None) => {}
            Message::JsonSaved(Some(Ok(p))) => self.say(format!("حُفظ JSON في {}", p.display()), false),
            Message::JsonSaved(Some(Err(e))) => self.say(format!("تعذّر حفظ JSON: {e}"), true),
            Message::PickOutputDir => {
                let dir = self.settings.output_dir.clone();
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_directory(&dir)
                            .pick_folder()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::OutputDirPicked,
                );
            }
            Message::OutputDirPicked(Some(dir)) => {
                self.settings.output_dir = dir;
                self.settings.save();
            }
            Message::OutputDirPicked(None) => {}
            Message::ResetOutputDir => {
                self.settings.output_dir = default_output_dir();
                self.settings.save();
            }
            Message::OpenOutputDir => {
                let dir = self.settings.output_dir.clone();
                let _ = std::fs::create_dir_all(&dir);
                // ponytail: xdg-open only (Linux); add `open`/`explorer` if the app ships elsewhere
                if let Err(e) = std::process::Command::new("xdg-open").arg(&dir).spawn() {
                    self.say(format!("تعذّر فتح المجلد: {e}"), true);
                }
            }
            Message::AskEachTimeToggled(v) => {
                self.settings.ask_each_time = v;
                self.settings.save();
            }
            Message::DenoiseStrengthPicked(db) => {
                self.settings.denoise_atten_db = db as f32;
                self.settings.save();
            }
            Message::LoadCleanedToggled(v) => {
                self.settings.load_cleaned = v;
                self.settings.save();
            }
            Message::StartDenoise => {
                if self.ai_running || self.export_running {
                    return Task::none();
                }
                let Some(audio_path) = self.audio_path.clone() else {
                    self.say("حمّل التلاوة أولًا.", true);
                    return Task::none();
                };
                let stem = match self.source_tab {
                    SourceTab::File | SourceTab::Folder => audio_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("{:03}", self.surah)),
                    _ => format!("{:03}", self.surah),
                };
                let name = format!("{stem}_clean.{}", self.export_opts.format.ext());
                let (dir, ask) = (self.settings.output_dir.clone(), self.settings.ask_each_time);
                return Task::perform(dest_file(dir, name, ask), Message::DenoiseDestPicked);
            }
            Message::DenoiseDestPicked(None) => {}
            Message::DenoiseDestPicked(Some(dest)) => {
                let Some(audio_path) = self.audio_path.clone() else {
                    return Task::none();
                };
                if dest == audio_path {
                    self.say("لا يمكن الكتابة فوق الملف الأصلي.", true);
                    return Task::none();
                }
                let opts = self.export_opts;
                let atten = self.settings.denoise_atten_db;
                let duration_ms = self.duration_ms;
                let cancel = Arc::new(AtomicBool::new(false));
                self.ai_cancel = Arc::clone(&cancel);
                self.export_running = true;
                let slot = Arc::clone(&self.ai_progress_slot);
                let init_prog = AiProgress {
                    text: "جارٍ تنظيف الصوت (DeepFilterNet)…".into(),
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
                            denoise::denoise_file(
                                &audio_path,
                                &dest,
                                denoise::codec_for(&opts),
                                atten,
                                None,
                                duration_ms,
                                &cancel,
                                &report,
                            )
                                .map(|bytes| (dest, bytes))
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::DenoiseFinished,
                );
            }
            Message::DenoiseFinished(Ok((path, bytes))) => {
                let msg = format!("تم تنظيف الصوت ({}) في {}", format_bytes(bytes), path.display());
                let task = self.update(Message::ExportFinished(Ok(msg)));
                if !self.settings.load_cleaned {
                    return task;
                }
                // Latency is compensated, so the marks carry over to the cleaned file.
                let starts = self.starts.clone();
                self.source_tab = SourceTab::File;
                self.file_path = Some(path);
                save_work(&self.store_key(), &starts);
                return Task::batch([task, self.update(Message::LoadAudio)]);
            }
            Message::DenoiseFinished(Err(e)) => return self.update(Message::ExportFinished(Err(e))),
            Message::PreviewDenoise => {
                let Some(audio_path) = self.audio_path.clone() else {
                    self.say("حمّل التلاوة أولًا.", true);
                    return Task::none();
                };
                if self.preview_busy {
                    return Task::none();
                }
                self.stop_preview();
                // Start from the playhead, but keep a full 10 s window inside the file.
                let start = self.now_ms.min(self.duration_ms.saturating_sub(PREVIEW_MS));
                self.preview_start = start;
                self.preview_busy = true;
                let atten = self.settings.denoise_atten_db;
                let dest = cache_dir().join(format!("denoise_preview_{}.wav", atten as u32));
                self.say("جارٍ تحضير المعاينة…", false);
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            if let Some(d) = dest.parent() {
                                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                            }
                            denoise::denoise_file(
                                &audio_path,
                                &dest,
                                denoise::codec_wav(),
                                atten,
                                Some((start, PREVIEW_MS)),
                                PREVIEW_MS,
                                &AtomicBool::new(false),
                                &|_| {},
                            )
                            .map(|_| dest)
                            .map_err(|e| e.to_string())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::PreviewReady,
                );
            }
            Message::PreviewReady(res) => {
                self.preview_busy = false;
                match res {
                    Ok(path) => {
                        if self.player.is_playing() {
                            self.player.toggle();
                        }
                        self.preview_stop = Some(play_once(path));
                        self.say(format!("معاينة بعد التنظيف من {}", fmt_ms(self.preview_start)), false);
                    }
                    Err(e) => self.say(format!("تعذّرت المعاينة: {e}"), true),
                }
            }
            Message::PreviewOriginal => {
                self.stop_preview();
                if self.player.has_audio() {
                    let start = self.preview_start;
                    self.player.play_span(start, Some(start + PREVIEW_MS));
                    self.say(format!("الصوت الأصلي من {}", fmt_ms(start)), false);
                }
            }
            Message::StopPreview => self.stop_preview(),
            Message::AiModelPicked(m) => {
                self.settings.ai_model = m;
                self.settings.save();
            }
            Message::WesternDigitsToggled(v) => {
                self.settings.western_digits = v;
                self.settings.save();
            }
            Message::PickAudioFolder => {
                let start = self.settings.audio_folder.clone();
                return Task::perform(
                    async move {
                        let mut d = rfd::AsyncFileDialog::new();
                        if let Some(dir) = start {
                            d = d.set_directory(dir);
                        }
                        d.pick_folder().await.map(|h| h.path().to_path_buf())
                    },
                    Message::AudioFolderPicked,
                );
            }
            Message::AudioFolderPicked(None) => {}
            Message::AudioFolderPicked(Some(dir)) => {
                self.folder_files = scan_surah_folder(&dir);
                self.settings.audio_folder = Some(dir);
                self.settings.save();
                self.surah_opts = Self::surah_options(Some(&self.folder_surahs()));
                let n = self.folder_files.len();
                if n == 0 {
                    self.say("لم يُعثر على ملفات مرقّمة بالسور (مثل 001.mp3) في المجلد.", true);
                    return Task::none();
                }
                self.say(format!("رُبط {n} ملفًا بالسور."), false);
                if !self.folder_files.contains_key(&self.surah) {
                    self.surah = *self.folder_files.keys().next().unwrap();
                    self.surah_text = None;
                    self.restore_work();
                    return Task::batch([self.load_text_task(), self.update(Message::LoadAudio)]);
                }
                self.restore_work();
                return self.update(Message::LoadAudio);
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
            Message::ToggleTheme => {
                self.dark = !self.dark;
                self.settings.dark = self.dark;
                self.settings.save();
            }
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
                if self.drawer.is_some() {
                    if key == Key::Named(Named::Escape) {
                        self.drawer = None;
                    }
                    return Task::none();
                }
                if mods.control() || mods.alt() || mods.logo() {
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

}

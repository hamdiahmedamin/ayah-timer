mod align;
mod audio;
mod quran;
mod timing;
mod waveform;

use iced::keyboard::{self, key::Named, Key};
use iced::widget::{
    button, checkbox, column, combo_box, container, progress_bar, row, scrollable, slider,
    text, text_editor, text_input, Space,
};
use iced::{
    alignment, border, Color, Element, Font, Length, Subscription, Task, Theme,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use align::{auto_heuristic, ensure_ai_model, run_ai_alignment, AiProgress};
use audio::{analyze_audio_file, AudioAnalysis, AudioPlayer};
use quran::{
    ar_digits, cache_dir, ensure_fonts, fetch_published_timings, fetch_reciters_and_timed_reads,
    fetch_surah_text, fmt_ms, http_client, norm_url, Counting, LoadedFonts, Moshaf, Reciter,
    Riwayah, SurahText, TextEdition, SURAHS,
};
use timing::{
    build_timeline, current_ayah, export_json, import_json, load_work, next_unmarked, save_work,
    set_start_clamped,
};
use waveform::{view_waveform, WaveColors, WaveEvent, WaveformProgram};

fn main() -> iced::Result {
    iced::application("Ayah Timer · مؤقِّت الآيات", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size((1340.0, 860.0))
        .run_with(App::new)
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
}

#[derive(Debug, Clone)]
struct Palette {
    bg: Color,
    panel: Color,
    panel2: Color,
    ink: Color,
    muted: Color,
    line: Color,
    accent: Color,
    accent_ink: Color,
    soft: Color,
    wave: Color,
    wave_played: Color,
    mark: Color,
    now: Color,
    verse: Color,
    danger: Color,
}

impl Palette {
    fn light() -> Self {
        Self {
            bg: Color::from_rgb8(0xf4, 0xf1, 0xe8),
            panel: Color::from_rgb8(0xff, 0xfd, 0xf8),
            panel2: Color::from_rgb8(0xf8, 0xf5, 0xec),
            ink: Color::from_rgb8(0x1a, 0x22, 0x20),
            muted: Color::from_rgb8(0x5d, 0x6b, 0x67),
            line: Color::from_rgb8(0xe2, 0xdc, 0xcb),
            accent: Color::from_rgb8(0x1f, 0x64, 0x54),
            accent_ink: Color::WHITE,
            soft: Color::from_rgb8(0xe2, 0xef, 0xe9),
            wave: Color::from_rgb8(0x9c, 0xb9, 0xaf),
            wave_played: Color::from_rgb8(0x1f, 0x64, 0x54),
            mark: Color::from_rgb8(0xc8, 0x4b, 0x14),
            now: Color::from_rgb8(0xc9, 0x92, 0x0e),
            verse: Color::from_rgba8(0x1f, 0x64, 0x54, 0.11),
            danger: Color::from_rgb8(0xb4, 0x23, 0x18),
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
            accent_ink: Color::from_rgb8(0x0b, 0x14, 0x12),
            soft: Color::from_rgb8(0x1b, 0x2c, 0x28),
            wave: Color::from_rgb8(0x38, 0x52, 0x4d),
            wave_played: Color::from_rgb8(0x6f, 0xd1, 0xb6),
            mark: Color::from_rgb8(0xf0, 0x8a, 0x5d),
            now: Color::from_rgb8(0xec, 0xc4, 0x5a),
            verse: Color::from_rgba8(0x6f, 0xd1, 0xb6, 0.12),
            danger: Color::from_rgb8(0xf9, 0x70, 0x66),
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
    ReciterPicked(String),
    MoshafPicked(String),
    SurahPicked(String),
    CountingPicked(Counting),
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
    player: AudioPlayer,
    source_tab: SourceTab,
    riwayah: Riwayah,
    inspector_tab: InspectorTab,

    all_reciters: Vec<Reciter>,
    reciters: Vec<Reciter>,
    timed_reads: HashMap<String, u64>,
    selected_reciter: usize,
    selected_moshaf: usize,

    reciter_combo: combo_box::State<String>,
    moshaf_combo: combo_box::State<String>,
    surah_combo: combo_box::State<String>,
    counting_combo: combo_box::State<Counting>,
    rate_combo: combo_box::State<String>,

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
    fn new() -> (Self, Task<Message>) {
        let mut app = Self {
            dark: false,
            fonts: LoadedFonts::default(),
            player: AudioPlayer::new(),
            source_tab: SourceTab::Mp3Quran,
            riwayah: Riwayah::All,
            inspector_tab: InspectorTab::Table,
            all_reciters: Vec::new(),
            reciters: Vec::new(),
            timed_reads: HashMap::new(),
            selected_reciter: 0,
            selected_moshaf: 0,
            reciter_combo: combo_box::State::new(vec!["جارٍ التحميل…".into()]),
            moshaf_combo: combo_box::State::new(vec![]),
            surah_combo: combo_box::State::new(Self::surah_labels(None)),
            counting_combo: combo_box::State::new(vec![Counting::Hafs, Counting::Madani]),
            rate_combo: combo_box::State::new(
                ["0.75", "1", "1.25", "1.5", "2"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
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
            table_inputs: HashMap::new(),
            json_content: text_editor::Content::new(),
            status: String::new(),
            status_err: false,
            ai_running: false,
            ai_cancel: Arc::new(AtomicBool::new(false)),
            ai_progress_slot: Arc::new(Mutex::new(None)),
            ai_progress: None,
        };
        app.restore_work();

        let t_fonts = Task::perform(ensure_fonts(), Message::FontsLoaded);
        let t_rec = Task::perform(
            async { fetch_reciters_and_timed_reads().await.map_err(|e| e.to_string()) },
            Message::RecitersLoaded,
        );
        let t_txt = app.load_text_task();
        (app, Task::batch([t_fonts, t_rec, t_txt]))
    }

    fn surah_labels(allowed: Option<&[usize]>) -> Vec<String> {
        SURAHS
            .iter()
            .enumerate()
            .filter(|(i, _)| allowed.map(|a| a.contains(&(i + 1))).unwrap_or(true))
            .map(|(i, name)| format!("{}. {name}", i + 1))
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

    fn ui_font(&self) -> Font {
        self.fonts
            .ui_family
            .as_deref()
            .map(|n| Font::with_name(Box::leak(n.to_string().into_boxed_str())))
            .unwrap_or(Font::DEFAULT)
    }

    fn quran_font(&self) -> Font {
        let fam = match self.text_edition() {
            TextEdition::Warsh => self.fonts.quran_warsh_family.as_deref(),
            TextEdition::Qalun => self.fonts.quran_qalun_family.as_deref(),
            TextEdition::Hafs => None,
        }
        .or(self.fonts.quran_hafs_family.as_deref());
        fam.map(|n| Font::with_name(Box::leak(n.to_string().into_boxed_str())))
            .unwrap_or_else(|| self.ui_font())
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
        let j = export_json(&self.starts, self.count(), self.duration_ms, pages, self.full_fields);
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
        let labels: Vec<String> = self.reciters.iter().map(|r| r.name.clone()).collect();
        self.reciter_combo = combo_box::State::new(labels);

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
            self.moshaf_combo = combo_box::State::new(vec![]);
            return Task::none();
        };
        let labels: Vec<String> = r
            .moshaf
            .iter()
            .map(|m| {
                let timed = self.timed_reads.contains_key(&norm_url(&m.server));
                if timed {
                    format!("{} · له توقيت", m.name)
                } else {
                    m.name.clone()
                }
            })
            .collect();
        self.moshaf_combo = combo_box::State::new(labels);

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
            self.surah_combo = combo_box::State::new(Self::surah_labels(Some(&avail)));
            if !avail.contains(&self.surah) {
                self.surah = avail[0];
            }
        } else {
            self.surah_combo = combo_box::State::new(Self::surah_labels(None));
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

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::FontsLoaded(loaded) => {
                let mut tasks = Vec::new();
                for bytes in loaded.font_bytes.iter().cloned() {
                    tasks.push(iced::font::load(bytes).map(Message::FontRegistered));
                }
                self.fonts = loaded;
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
                self.reciter_combo = combo_box::State::new(vec!["تعذّر تحميل القرّاء".into()]);
                self.source_tab = SourceTab::File;
                self.surah_combo = combo_box::State::new(Self::surah_labels(None));
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
                    self.surah_combo = combo_box::State::new(Self::surah_labels(None));
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
            Message::ReciterPicked(name) => {
                if let Some(i) = self.reciters.iter().position(|r| r.name == name) {
                    self.selected_reciter = i;
                    return self.fill_moshaf();
                }
            }
            Message::MoshafPicked(label) => {
                if let Some(r) = self.reciters.get(self.selected_reciter) {
                    if let Some(i) = r.moshaf.iter().position(|m| label.starts_with(&m.name)) {
                        self.selected_moshaf = i;
                        return self.picked_moshaf();
                    }
                }
            }
            Message::SurahPicked(label) => {
                if let Some(num) = label.split('.').next().and_then(|s| s.trim().parse::<usize>().ok()) {
                    if (1..=114).contains(&num) {
                        if self.source_tab == SourceTab::Mp3Quran && num != self.surah {
                            self.clear_audio();
                            self.say("", false);
                        }
                        self.surah = num;
                        self.surah_text = None;
                        self.restore_work();
                        return self.load_text_task();
                    }
                }
            }
            Message::CountingPicked(c) => {
                self.counting = c;
                self.surah_text = None;
                self.restore_work();
                return self.load_text_task();
            }
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
                if self.player.has_audio() {
                    let pos = self.player.position_ms();
                    self.now_ms = pos.min(self.duration_ms.max(pos));
                    if self.player.is_playing() && self.follow {
                        self.reveal_now();
                    }
                    let cur = current_ayah(&self.starts, self.now_ms);
                    if cur != self.last_cur {
                        self.last_cur = cur;
                    }
                }
                if self.ai_running {
                    let prog = self.ai_progress_slot.lock().ok().and_then(|g| g.clone());
                    if let Some(p) = prog {
                        let txt = p.text.clone();
                        let err = p.err.is_some();
                        self.ai_progress = Some(p);
                        self.say(txt, err);
                    }
                }
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
                    if self.follow {
                        self.reveal_now();
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
            Message::InspectorChanged(tab) => self.inspector_tab = tab,
            Message::PlayAyah(a) => {
                if self.player.has_audio() {
                    let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
                    let tl = build_timeline(&self.starts, self.count(), self.duration_ms, pages);
                    if let Some(row) = tl.get(a) {
                        if let Some(st) = row.start {
                            self.now_ms = st;
                            self.player.play_span(st, row.end);
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
                let json = export_json(&self.starts, self.count(), self.duration_ms, pages, self.full_fields);
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
                let json = export_json(&self.starts, self.count(), self.duration_ms, pages, self.full_fields);
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
                    _ => {}
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick);
        let keys = keyboard::on_key_press(|key, mods| Some(Message::KeyPressed(key, mods)));
        Subscription::batch([tick, keys])
    }

    fn card<'a>(&self, content: impl Into<Element<'a, Message>>) -> container::Container<'a, Message> {
        let p = self.pal();
        container(content).padding(14).style(move |_| container::Style {
            background: Some(p.panel.into()),
            border: border::Border {
                color: p.line,
                width: 1.0,
                radius: 14.0.into(),
            },
            ..Default::default()
        })
    }

    fn seg_btn<'a>(&self, label: &'a str, active: bool, msg: Message) -> Element<'a, Message> {
        let p = self.pal();
        button(text(label).size(13))
            .padding([6, 12])
            .on_press(msg)
            .style(move |_, _| button::Style {
                background: Some(if active { p.accent } else { p.panel2 }.into()),
                text_color: if active { p.accent_ink } else { p.muted },
                border: border::Border {
                    color: p.line,
                    width: if active { 0.0 } else { 1.0 },
                    radius: 8.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn view(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font();
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

        // Header
        let header = row![
            column![
                text("مؤقِّت الآيات").size(22).font(ui),
                text("تطبيق سطح مكتب أصلي بالـ Rust لتوقيت الآيات بالذكاء الاصطناعي")
                    .size(12)
                    .color(p.muted)
                    .font(ui),
            ]
            .spacing(2),
            Space::with_width(Length::Fill),
            container(
                column![
                    row![
                        text(format!("مُعلَّم {done_count} من {n_ayahs}")).size(12).font(ui),
                        text(format!("{}%", pct.round() as u32)).size(12).color(p.accent),
                    ]
                    .spacing(10),
                    progress_bar(0.0..=100.0, pct).height(5),
                ]
                .spacing(4)
                .width(Length::Fixed(170.0))
            )
            .padding([6, 12])
            .style(move |_| container::Style {
                background: Some(p.panel.into()),
                border: border::Border {
                    color: p.line,
                    width: 1.0,
                    radius: 11.0.into(),
                },
                ..Default::default()
            }),
            button(text(if self.dark { "☀ فاتح" } else { "◐ داكن" }).size(13).font(ui))
                .padding([8, 12])
                .on_press(Message::ToggleTheme),
        ]
        .spacing(12)
        .align_y(alignment::Vertical::Center);

        // Source panel
        let source_tabs = row![
            self.seg_btn("من mp3quran", self.source_tab == SourceTab::Mp3Quran, Message::SourceTabChanged(SourceTab::Mp3Quran)),
            self.seg_btn("ملف من جهازك", self.source_tab == SourceTab::File, Message::SourceTabChanged(SourceTab::File)),
            self.seg_btn("رابط", self.source_tab == SourceTab::Url, Message::SourceTabChanged(SourceTab::Url)),
        ]
        .spacing(4);

        let status_txt = text(&self.status)
            .size(12)
            .color(if self.status_err { p.danger } else { p.muted })
            .font(ui);

        let riw_bar = row![
            self.seg_btn("الكل", self.riwayah == Riwayah::All, Message::RiwayahChanged(Riwayah::All)),
            self.seg_btn("حفص", self.riwayah == Riwayah::Hafs, Message::RiwayahChanged(Riwayah::Hafs)),
            self.seg_btn("ورش", self.riwayah == Riwayah::Warsh, Message::RiwayahChanged(Riwayah::Warsh)),
            self.seg_btn("قالون", self.riwayah == Riwayah::Qalun, Message::RiwayahChanged(Riwayah::Qalun)),
        ]
        .spacing(4);

        let cur_rec_label = self.reciters.get(self.selected_reciter).map(|r| r.name.clone());
        let cur_moshaf_label = self.current_moshaf().map(|m| m.name.clone());
        let cur_surah_label = Some(format!("{}. {}", self.surah, SURAHS[self.surah - 1]));

        let src_inputs: Element<'_, Message> = match self.source_tab {
            SourceTab::Mp3Quran => row![
                column![text("الرواية").size(11).color(p.muted).font(ui), riw_bar].spacing(4),
                column![
                    text("القارئ").size(11).color(p.muted).font(ui),
                    combo_box(&self.reciter_combo, "ابحث عن قارئ…", cur_rec_label.as_ref(), Message::ReciterPicked)
                        .width(Length::FillPortion(2))
                ]
                .spacing(4)
                .width(Length::FillPortion(2)),
                column![
                    text("المصحف/الرواية").size(11).color(p.muted).font(ui),
                    combo_box(&self.moshaf_combo, "اختر الرواية…", cur_moshaf_label.as_ref(), Message::MoshafPicked)
                        .width(Length::FillPortion(2))
                ]
                .spacing(4)
                .width(Length::FillPortion(2)),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Bottom)
            .into(),
            SourceTab::File => row![
                button(text("📂 اختر ملفًا صوتيًا (MP3 / WAV / FLAC / M4A)…").size(13).font(ui))
                    .padding([9, 14])
                    .on_press(Message::PickAudioFile),
                text(
                    self.file_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "لم يُختر ملف".into())
                )
                .size(12)
                .color(p.muted)
                .font(ui),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center)
            .into(),
            SourceTab::Url => row![
                text_input("https://…/001.mp3", &self.url_input)
                    .on_input(Message::UrlChanged)
                    .width(Length::Fill),
            ]
            .into(),
        };

        let surah_row = row![
            src_inputs,
            column![
                text("السورة").size(11).color(p.muted).font(ui),
                combo_box(&self.surah_combo, "ابحث عن سورة…", cur_surah_label.as_ref(), Message::SurahPicked)
                    .width(Length::Fixed(185.0))
            ]
            .spacing(4),
            column![
                text("العدّ").size(11).color(p.muted).font(ui),
                combo_box(&self.counting_combo, "العدّ", Some(&self.counting), Message::CountingPicked)
                    .width(Length::Fixed(175.0))
            ]
            .spacing(4),
            container(text(format!("{n_ayahs} آية")).size(12).font(ui))
                .padding([8, 12])
                .style(move |_| container::Style {
                    background: Some(p.panel2.into()),
                    border: border::Border {
                        color: p.line,
                        width: 1.0,
                        radius: 999.0.into(),
                    },
                    ..Default::default()
                }),
            button(text("تحميل التلاوة").size(13).font(ui))
                .padding([9, 16])
                .on_press(Message::LoadAudio),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Bottom);

        let source_panel = self.card(
            column![
                row![
                    text("١. التلاوة والسورة").size(14).font(ui),
                    source_tabs,
                    Space::with_width(Length::Fill),
                    status_txt,
                ]
                .spacing(10)
                .align_y(alignment::Vertical::Center),
                surah_row,
            ]
            .spacing(10),
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
                if ed == TextEdition::Hafs {
                    format!("{txt} ﴿{cur}﴾")
                } else {
                    format!("{txt}\u{00A0}{}", ar_digits(cur))
                }
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
                        text(format!("التالي (آية {nx}):")).size(13).color(p.accent).font(ui),
                        text(txt)
                            .size(22)
                            .font(qfont)
                            .shaping(text::Shaping::Advanced),
                    ]
                    .spacing(10)
                    .align_y(alignment::Vertical::Center),
                )
                .padding([8, 12])
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(p.soft.into()),
                    border: border::Border {
                        color: p.line,
                        width: 0.0,
                        radius: 10.0.into(),
                    },
                    ..Default::default()
                })
                .into()
            } else {
                Space::with_height(0).into()
            }
        } else {
            container(text("اكتمل تعليم جميع الآيات ✓").size(13).color(p.accent).font(ui))
                .padding([8, 12])
                .width(Length::Fill)
                .center_x(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(p.soft.into()),
                    border: border::Border {
                        color: p.line,
                        width: 0.0,
                        radius: 10.0.into(),
                    },
                    ..Default::default()
                })
                .into()
        };

        let verse_stage = self.card(
            column![
                row![
                    container(
                        text(if cur == 0 { "الافتتاح".into() } else { format!("الآية {cur}") })
                            .size(12)
                            .color(p.accent)
                            .font(ui)
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
                    Space::with_width(Length::Fill),
                    text(riw_meta).size(12).color(p.muted).font(ui),
                ]
                .align_y(alignment::Vertical::Center),
                container(
                    text(cur_text)
                        .size(30)
                        .font(qfont)
                        .shaping(text::Shaping::Advanced)
                        .align_x(alignment::Horizontal::Center)
                )
                .padding([12, 16])
                .width(Length::Fill)
                .center_x(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(p.panel2.into()),
                    border: border::Border {
                        radius: 12.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                next_banner,
            ]
            .spacing(10),
        );

        // Waveform & transport studio
        let wave_el = view_waveform(WaveformProgram {
            peaks: &self.peaks,
            duration_ms: self.duration_ms,
            starts: &self.starts,
            now_ms: self.now_ms,
            current_ayah: cur,
            zoom: self.zoom,
            scroll_frac: self.scroll_frac,
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
        })
        .map(Message::Wave);

        let zoom_bar = row![
            text("تكبير").size(12).color(p.muted).font(ui),
            button(text("−").size(13)).padding([2, 8]).on_press(Message::ZoomStep(-1.0)),
            slider(1.0..=60.0, self.zoom, Message::ZoomChanged).step(1.0).width(Length::Fixed(140.0)),
            button(text("+").size(13)).padding([2, 8]).on_press(Message::ZoomStep(1.0)),
            text(format!("×{}", self.zoom.round() as u32)).size(12),
            Space::with_width(12),
            text("تمرير").size(12).color(p.muted).font(ui),
            slider(0.0..=1.0, self.scroll_frac, Message::ScrollChanged)
                .step(0.005)
                .width(Length::Fill),
            checkbox("متابعة رأس التشغيل", self.follow).on_toggle(Message::FollowToggled).size(16),
        ]
        .spacing(8)
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
        let mut mark_btn = button(text(mark_label).size(15).font(ui)).padding([12, 20]).width(Length::Fill);
        if self.duration_ms > 0 && next.is_some() {
            mark_btn = mark_btn.on_press(Message::MarkNext);
        }

        let transport = row![
            mark_btn,
            button(text("−٥ث").size(13).font(ui))
                .padding([10, 12])
                .on_press(Message::Seek(self.now_ms.saturating_sub(5000))),
            button(
                text(if self.player.is_playing() { "❚❚ إيقاف" } else { "▶ تشغيل" })
                    .size(13)
                    .font(ui)
            )
            .padding([10, 14])
            .on_press(Message::TogglePlay),
            button(text("+٥ث").size(13).font(ui))
                .padding([10, 12])
                .on_press(Message::Seek(self.now_ms + 5000)),
            container(text(format!("{} / {}", fmt_ms(self.now_ms), fmt_ms(self.duration_ms))).size(13))
                .padding([8, 10])
                .style(move |_| container::Style {
                    background: Some(p.bg.into()),
                    border: border::Border {
                        color: p.line,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

        let ai_btn_lbl = if self.ai_running { "⏹ إيقاف AI" } else { "🤖 توقيت AI" };
        let tools_row = row![
            button(text(ai_btn_lbl).size(13).font(ui)).padding([7, 12]).on_press(Message::ToggleAiAlign),
            button(text("✨ اقتراح تلقائي").size(13).font(ui)).padding([7, 12]).on_press(Message::AutoHeuristic),
            button(text("↶ تراجع").size(13).font(ui)).padding([7, 12]).on_press(Message::Undo),
            button(text("مسح الكل").size(13).font(ui)).padding([7, 12]).on_press(Message::ResetMarks),
            Space::with_width(Length::Fill),
            text("السرعة").size(12).color(p.muted).font(ui),
            combo_box(&self.rate_combo, "1", Some(&self.rate_str), Message::RatePicked).width(Length::Fixed(72.0)),
            text("تأخّر(ms)").size(12).color(p.muted).font(ui),
            text_input("250", &self.reaction_str)
                .on_input(Message::ReactionChanged)
                .width(Length::Fixed(64.0)),
            checkbox("التصاق بالسكتة", self.snap).on_toggle(Message::SnapToggled).size(16),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

        let mut studio_col = column![
            verse_stage,
            self.card(
                column![
                    row![
                        text("٢. الاستماع والتعليم").size(14).font(ui),
                        Space::with_width(Length::Fill),
                        text("Space: علِّم · P: تشغيل · ←/→: ٥ث · Z: تراجع").size(11).color(p.muted).font(ui),
                    ],
                    wave_el,
                    zoom_bar,
                    seek_bar,
                    transport,
                    tools_row,
                ]
                .spacing(10),
            )
        ]
        .spacing(12)
        .width(Length::FillPortion(6));

        if let Some(ap) = &self.ai_progress {
            let bar_pct = ap.pct.unwrap_or(40) as f32;
            let bar_card = container(
                column![
                    row![
                        text(&ap.text).size(12.5).font(ui),
                        Space::with_width(Length::Fill),
                        text(ap.pct.map(|v| format!("{v}%")).unwrap_or_else(|| "…".into())).size(12.5).color(p.accent),
                    ],
                    progress_bar(0.0..=100.0, bar_pct).height(6),
                ]
                .spacing(6),
            )
            .padding(10)
            .style(move |_| container::Style {
                background: Some(p.panel2.into()),
                border: border::Border {
                    color: if ap.err.is_some() { p.danger } else { p.accent },
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..Default::default()
            });
            studio_col = studio_col.push(bar_card);
        }

        // Inspector column
        let insp_tabs = row![
            self.seg_btn("الآيات", self.inspector_tab == InspectorTab::Table, Message::InspectorChanged(InspectorTab::Table)),
            self.seg_btn("المصحف", self.inspector_tab == InspectorTab::Mushaf, Message::InspectorChanged(InspectorTab::Mushaf)),
            self.seg_btn("JSON واستيراد", self.inspector_tab == InspectorTab::Io, Message::InspectorChanged(InspectorTab::Io)),
            Space::with_width(Length::Fill),
            button(text("⬇ JSON").size(12).font(ui)).padding([5, 10]).on_press(Message::DownloadJson),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center);

        let pages_slice = self.surah_text.as_ref().map(|t| t.pages.as_slice());
        let tl = build_timeline(&self.starts, n_ayahs, self.duration_ms, pages_slice);

        let insp_body: Element<'_, Message> = match self.inspector_tab {
            InspectorTab::Table => {
                let mut rows_col = column![].spacing(4);
                for r in tl {
                    let a = r.ayah;
                    let is_now = a == cur;
                    let start_el: Element<'_, Message> = if a == 0 {
                        text("0").size(12).width(Length::Fixed(72.0)).into()
                    } else {
                        let val = self.table_inputs.get(&a).cloned().unwrap_or_default();
                        text_input("—", &val)
                            .on_input(move |s| Message::TableInputChanged(a, s))
                            .on_submit(Message::TableInputSubmit(a))
                            .width(Length::Fixed(74.0))
                            .size(12)
                            .into()
                    };
                    let end_txt = r.end.map(fmt_ms).unwrap_or_else(|| "–".into());
                    let dur_txt = match (r.start, r.end) {
                        (Some(s), Some(e)) => format!("{:.1}s", (e.saturating_sub(s)) as f64 / 1000.0),
                        _ => "–".into(),
                    };

                    let mut acts = row![
                        button(text("▶").size(11)).padding([3, 6]).on_press(Message::PlayAyah(a)),
                    ]
                    .spacing(3);
                    if a > 0 {
                        acts = acts
                            .push(button(text("⌖").size(11)).padding([3, 6]).on_press(Message::SetAyahToNow(a)))
                            .push(button(text("−").size(11)).padding([3, 6]).on_press(Message::NudgeAyah(a, -100)))
                            .push(button(text("+").size(11)).padding([3, 6]).on_press(Message::NudgeAyah(a, 100)))
                            .push(button(text("✕").size(11)).padding([3, 6]).on_press(Message::ClearAyah(a)));
                    }

                    let row_box = container(
                        row![
                            text(if a == 0 { "الافتتاح".into() } else { a.to_string() })
                                .size(12)
                                .width(Length::Fixed(46.0))
                                .font(ui),
                            start_el,
                            text(end_txt).size(12).width(Length::Fixed(62.0)),
                            text(dur_txt).size(12).width(Length::Fixed(46.0)),
                            Space::with_width(Length::Fill),
                            acts,
                        ]
                        .spacing(6)
                        .align_y(alignment::Vertical::Center),
                    )
                    .padding([4, 6])
                    .style(move |_| container::Style {
                        background: Some(if is_now { p.verse } else { Color::TRANSPARENT }.into()),
                        border: border::Border {
                            color: p.line,
                            width: 0.5,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    });
                    rows_col = rows_col.push(row_box);
                }
                scrollable(rows_col).height(Length::Fixed(520.0)).into()
            }
            InspectorTab::Mushaf => {
                let mut m_col = column![].spacing(8);
                if let Some(st) = &self.surah_text {
                    if self.surah != 9 && !(self.surah == 1 && ed == TextEdition::Hafs) {
                        m_col = m_col.push(
                            container(
                                text("بِسْمِ اللَّهِ الرَّحْمَٰنِ الرَّحِيمِ")
                                    .size(24)
                                    .font(qfont)
                                    .shaping(text::Shaping::Advanced),
                            )
                            .width(Length::Fill)
                            .center_x(Length::Fill),
                        );
                    }
                    for (i, t) in st.texts.iter().enumerate() {
                        let a = i + 1;
                        let is_now = a == cur;
                        let marked = self.starts.get(a).copied().flatten().is_some();
                        let line_txt = if ed == TextEdition::Hafs {
                            format!("{t} ﴿{a}﴾")
                        } else {
                            format!("{t}\u{00A0}{}", ar_digits(a))
                        };
                        let item = button(
                            text(line_txt)
                                .size(24)
                                .font(qfont)
                                .shaping(text::Shaping::Advanced)
                                .color(if marked { p.ink } else { p.muted }),
                        )
                        .width(Length::Fill)
                        .padding([6, 10])
                        .on_press(Message::PlayAyah(a))
                        .style(move |_, _| button::Style {
                            background: Some(if is_now { p.verse } else { p.panel2 }.into()),
                            text_color: p.ink,
                            border: border::Border {
                                color: if is_now { p.accent } else { p.line },
                                width: if is_now { 1.0 } else { 0.5 },
                                radius: 8.0.into(),
                            },
                            ..Default::default()
                        });
                        m_col = m_col.push(item);
                    }
                } else {
                    m_col = m_col.push(text("جارٍ تحميل نص المصحف…").size(13).color(p.muted).font(ui));
                }
                scrollable(m_col).height(Length::Fixed(520.0)).into()
            }
            InspectorTab::Io => column![
                row![
                    button(text("⬇ تنزيل JSON").size(12).font(ui)).padding([7, 12]).on_press(Message::DownloadJson),
                    button(text("نسخ").size(12).font(ui)).padding([7, 12]).on_press(Message::CopyJson),
                    checkbox("حقول mp3quran الكاملة", self.full_fields)
                        .on_toggle(Message::FullFieldsToggled)
                        .size(16),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center),
                text_editor(&self.json_content)
                    .on_action(Message::JsonAction)
                    .height(Length::Fixed(340.0)),
                row![
                    button(text("استيراد من النص").size(12).font(ui)).padding([7, 12]).on_press(Message::ImportFromText),
                    button(text("📂 استيراد ملف JSON").size(12).font(ui)).padding([7, 12]).on_press(Message::ImportFromFile),
                    button(text("جلب توقيت mp3quran").size(12).font(ui)).padding([7, 12]).on_press(Message::FetchPublishedTiming),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .into(),
        };

        let insp_col = self
            .card(column![insp_tabs, insp_body].spacing(10))
            .width(Length::FillPortion(4));

        let workspace = row![studio_col, insp_col].spacing(14).align_y(alignment::Vertical::Top);

        container(column![header, source_panel, workspace].spacing(12))
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(p.bg.into()),
                text_color: Some(p.ink),
                ..Default::default()
            })
            .into()
    }
}

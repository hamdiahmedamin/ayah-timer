//! All layout and styling. Layout (RTL): top bar · [ayah list | studio] · status bar,
//! with source / tools / settings drawers and the searchable selector as overlays.

use iced::overlay::menu;
use iced::widget::{
    button, checkbox, column, container, mouse_area, pick_list, progress_bar, row, scrollable,
    slider, stack, svg, text, text_editor, text_input, tooltip, Space,
};
use iced::{alignment, border, Color, Element, Length, Padding, Shadow, Vector};

use crate::icons::{icon, Icon};
use crate::settings::default_output_dir;
use crate::timing::{build_timeline, current_ayah, next_unmarked};
use crate::waveform::{view_waveform, WaveColors, WaveformProgram};
use crate::*;

/// Type scale (logical px; the window also runs at `scale_factor` 1.15).
mod ty {
    pub const CAPTION: f32 = 12.0;
    pub const BODY: f32 = 13.5;
    pub const TITLE: f32 = 16.0;
    pub const VERSE: f32 = 30.0;
    pub const NEXT: f32 = 19.0;
}

const R: f32 = 10.0; // control radius
const R_CARD: f32 = 14.0;

impl App {
    // ───────────────────────── base widgets ─────────────────────────

    pub(crate) fn card<'a>(&self, content: impl Into<Element<'a, Message>>, pad: u16) -> container::Container<'a, Message> {
        let p = self.pal();
        let dark = self.dark;
        container(content).padding(pad).style(move |_| container::Style {
            background: Some(p.panel.into()),
            border: border::Border {
                color: p.line,
                width: if dark { 1.0 } else { 0.0 },
                radius: R_CARD.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba(0.1, 0.13, 0.12, 0.07),
                offset: Vector::new(0.0, 1.0),
                blur_radius: 3.0,
            },
            ..Default::default()
        })
    }

    fn sub_card<'a>(&self, content: impl Into<Element<'a, Message>>) -> container::Container<'a, Message> {
        let p = self.pal();
        container(content).padding(12).width(Length::Fill).style(move |_| container::Style {
            border: border::Border {
                color: p.line,
                width: 1.0,
                radius: 12.0.into(),
            },
            ..Default::default()
        })
    }

    pub(crate) fn btn_primary<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Option<Message>,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        let b = button(label).padding(pad).style(move |_, status| {
            let bg = match status {
                button::Status::Hovered | button::Status::Pressed => p.accent_hover,
                button::Status::Disabled => p.track,
                button::Status::Active => p.accent,
            };
            button::Style {
                background: Some(bg.into()),
                text_color: if status == button::Status::Disabled { p.muted } else { p.accent_ink },
                border: border::Border {
                    color: p.line,
                    width: if status == button::Status::Disabled { 1.0 } else { 0.0 },
                    radius: R.into(),
                },
                ..Default::default()
            }
        });
        match msg {
            Some(m) => b.on_press(m),
            None => b,
        }
    }

    pub(crate) fn btn_soft<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Option<Message>,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        let b = button(label).padding(pad).style(move |_, status| button::Style {
            background: Some(
                match status {
                    button::Status::Hovered | button::Status::Pressed => p.verse,
                    button::Status::Disabled => p.track,
                    button::Status::Active => p.soft,
                }
                .into(),
            ),
            text_color: if status == button::Status::Disabled { p.muted } else { p.accent },
            border: border::Border {
                color: Color { a: 0.35, ..p.accent },
                width: if status == button::Status::Disabled { 0.0 } else { 1.0 },
                radius: R.into(),
            },
            ..Default::default()
        });
        match msg {
            Some(m) => b.on_press(m),
            None => b,
        }
    }

    pub(crate) fn btn_ghost<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Message,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        button(label).padding(pad).on_press(msg).style(move |_, status| button::Style {
            background: Some(
                if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    p.track
                } else {
                    p.panel
                }
                .into(),
            ),
            text_color: p.ink,
            border: border::Border {
                color: p.line,
                width: 1.0,
                radius: R.into(),
            },
            ..Default::default()
        })
    }

    pub(crate) fn btn_danger<'a>(
        &self,
        label: impl Into<Element<'a, Message>>,
        pad: impl Into<Padding>,
        msg: Message,
    ) -> button::Button<'a, Message> {
        let p = self.pal();
        button(label).padding(pad).on_press(msg).style(move |_, status| button::Style {
            background: Some(
                if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    p.danger_soft
                } else {
                    p.panel
                }
                .into(),
            ),
            text_color: p.danger,
            border: border::Border {
                color: Color { a: 0.45, ..p.danger },
                width: 1.0,
                radius: R.into(),
            },
            ..Default::default()
        })
    }

    /// Icon-only button with a tooltip (shows the shortcut where there is one).
    fn icon_btn<'a>(&self, i: Icon, tip: &'a str, msg: Message, size: f32) -> Element<'a, Message> {
        let p = self.pal();
        self.with_tip(
            self.btn_ghost(icon(i, size, p.ink), [8, 9], msg).into(),
            tip,
        )
    }

    fn with_tip<'a>(&self, content: Element<'a, Message>, tip: &'a str) -> Element<'a, Message> {
        let p = self.pal();
        tooltip(
            content,
            container(ar(tip, self.ui_font).size(ty::CAPTION).color(p.accent_ink)).padding([4, 8]),
            tooltip::Position::Bottom,
        )
        .gap(4)
        .style(move |_| container::Style {
            background: Some(p.ink.into()),
            border: border::Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    /// Icon + label laid out RTL (icon on the right).
    fn ic_label<'a>(&self, i: Icon, label: impl AsRef<str>, color: Color, size: f32) -> Element<'a, Message> {
        row![ar(label, self.ui_font).size(size), icon(i, size + 2.0, color)]
            .spacing(7)
            .align_y(alignment::Vertical::Center)
            .into()
    }

    pub(crate) fn styled_input<'a>(
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
                    color: if status == text_input::Status::Focused { p.accent } else { p.line },
                    width: 1.0,
                    radius: 8.0.into(),
                },
                icon: p.muted,
                placeholder: p.muted,
                value: p.ink,
                selection: p.soft,
            })
    }

    /// One option of a segmented control; put several in [`App::seg_group`].
    pub(crate) fn seg_btn<'a>(&self, label: &'a str, active: bool, msg: Message) -> Element<'a, Message> {
        let p = self.pal();
        // ASCII labels (001.ext, 48k, MP3) must not go through the RTL wrapper.
        let lbl = if label.is_ascii() { text(label).font(self.ui_font) } else { ar(label, self.ui_font) };
        button(lbl.size(ty::BODY))
            .padding([5, 12])
            .on_press(msg)
            .style(move |_, status| button::Style {
                background: Some(
                    if active {
                        p.accent
                    } else if status == button::Status::Hovered {
                        p.panel
                    } else {
                        Color::TRANSPARENT
                    }
                    .into(),
                ),
                text_color: if active { p.accent_ink } else { p.ink },
                border: border::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn seg_group<'a>(&self, items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
        let p = self.pal();
        container(row(items).spacing(2))
            .padding(3)
            .style(move |_| container::Style {
                background: Some(p.track.into()),
                border: border::Border {
                    radius: R.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn styled_pick<'a, T>(
        &self,
        options: &'a [T],
        selected: Option<T>,
        on_selected: impl Fn(T) -> Message + 'a,
        width: Length,
    ) -> pick_list::PickList<'a, T, &'a [T], T, Message>
    where
        T: ToString + PartialEq + Clone + 'a,
    {
        let p = self.pal();
        pick_list(options, selected, on_selected)
            .font(self.ui_font)
            .text_shaping(text::Shaping::Advanced)
            .text_size(ty::BODY)
            .padding([9, 10])
            .width(width)
            .style(move |_, status| pick_list::Style {
                text_color: p.ink,
                placeholder_color: p.muted,
                handle_color: p.muted,
                background: p.panel.into(),
                border: border::Border {
                    color: if status == pick_list::Status::Hovered || status == pick_list::Status::Opened {
                        p.accent
                    } else {
                        p.line
                    },
                    width: 1.0,
                    radius: R.into(),
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

    /// Field that opens the searchable selector overlay.
    fn selector_trigger<'a>(&self, label: &str, kind: SelectorKind, width: Length) -> Element<'a, Message> {
        let p = self.pal();
        let active = self.open_selector == Some(kind);
        button(
            row![
                icon(Icon::ChevronDown, 15.0, p.muted),
                Space::with_width(Length::Fill),
                ar(label, self.ui_font).size(ty::BODY),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center),
        )
        .width(width)
        .padding([10, 12])
        .on_press(Message::OpenSelector(kind))
        .style(move |_, status| button::Style {
            background: Some(p.panel2.into()),
            text_color: p.ink,
            border: border::Border {
                color: if active || status == button::Status::Hovered { p.accent } else { p.line },
                width: 1.0,
                radius: R.into(),
            },
            ..Default::default()
        })
        .into()
    }

    pub(crate) fn rtl_checkbox<'a>(
        &self,
        label: &'a str,
        checked: bool,
        on_toggle: impl Fn(bool) -> Message + 'a,
    ) -> Element<'a, Message> {
        let p = self.pal();
        row![
            ar(label, self.ui_font).size(ty::BODY),
            checkbox("", checked).on_toggle(on_toggle).size(16).style(move |_, status| {
                let checked = match status {
                    checkbox::Status::Active { is_checked }
                    | checkbox::Status::Hovered { is_checked }
                    | checkbox::Status::Disabled { is_checked } => is_checked,
                };
                checkbox::Style {
                    background: (if checked { p.accent } else { p.panel }).into(),
                    icon_color: p.accent_ink,
                    border: border::Border {
                        color: if checked { p.accent } else { p.muted },
                        width: 1.5,
                        radius: 4.0.into(),
                    },
                    text_color: None,
                }
            }),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .into()
    }

    pub(crate) fn styled_scrollable<'a>(&self, content: impl Into<Element<'a, Message>>) -> scrollable::Scrollable<'a, Message> {
        let p = self.pal();
        scrollable(content)
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(3).scroller_width(3).spacing(3),
            ))
            .style(move |_, status| {
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

    fn pill<'a>(&self, content: impl Into<Element<'a, Message>>, bg: Color) -> Element<'a, Message> {
        container(content)
            .padding([3, 10])
            .style(move |_| container::Style {
                background: Some(bg.into()),
                border: border::Border {
                    radius: 999.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Label above a field, both right-aligned.
    fn field<'a>(&self, label: &'a str, input: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
        let p = self.pal();
        column![ar(label, self.ui_font).size(ty::CAPTION).color(p.muted), input.into()]
            .spacing(5)
            .align_x(alignment::Horizontal::Right)
            .width(Length::Fill)
            .into()
    }

    // ───────────────────────── view ─────────────────────────

    pub(crate) fn view(&self) -> Element<'_, Message> {
        let p = self.pal();
        let body = row![self.ayah_list(), self.studio()]
            .spacing(14)
            .height(Length::Fill);
        let base: Element<'_, Message> = container(
            column![
                self.top_bar(),
                container(body).padding([12, 16]).height(Length::Fill),
                self.status_bar(),
            ]
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(p.bg.into()),
            text_color: Some(p.ink),
            ..Default::default()
        })
        .into();

        let mut layers = vec![base];
        if let Some(d) = self.drawer {
            layers.push(self.backdrop(Message::CloseDrawer));
            layers.push(self.drawer_layer(d));
        }
        if let Some(kind) = self.open_selector {
            layers.push(self.backdrop(Message::CloseSelector));
            layers.push(self.selector_layer(kind));
        }
        if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            stack(layers).into()
        }
    }

    fn backdrop<'a>(&self, on_press: Message) -> Element<'a, Message> {
        mouse_area(
            container(Space::new(Length::Fill, Length::Fill)).style(|_| container::Style {
                background: Some(Color::from_rgba(0.06, 0.08, 0.08, 0.38).into()),
                ..Default::default()
            }),
        )
        .on_press(on_press)
        .into()
    }

    fn done_count(&self) -> usize {
        (1..=self.count())
            .filter(|&a| self.starts.get(a).copied().flatten().is_some())
            .count()
    }

    fn source_summary(&self) -> String {
        let name = |pb: &PathBuf| pb.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let s = match self.source_tab {
            SourceTab::Mp3Quran => {
                let rec = self.reciter_opts.get(self.selected_reciter).map(|o| o.raw_label.as_str()).unwrap_or("");
                let mos = self.moshaf_opts.get(self.selected_moshaf).map(|o| o.raw_label.as_str()).unwrap_or("");
                let mos = mos.split(" - ").next().unwrap_or(mos);
                format!("mp3quran · {rec} · {mos}")
            }
            SourceTab::File => self.file_path.as_ref().map(name).unwrap_or_else(|| "اختر ملفًا صوتيًا".into()),
            SourceTab::Folder => match &self.settings.audio_folder {
                Some(d) => format!("مجلد {} · {} سورة", name(d), self.folder_files.len()),
                None => "اختر مجلد السور".into(),
            },
            SourceTab::Url => {
                let u = self.url_input.trim();
                if u.is_empty() { "رابط مباشر".into() } else { u.to_string() }
            }
        };
        // ponytail: char cut, iced 0.13 has no text ellipsis
        if s.chars().count() > 46 {
            format!("{}…", s.chars().take(45).collect::<String>())
        } else {
            s
        }
    }

    fn top_bar(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let n = self.count();
        let done = self.done_count();
        let cur_surah = self
            .surah_opts
            .iter()
            .find(|o| o.idx == self.surah)
            .map(|o| o.raw_label.clone())
            .unwrap_or_else(|| format!("{}", self.surah));

        let chip_style = move |_: &iced::Theme, status: button::Status| button::Style {
            background: Some(if status == button::Status::Hovered { p.track } else { p.panel2 }.into()),
            text_color: p.ink,
            border: border::Border {
                color: if status == button::Status::Hovered { p.accent } else { p.line },
                width: 1.0,
                radius: R.into(),
            },
            ..Default::default()
        };

        let surah_chip = button(
            row![
                icon(Icon::ChevronDown, 15.0, p.muted),
                ar(cur_surah, ui).size(15.0),
                ar("السورة", ui).size(ty::CAPTION).color(p.muted),
            ]
            .spacing(9)
            .align_y(alignment::Vertical::Center),
        )
        .padding([8, 12])
        .on_press(Message::OpenSelector(SelectorKind::Surah))
        .style(chip_style);

        let source_chip = button(
            row![
                icon(Icon::ChevronDown, 15.0, p.muted),
                ar(self.source_summary(), ui).size(ty::BODY),
                icon(
                    if self.source_tab == SourceTab::Folder { Icon::Folder } else { Icon::Headphones },
                    16.0,
                    p.accent
                ),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([8, 12])
        .on_press(Message::OpenDrawer(Drawer::Source))
        .style(chip_style);

        let progress = container(
            row![
                progress_bar(0.0..=(n.max(1) as f32), done as f32)
                    .width(Length::Fixed(56.0))
                    .height(6)
                    .style(move |_| progress_bar::Style {
                        background: Color { a: 0.18, ..p.accent }.into(),
                        bar: p.accent.into(),
                        border: border::Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                    }),
                ar(format!("{} من {} مُعلَّمة", self.num(done), self.num(n)), ui)
                    .size(ty::BODY)
                    .color(p.accent),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([8, 12])
        .style(move |_| container::Style {
            background: Some(p.soft.into()),
            border: border::Border {
                radius: R.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let tools_btn = self.btn_primary(
            self.ic_label(Icon::Download, "تصدير وأدوات", p.accent_ink, ty::BODY),
            [8, 14],
            Some(Message::OpenDrawer(Drawer::Tools)),
        );

        container(
            row![
                self.icon_btn(
                    if self.dark { Icon::Sun } else { Icon::Moon },
                    if self.dark { "الوضع الفاتح" } else { "الوضع الداكن" },
                    Message::ToggleTheme,
                    17.0
                ),
                self.icon_btn(Icon::Settings, "الإعدادات", Message::OpenDrawer(Drawer::Settings), 17.0),
                tools_btn,
                progress,
                Space::with_width(Length::Fill),
                source_chip,
                surah_chip,
                Space::with_width(6),
                ar("مؤقِّت الآيات", ui).size(18.0),
                svg(svg::Handle::from_memory(APP_ICON_SVG))
                    .width(Length::Fixed(30.0))
                    .height(Length::Fixed(30.0)),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([8, 16])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(p.panel.into()),
            border: border::Border {
                color: p.line,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let mut r = row![
            ar("Space علِّم · P تشغيل · ←/→ ٥ث · Z تراجع · [ ] آية", ui)
                .size(ty::CAPTION)
                .color(p.muted),
            Space::with_width(Length::Fill),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center);

        let (txt, err) = match &self.ai_progress {
            Some(ap) => (ap.text.clone(), ap.err.is_some()),
            None => (self.status.clone(), self.status_err),
        };
        if let Some(ap) = &self.ai_progress {
            let action: Element<'_, Message> = if self.ai_running {
                self.btn_danger(ar("إيقاف", ui).size(ty::CAPTION), [2, 10], Message::ToggleAiAlign).into()
            } else if self.export_running {
                self.btn_danger(ar("إيقاف", ui).size(ty::CAPTION), [2, 10], Message::CancelExport).into()
            } else {
                self.btn_ghost(icon(Icon::X, 12.0, p.muted), [3, 6], Message::DismissAiProgress).into()
            };
            r = r
                .push(action)
                .push(text(ap.pct.map(|v| format!("{v}%")).unwrap_or_else(|| "…".into())).size(ty::CAPTION).color(p.accent))
                .push(
                    progress_bar(0.0..=100.0, ap.pct.unwrap_or(35) as f32)
                        .width(Length::Fixed(160.0))
                        .height(6)
                        .style(move |_| progress_bar::Style {
                            background: p.track.into(),
                            bar: (if err { p.danger } else { p.accent }).into(),
                            border: border::Border {
                                radius: 3.0.into(),
                                ..Default::default()
                            },
                        }),
                );
        }
        let dot = container(Space::new(8, 8)).style(move |_| container::Style {
            background: Some(
                (if err {
                    p.danger
                } else if self.player.has_audio() {
                    p.accent
                } else {
                    p.muted
                })
                .into(),
            ),
            border: border::Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        r = r
            .push(ar(txt, ui).size(ty::CAPTION).color(if err { p.danger } else { p.ink }))
            .push(dot);

        container(r)
            .padding([6, 16])
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(p.panel.into()),
                border: border::Border {
                    color: p.line,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    // ───────────────────────── studio ─────────────────────────

    fn studio(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let qfont = self.quran_font();
        let n = self.count();
        let cur = current_ayah(&self.starts, self.now_ms);
        let next = next_unmarked(&self.starts, n);
        let ed = self.text_edition();
        let page = self
            .surah_text
            .as_ref()
            .and_then(|st| st.pages.get(cur.saturating_sub(1)).copied().flatten());
        let meta = match page {
            Some(pg) => format!("{} · صفحة {}", ed.ar_name(), self.num(pg as usize)),
            None => ed.ar_name().to_string(),
        };
        let cur_text = if cur == 0 {
            "الاستعاذة والبسملة".to_string()
        } else {
            match self.surah_text.as_ref().and_then(|st| st.texts.get(cur - 1)) {
                Some(t) => format!("{t} ﴿{}﴾", ar_digits(cur)),
                None => format!("الآية {cur}"),
            }
        };

        let next_line: Element<'_, Message> = match next {
            Some(nx) => {
                let t = self
                    .surah_text
                    .as_ref()
                    .and_then(|s| s.texts.get(nx - 1))
                    .map(|t| format!("{t} ﴿{}﴾", ar_digits(nx)))
                    .unwrap_or_default();
                row![
                    container(
                        text(rtl_para(&t))
                            .size(ty::NEXT)
                            .line_height(1.6)
                            .font(qfont)
                            .shaping(text::Shaping::Advanced)
                            .width(Length::Fill)
                    )
                    .width(Length::Fill)
                    .max_height(44.0)
                    .clip(true),
                    ar("التالي:", ui).size(ty::CAPTION).color(p.muted),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .into()
            }
            None => ar("اكتمل تعليم جميع الآيات ✓", ui).size(ty::BODY).color(p.accent).into(),
        };

        let verse_card = self.card(
            column![
                row![
                    self.icon_btn(Icon::ChevronLeft, "الآية التالية  ]", Message::StepAyah(1), 14.0),
                    self.icon_btn(Icon::ChevronRight, "الآية السابقة  [", Message::StepAyah(-1), 14.0),
                    Space::with_width(Length::Fill),
                    ar(meta, ui).size(ty::CAPTION).color(p.muted),
                    self.pill(
                        ar(if cur == 0 { "الافتتاح".to_string() } else { format!("الآية {}", self.num(cur)) }, ui)
                            .size(ty::CAPTION)
                            .color(p.accent),
                        p.soft
                    ),
                ]
                .spacing(6)
                .align_y(alignment::Vertical::Center),
                container(
                    self.styled_scrollable(
                        text(rtl_para(&cur_text))
                            .size(ty::VERSE)
                            .line_height(1.8)
                            .font(qfont)
                            .shaping(text::Shaping::Advanced)
                            .width(Length::Fill)
                    )
                )
                .max_height(118.0),
                next_line,
            ]
            .spacing(4),
            14,
        );

        // Waveform with zoom / follow overlay.
        let wave = view_waveform(
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
                    bg: p.panel2,
                    line: p.line,
                    muted: p.muted,
                    wave: p.wave,
                    wave_played: p.wave_played,
                    verse: p.verse,
                    mark: p.mark,
                    now: p.now,
                },
            },
            Length::Fill,
        )
        .map(Message::Wave);
        let follow_btn = if self.follow {
            self.btn_soft(ar("متابعة", ui).size(ty::CAPTION), [6, 10], Some(Message::FollowToggled(false)))
        } else {
            self.btn_soft(ar("متابعة", ui).size(ty::CAPTION).color(p.muted), [6, 10], Some(Message::FollowToggled(true)))
        };
        let wave_tools = container(
            row![
                self.with_tip(follow_btn.into(), "تحريك العرض مع التشغيل"),
                self.pill(text(format!("×{}", self.zoom.round() as u32)).size(ty::CAPTION).color(p.muted), p.panel),
                self.icon_btn(Icon::Minus, "تصغير (عجلة الفأرة)", Message::ZoomStep(-1.0), 13.0),
                self.icon_btn(Icon::Plus, "تكبير (عجلة الفأرة)", Message::ZoomStep(1.0), 13.0),
            ]
            .spacing(4)
            .align_y(alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Right)
        .padding(6);
        let wave_box = container(stack![wave, wave_tools])
            .height(Length::Fill)
            .clip(true)
            .style(move |_| container::Style {
                background: Some(p.panel2.into()),
                border: border::Border {
                    radius: R.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let seek = row![
            text(fmt_ms(self.now_ms)).size(ty::CAPTION).color(p.muted),
            slider(0.0..=(self.duration_ms.max(1) as f32), self.now_ms as f32, |v| Message::Seek(v.round() as u64))
                .step(20.0_f32)
                .width(Length::Fill),
            text(fmt_ms(self.duration_ms)).size(ty::CAPTION).color(p.muted),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center);

        let mark_msg = (self.duration_ms > 0 && next.is_some()).then_some(Message::MarkNext);
        let mark_btn = self
            .btn_primary(
                row![
                    if mark_msg.is_some() {
                        self.pill(text("Space").size(ty::CAPTION).color(p.accent_ink), Color { a: 0.18, ..Color::WHITE })
                    } else {
                        self.pill(text("Space").size(ty::CAPTION).color(p.muted), p.line)
                    },
                    ar(
                        match next {
                            Some(a) => format!("علِّم بداية الآية {}", self.num(a)),
                            None => "اكتمل التعليم ✓".into(),
                        },
                        ui
                    )
                    .size(15.0),
                    icon(Icon::Flag, 18.0, if mark_msg.is_some() { p.accent_ink } else { p.muted }),
                ]
                .spacing(10)
                .align_y(alignment::Vertical::Center),
                [11, 16],
                mark_msg,
            )
            .width(Length::Fill);
        let mark_btn = container(mark_btn).width(Length::Fill).center_x(Length::Fill);

        let playing = self.player.is_playing();
        let transport = row![
            self.styled_pick(&self.rate_opts, Some(self.rate_str.clone()), Message::RatePicked, Length::Fixed(70.0)),
            self.with_tip(
                self.btn_ghost(
                    ar(format!("+{}ث", self.num(5)), ui).size(ty::BODY),
                    [11, 11],
                    Message::Seek(self.now_ms + 5000),
                )
                .into(),
                "تقديم ٥ ثوانٍ  →",
            ),
            self.with_tip(
                self.btn_soft(
                    icon(if playing { Icon::Pause } else { Icon::Play }, 20.0, p.accent),
                    [11, 18],
                    Some(Message::TogglePlay),
                )
                .into(),
                if playing { "إيقاف مؤقت  P" } else { "تشغيل  P" },
            ),
            self.with_tip(
                self.btn_ghost(
                    ar(format!("−{}ث", self.num(5)), ui).size(ty::BODY),
                    [11, 11],
                    Message::Seek(self.now_ms.saturating_sub(5000)),
                )
                .into(),
                "رجوع ٥ ثوانٍ  ←",
            ),
            mark_btn,
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

        let ai_btn = if self.ai_running {
            self.btn_danger(self.ic_label(Icon::X, "إيقاف الذكاء الاصطناعي", p.danger, ty::BODY), [7, 12], Message::ToggleAiAlign)
        } else {
            self.btn_soft(
                self.ic_label(Icon::Sparkles, "توقيت بالذكاء الاصطناعي", p.accent, ty::BODY),
                [7, 12],
                Some(Message::ToggleAiAlign),
            )
        };
        let tools = row![
            self.btn_danger(self.ic_label(Icon::Trash, "مسح الكل", p.danger, ty::BODY), [7, 12], Message::ResetMarks),
            self.with_tip(
                self.btn_ghost(self.ic_label(Icon::Undo, "تراجع", p.ink, ty::BODY), [7, 12], Message::Undo).into(),
                "Z",
            ),
            Space::with_width(Length::Fill),
            self.btn_ghost(self.ic_label(Icon::Wand, "اقتراح تلقائي", p.ink, ty::BODY), [7, 12], Message::AutoHeuristic),
            ai_btn,
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center);

        let studio_card = self
            .card(column![wave_box, seek, transport, tools].spacing(10).height(Length::Fill), 12)
            .height(Length::Fill);

        column![verse_card, studio_card]
            .spacing(12)
            .width(Length::FillPortion(62))
            .height(Length::Fill)
            .into()
    }

    // ───────────────────────── ayah list ─────────────────────────

    fn ayah_list(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let qfont = self.quran_font();
        let n = self.count();
        let cur = current_ayah(&self.starts, self.now_ms);
        let sel = self.selected_ayah.unwrap_or(cur);
        let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
        let tl = build_timeline(&self.starts, n, self.duration_ms, pages);

        let header = row![
            self.icon_btn(Icon::Minus, "تصغير الخط", Message::MushafFontDelta(-2.0), 12.0),
            self.icon_btn(Icon::Plus, "تكبير الخط", Message::MushafFontDelta(2.0), 12.0),
            self.icon_btn(Icon::Locate, "الانتقال إلى الآية الحالية", Message::JumpToCurrentAyah, 14.0),
            Space::with_width(Length::Fill),
            ar(format!("{} آية · {} مُعلَّمة", self.num(n), self.num(self.done_count())), ui)
                .size(ty::CAPTION)
                .color(p.muted),
            ar("الآيات", ui).size(ty::TITLE),
        ]
        .spacing(5)
        .align_y(alignment::Vertical::Center);

        let mut list = column![].spacing(6);
        let mut prev_pg: Option<u32> = None;
        for r in tl {
            let a = r.ayah;
            let marked = r.start.is_some();
            let is_now = a == cur && self.duration_ms > 0;
            let is_sel = a == sel;

            if a > 0 {
                let pg = self.surah_text.as_ref().and_then(|st| st.pages.get(a - 1).copied().flatten());
                if let Some(pg_num) = pg {
                    if Some(pg_num) != prev_pg {
                        prev_pg = Some(pg_num);
                        list = list.push(
                            container(ar(format!("صفحة {}", self.num(pg_num as usize)), ui).size(ty::CAPTION).color(p.muted))
                                .width(Length::Fill)
                                .center_x(Length::Fill),
                        );
                    }
                }
            }

            let body: Element<'_, Message> = if a == 0 {
                ar("الاستعاذة والبسملة", ui).size(ty::BODY).width(Length::Fill).align_x(alignment::Horizontal::Right).into()
            } else {
                let t = self.surah_text.as_ref().and_then(|st| st.texts.get(a - 1)).cloned().unwrap_or_else(|| format!("الآية {a}"));
                text(rtl_para(&t))
                    .size(self.mushaf_font_size)
                    .line_height(1.75)
                    .font(qfont)
                    .shaping(text::Shaping::Advanced)
                    .width(Length::Fill)
                    .color(if marked { p.ink } else { p.muted })
                    .into()
            };
            let badge = container(text(self.num(a)).size(ty::CAPTION).shaping(text::Shaping::Advanced).color(if marked { p.accent_ink } else { p.muted }))
                .width(26)
                .height(26)
                .center_x(26)
                .center_y(26)
                .style(move |_| container::Style {
                    background: Some((if marked { p.accent } else { p.track }).into()),
                    border: border::Border {
                        radius: 13.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            let time = text(r.start.map(fmt_ms).unwrap_or_else(|| "—".into()))
                .size(ty::CAPTION)
                .color(if marked { p.accent } else { p.muted });

            let main_row = button(
                row![time, body, badge].spacing(10).align_y(alignment::Vertical::Center),
            )
            .width(Length::Fill)
            .padding([6, 8])
            .on_press(Message::SelectAyah(a))
            .style(move |_, _| button::Style {
                background: None,
                text_color: p.ink,
                ..Default::default()
            });

            let mut item = column![main_row];
            if is_sel {
                let mut acts = row![].spacing(5).align_y(alignment::Vertical::Center);
                if a > 0 {
                    let val = self.table_inputs.get(&a).cloned().unwrap_or_default();
                    let dur = match (r.start, r.end) {
                        (Some(s), Some(e)) => format!("{:.1}ث", e.saturating_sub(s) as f64 / 1000.0),
                        _ => String::new(),
                    };
                    acts = acts
                        .push(self.with_tip(self.btn_danger(icon(Icon::X, 13.0, p.danger), [6, 7], Message::ClearAyah(a)).into(), "مسح التوقيت"))
                        .push(ar(dur, ui).size(ty::CAPTION).color(p.muted))
                        .push(Space::with_width(Length::Fill))
                        .push(self.icon_btn(Icon::Plus, "+١٠٠ ms", Message::NudgeAyah(a, 100), 12.0))
                        .push(
                            self.styled_input("ms", &val, move |s| Message::TableInputChanged(a, s), ty::BODY, [6, 6], Length::Fixed(74.0))
                                .align_x(alignment::Horizontal::Center)
                                .on_submit(Message::TableInputSubmit(a)),
                        )
                        .push(self.icon_btn(Icon::Minus, "−١٠٠ ms", Message::NudgeAyah(a, -100), 12.0));
                } else {
                    acts = acts.push(Space::with_width(Length::Fill));
                }
                acts = acts.push(self.with_tip(
                    self.btn_ghost(icon(Icon::Play, 13.0, p.accent), [7, 9], Message::PlayAyah(a)).into(),
                    "تشغيل الآية",
                ));
                if a > 0 {
                    acts = acts.push(self.btn_primary(
                        self.ic_label(Icon::Clock, "الآن", p.accent_ink, ty::CAPTION),
                        [6, 10],
                        Some(Message::SetAyahToNow(a)),
                    ));
                }
                item = item.push(
                    container(acts).padding(Padding { top: 6.0, right: 8.0, bottom: 4.0, left: 8.0 }),
                );
            }

            list = list.push(
                container(item)
                    .padding(2)
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(
                            (if is_sel {
                                p.soft
                            } else if is_now {
                                p.verse
                            } else {
                                p.panel2
                            })
                            .into(),
                        ),
                        border: border::Border {
                            color: if is_sel { p.accent } else if is_now { Color { a: 0.4, ..p.accent } } else { Color::TRANSPARENT },
                            width: 1.5,
                            radius: 12.0.into(),
                        },
                        ..Default::default()
                    }),
            );
        }
        if self.surah_text.is_none() {
            list = list.push(ar("جارٍ تحميل نص المصحف…", ui).size(ty::BODY).color(p.muted));
        }

        self.card(
            column![
                header,
                self.styled_scrollable(container(list).padding(Padding { top: 0.0, right: 10.0, bottom: 6.0, left: 0.0 }))
                    .id(table_scroll_id())
                    .height(Length::Fill),
            ]
            .spacing(10)
            .height(Length::Fill),
            12,
        )
        .width(Length::FillPortion(38))
        .height(Length::Fill)
        .into()
    }

    // ───────────────────────── drawers ─────────────────────────

    fn drawer_layer(&self, d: Drawer) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let (title, body, side) = match d {
            Drawer::Source => ("مصدر التلاوة", self.source_body(), false),
            Drawer::Tools => ("تصدير وأدوات", self.tools_body(), true),
            Drawer::Settings => ("الإعدادات", self.settings_body(), true),
        };
        let head = row![
            self.icon_btn(Icon::X, "إغلاق  Esc", Message::CloseDrawer, 15.0),
            Space::with_width(Length::Fill),
            ar(title, ui).size(17.0),
        ]
        .align_y(alignment::Vertical::Center);

        let shadow = Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
            offset: Vector::new(0.0, 12.0),
            blur_radius: 32.0,
        };
        if side {
            let panel = container(
                column![
                    head,
                    self.styled_scrollable(container(body).padding(Padding { top: 0.0, right: 8.0, bottom: 8.0, left: 0.0 }))
                        .height(Length::Fill),
                ]
                .spacing(12),
            )
            .width(Length::Fixed(470.0))
            .height(Length::Fill)
            .padding([16, 18])
            .style(move |_| container::Style {
                background: Some(p.panel.into()),
                shadow,
                ..Default::default()
            });
            container(mouse_area(panel).on_press(Message::Noop))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Horizontal::Left)
                .into()
        } else {
            let panel = container(column![head, body].spacing(14))
                .width(Length::Fixed(540.0))
                .padding([16, 20])
                .style(move |_| container::Style {
                    background: Some(p.panel.into()),
                    border: border::Border {
                        radius: 16.0.into(),
                        ..Default::default()
                    },
                    shadow,
                    ..Default::default()
                });
            container(mouse_area(panel).on_press(Message::Noop))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Horizontal::Right)
                .padding(Padding { top: 58.0, right: 150.0, bottom: 16.0, left: 16.0 })
                .into()
        }
    }

    fn source_body(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let tabs = self.seg_group(vec![
            self.seg_btn("رابط", self.source_tab == SourceTab::Url, Message::SourceTabChanged(SourceTab::Url)),
            self.seg_btn("مجلد السور", self.source_tab == SourceTab::Folder, Message::SourceTabChanged(SourceTab::Folder)),
            self.seg_btn("ملف", self.source_tab == SourceTab::File, Message::SourceTabChanged(SourceTab::File)),
            self.seg_btn("mp3quran", self.source_tab == SourceTab::Mp3Quran, Message::SourceTabChanged(SourceTab::Mp3Quran)),
        ]);

        let mut col = column![container(tabs).width(Length::Fill).align_x(alignment::Horizontal::Right)].spacing(14);
        match self.source_tab {
            SourceTab::Mp3Quran => {
                let rec = self.reciter_opts.get(self.selected_reciter).map(|o| o.raw_label.as_str()).unwrap_or("اختر القارئ…");
                let mos = self.moshaf_opts.get(self.selected_moshaf).map(|o| o.raw_label.as_str()).unwrap_or("اختر المصحف…");
                col = col
                    .push(self.field(
                        "الرواية",
                        self.seg_group(vec![
                            self.seg_btn(Riwayah::Qalun.label(), self.riwayah == Riwayah::Qalun, Message::RiwayahChanged(Riwayah::Qalun)),
                            self.seg_btn(Riwayah::Warsh.label(), self.riwayah == Riwayah::Warsh, Message::RiwayahChanged(Riwayah::Warsh)),
                            self.seg_btn(Riwayah::Hafs.label(), self.riwayah == Riwayah::Hafs, Message::RiwayahChanged(Riwayah::Hafs)),
                            self.seg_btn(Riwayah::All.label(), self.riwayah == Riwayah::All, Message::RiwayahChanged(Riwayah::All)),
                        ]),
                    ))
                    .push(self.field("القارئ", self.selector_trigger(rec, SelectorKind::Reciter, Length::Fill)))
                    .push(self.field("المصحف", self.selector_trigger(mos, SelectorKind::Moshaf, Length::Fill)));
            }
            SourceTab::File => {
                let name = self
                    .file_path
                    .as_ref()
                    .and_then(|f| f.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "لم يُختر ملف".into());
                col = col.push(self.field(
                    "الملف",
                    row![
                        self.btn_ghost(self.ic_label(Icon::Upload, "اختر ملفًا…", p.ink, ty::BODY), [9, 12], Message::PickAudioFile),
                        Space::with_width(Length::Fill),
                        ar(name, ui).size(ty::BODY),
                    ]
                    .spacing(8)
                    .align_y(alignment::Vertical::Center),
                ));
            }
            SourceTab::Folder => {
                let label = match &self.settings.audio_folder {
                    Some(d) => format!("{} · {} سورة", d.display(), self.num(self.folder_files.len())),
                    None => "لم يُختر مجلد".into(),
                };
                col = col
                    .push(self.field(
                        "مجلد السور",
                        row![
                            self.btn_ghost(self.ic_label(Icon::Folder, "اختر مجلدًا…", p.ink, ty::BODY), [9, 12], Message::PickAudioFolder),
                            Space::with_width(Length::Fill),
                            ar(label, ui).size(ty::BODY),
                        ]
                        .spacing(8)
                        .align_y(alignment::Vertical::Center),
                    ))
                    .push(
                        ar("تُربط الملفات بالسور من أول رقم في اسم الملف (001.mp3 ← الفاتحة). اختيار سورة من الأعلى يحمّل ملفها مباشرة.", ui)
                            .size(ty::CAPTION)
                            .color(p.muted)
                            .width(Length::Fill)
                            .align_x(alignment::Horizontal::Right),
                    );
            }
            SourceTab::Url => {
                col = col.push(self.field(
                    "رابط الملف الصوتي",
                    self.styled_input("https://…/001.mp3", &self.url_input, Message::UrlChanged, ty::BODY, [10, 12], Length::Fill),
                ));
            }
        }
        col = col.push(self.field("العدّ", self.selector_trigger(self.counting.label(), SelectorKind::Counting, Length::Fill)));
        col = col.push(
            self.btn_primary(
                ar("تحميل التلاوة", ui).size(15.0).width(Length::Fill).align_x(alignment::Horizontal::Center),
                [12, 16],
                Some(Message::LoadAudio),
            )
            .width(Length::Fill),
        );
        col.into()
    }

    fn section_title<'a>(&self, title: &'a str, aside: String) -> Element<'a, Message> {
        let p = self.pal();
        let aside_txt = if aside.is_ascii() { text(aside).font(self.ui_font) } else { ar(aside, self.ui_font) };
        row![
            aside_txt.size(ty::CAPTION).color(p.muted),
            Space::with_width(Length::Fill),
            ar(title, self.ui_font).size(15.0),
        ]
        .align_y(alignment::Vertical::Center)
        .into()
    }

    fn labeled_row<'a>(&self, label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
        let p = self.pal();
        row![
            Space::with_width(Length::Fill),
            control,
            ar(label, self.ui_font).size(ty::BODY).color(p.muted).width(Length::Fixed(64.0)).align_x(alignment::Horizontal::Right),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center)
        .into()
    }

    fn tools_body(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let busy = self.ai_running || self.export_running || self.audio_path.is_none();
        let n = self.count();
        let pages = self.surah_text.as_ref().map(|t| t.pages.as_slice());
        let tl = build_timeline(&self.starts, n, self.duration_ms, pages);
        let ready = tl
            .iter()
            .filter(|r| match (r.start, r.end) {
                (Some(s), Some(e)) => e > s && (r.ayah > 0 || self.export_opts.include_intro),
                _ => false,
            })
            .count();

        // JSON
        let mut json = column![
            self.section_title("توقيت JSON", format_bytes(self.json_content.text().len() as u64)),
            row![
                self.btn_ghost(self.ic_label(Icon::Copy, "نسخ", p.ink, ty::BODY), [8, 12], Message::CopyJson),
                self.btn_primary(self.ic_label(Icon::Download, "تنزيل", p.accent_ink, ty::BODY), [8, 14], Some(Message::DownloadJson)),
            ]
            .spacing(6),
            row![
                self.btn_ghost(ar("استيراد نص", ui).size(ty::BODY), [8, 12], Message::ImportFromText),
                self.btn_ghost(ar("استيراد ملف", ui).size(ty::BODY), [8, 12], Message::ImportFromFile),
                self.btn_soft(ar("جلب توقيت mp3quran", ui).size(ty::BODY), [8, 12], Some(Message::FetchPublishedTiming)),
            ]
            .spacing(6),
            row![
                self.btn_soft(
                    ar(if self.show_json_editor { "إخفاء المحرر" } else { "إظهار المحرر" }, ui).size(ty::CAPTION),
                    [5, 10],
                    Some(Message::ToggleJsonEditor)
                ),
                Space::with_width(Length::Fill),
                self.rtl_checkbox("حقول mp3quran", self.full_fields, Message::FullFieldsToggled),
                self.rtl_checkbox("ضغط JSON", self.json_minified, Message::JsonMinifiedToggled),
            ]
            .spacing(14)
            .align_y(alignment::Vertical::Center),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);
        if self.show_json_editor {
            json = json.push(
                text_editor(&self.json_content)
                    .on_action(Message::JsonAction)
                    .height(Length::Fixed(200.0))
                    .size(ty::CAPTION),
            );
        }

        // Audio export
        let f = self.export_opts.format;
        let b = self.export_opts.bitrate_kbps;
        let export = column![
            self.section_title("تصدير الصوت (ffmpeg)", format!("{} آية جاهزة", self.num(ready))),
            self.labeled_row(
                "الصيغة",
                self.seg_group(vec![
                    self.seg_btn(AudioFormat::Copy.label(), f == AudioFormat::Copy, Message::ExportFormatPicked(AudioFormat::Copy)),
                    self.seg_btn(AudioFormat::M4a.label(), f == AudioFormat::M4a, Message::ExportFormatPicked(AudioFormat::M4a)),
                    self.seg_btn(AudioFormat::Opus.label(), f == AudioFormat::Opus, Message::ExportFormatPicked(AudioFormat::Opus)),
                    self.seg_btn(AudioFormat::Mp3.label(), f == AudioFormat::Mp3, Message::ExportFormatPicked(AudioFormat::Mp3)),
                ]),
            ),
            self.labeled_row(
                "الجودة",
                self.seg_group(vec![
                    self.seg_btn("128k", b == 128, Message::ExportBitratePicked(128)),
                    self.seg_btn("64k", b == 64, Message::ExportBitratePicked(64)),
                    self.seg_btn("48k", b == 48, Message::ExportBitratePicked(48)),
                    self.seg_btn("32k", b == 32, Message::ExportBitratePicked(32)),
                    self.seg_btn("24k", b == 24, Message::ExportBitratePicked(24)),
                ]),
            ),
            self.labeled_row(
                "التسمية",
                self.seg_group(vec![
                    self.seg_btn("001.ext", self.export_opts.naming == NamingMode::AyahOnly, Message::ExportNamingPicked(NamingMode::AyahOnly)),
                    self.seg_btn("020001.ext", self.export_opts.naming == NamingMode::SurahAyah, Message::ExportNamingPicked(NamingMode::SurahAyah)),
                ]),
            ),
            row![
                Space::with_width(Length::Fill),
                self.rtl_checkbox("تضمين الافتتاح (000)", self.export_opts.include_intro, Message::ExportIntroToggled),
                self.rtl_checkbox("أحادي (Mono)", self.export_opts.mono, Message::ExportMonoToggled),
            ]
            .spacing(14),
            row![
                self.btn_soft(
                    ar("ضغط السورة كاملة", ui).size(ty::BODY).width(Length::Fill).align_x(alignment::Horizontal::Center),
                    [10, 12],
                    (!busy).then_some(Message::StartCompressAudio)
                )
                .width(Length::Fill),
                self.btn_primary(
                    ar(format!("تقسيم إلى {} مقطع", self.num(ready)), ui).size(ty::BODY).width(Length::Fill).align_x(alignment::Horizontal::Center),
                    [10, 12],
                    (!busy && ready > 0).then_some(Message::StartSplitAudio)
                )
                .width(Length::Fill),
            ]
            .spacing(6),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);

        // Clean-up
        let atten = self.settings.denoise_atten_db as u32;
        let clean = column![
            self.section_title("تنظيف الصوت من الضجيج", "DeepFilterNet3".into()),
            self.labeled_row(
                "القوة",
                self.seg_group(vec![
                    self.seg_btn("أقصى", atten == 100, Message::DenoiseStrengthPicked(100)),
                    self.seg_btn("قوي", atten == 40, Message::DenoiseStrengthPicked(40)),
                    self.seg_btn("متوسط", atten == 24, Message::DenoiseStrengthPicked(24)),
                    self.seg_btn("خفيف", atten == 12, Message::DenoiseStrengthPicked(12)),
                ]),
            ),
            row![
                Space::with_width(Length::Fill),
                self.rtl_checkbox("تحميل الملف المنظّف بعد الانتهاء", self.settings.load_cleaned, Message::LoadCleanedToggled),
            ],
            row![
                self.btn_ghost(ar("الأصل", ui).size(ty::BODY), [9, 12], Message::PreviewOriginal),
                if self.preview_playing() {
                    self.btn_danger(self.ic_label(Icon::X, "إيقاف المعاينة", p.danger, ty::BODY), [9, 12], Message::StopPreview)
                } else {
                    self.btn_ghost(
                        self.ic_label(Icon::Play, if self.preview_busy { "جارٍ التحضير…" } else { "معاينة ١٠ ثوانٍ" }, p.ink, ty::BODY),
                        [9, 12],
                        if busy || self.preview_busy { Message::Noop } else { Message::PreviewDenoise },
                    )
                },
                Space::with_width(Length::Fill),
                ar("من موضع التشغيل:", ui).size(ty::CAPTION).color(p.muted),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center),
            self.btn_soft(
                self.ic_label(Icon::Sparkles, "تنظيف الملف كاملًا", p.accent, ty::BODY),
                [10, 12],
                (!busy).then_some(Message::StartDenoise)
            )
            .width(Length::Fill),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);

        let footer = row![
            self.btn_ghost(ar("تغيير", ui).size(ty::CAPTION), [4, 10], Message::OpenDrawer(Drawer::Settings)),
            Space::with_width(Length::Fill),
            text(self.settings.output_dir.display().to_string()).size(ty::CAPTION).color(p.accent),
            ar("تُحفظ الملفات في:", ui).size(ty::CAPTION).color(p.muted),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center);

        column![
            self.sub_card(json),
            self.sub_card(export),
            self.sub_card(clean),
            footer,
        ]
        .spacing(12)
        .into()
    }

    fn settings_body(&self) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let s = &self.settings;
        let is_default = s.output_dir == default_output_dir();

        let output = column![
            self.section_title("مكان حفظ الملفات", String::new()),
            text(s.output_dir.display().to_string()).size(ty::BODY).color(p.accent),
            row![
                self.btn_ghost(ar("فتح المجلد", ui).size(ty::BODY), [7, 12], Message::OpenOutputDir),
                if is_default {
                    self.btn_primary(ar("افتراضي", ui).size(ty::BODY), [7, 12], None)
                } else {
                    self.btn_ghost(ar("افتراضي", ui).size(ty::BODY), [7, 12], Message::ResetOutputDir)
                },
                Space::with_width(Length::Fill),
                self.btn_soft(self.ic_label(Icon::Folder, "تغيير المجلد…", p.accent, ty::BODY), [7, 12], Some(Message::PickOutputDir)),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center),
            self.rtl_checkbox("اسأل عن مكان الحفظ في كل مرة", s.ask_each_time, Message::AskEachTimeToggled),
            ar(
                "يُحفظ JSON والصوت المضغوط والمنظّف في هذا المجلد، وتُقسَّم الآيات في مجلد فرعي باسم رقم السورة. الملفات بالاسم نفسه تُستبدل.",
                ui,
            )
            .size(ty::CAPTION)
            .color(p.muted)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Right),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);

        let folder = column![
            self.section_title("مجلد السور", format!("{} سورة", self.num(self.folder_files.len()))),
            text(s.audio_folder.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "—".into()))
                .size(ty::BODY)
                .color(p.accent),
            self.btn_soft(self.ic_label(Icon::Folder, "اختر مجلد السور…", p.accent, ty::BODY), [7, 12], Some(Message::PickAudioFolder)),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);

        let timing = column![
            self.section_title("التعليم", String::new()),
            self.labeled_row(
                "تأخّر (ms)",
                self.styled_input("250", &self.reaction_str, Message::ReactionChanged, ty::BODY, [7, 8], Length::Fixed(80.0))
                    .align_x(alignment::Horizontal::Center)
                    .into(),
            ),
            ar("يُطرح من وقت الضغط لتعويض زمن ردّ الفعل.", ui).size(ty::CAPTION).color(p.muted),
            self.rtl_checkbox("التصاق العلامة بأقرب سكتة", self.snap, Message::SnapToggled),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);

        let mut ai_opts = column![].spacing(6);
        for m in AiModel::ALL {
            let active = s.ai_model == m;
            let desc = match m {
                AiModel::Fast => "أصغر وأسرع، دقة أقل قليلًا",
                AiModel::Balanced => "الافتراضي: توازن بين السرعة والدقة",
                AiModel::Accurate => "النموذج الكامل: الأدق والأبطأ",
            };
            let status = if m.is_downloaded() { "محمَّل" } else { "يُنزَّل عند أول استخدام" };
            ai_opts = ai_opts.push(
                button(
                    row![
                        column![
                            text(m.size_label()).size(ty::CAPTION).color(p.muted),
                            ar(status, ui).size(ty::CAPTION).color(if m.is_downloaded() { p.accent } else { p.muted }),
                        ]
                        .spacing(2),
                        Space::with_width(Length::Fill),
                        column![
                            ar(m.label(), ui).size(ty::BODY),
                            ar(desc, ui).size(ty::CAPTION).color(p.muted),
                        ]
                        .spacing(2)
                        .align_x(alignment::Horizontal::Right),
                        container(Space::new(10, 10)).style(move |_| container::Style {
                            background: Some((if active { p.accent } else { Color::TRANSPARENT }).into()),
                            border: border::Border {
                                color: if active { p.accent } else { p.muted },
                                width: 1.5,
                                radius: 5.0.into(),
                            },
                            ..Default::default()
                        }),
                    ]
                    .spacing(10)
                    .align_y(alignment::Vertical::Center),
                )
                .width(Length::Fill)
                .padding([8, 10])
                .on_press(Message::AiModelPicked(m))
                .style(move |_, status| button::Style {
                    background: Some(
                        (if active {
                            p.soft
                        } else if status == button::Status::Hovered {
                            p.track
                        } else {
                            Color::TRANSPARENT
                        })
                        .into(),
                    ),
                    text_color: p.ink,
                    border: border::Border {
                        color: if active { p.accent } else { p.line },
                        width: 1.0,
                        radius: R.into(),
                    },
                    ..Default::default()
                }),
            );
        }
        let ai = column![
            self.section_title("نموذج التوقيت بالذكاء الاصطناعي", "wav2vec2 عربي".into()),
            ai_opts,
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);

        let display = column![
            self.section_title("العرض", String::new()),
            self.labeled_row(
                "الأرقام",
                self.seg_group(vec![
                    self.seg_btn("123", s.western_digits, Message::WesternDigitsToggled(true)),
                    self.seg_btn("١٢٣", !s.western_digits, Message::WesternDigitsToggled(false)),
                ]),
            ),
            self.labeled_row(
                "المظهر",
                self.seg_group(vec![
                    self.seg_btn("داكن", self.dark, if self.dark { Message::Noop } else { Message::ToggleTheme }),
                    self.seg_btn("فاتح", !self.dark, if self.dark { Message::ToggleTheme } else { Message::Noop }),
                ]),
            ),
        ]
        .spacing(10)
        .align_x(alignment::Horizontal::Right);

        column![
            self.sub_card(output),
            self.sub_card(folder),
            self.sub_card(ai),
            self.sub_card(timing),
            self.sub_card(display),
        ]
            .spacing(12)
            .into()
    }

    // ───────────────────────── searchable selector ─────────────────────────

    fn selector_layer(&self, kind: SelectorKind) -> Element<'_, Message> {
        let p = self.pal();
        let ui = self.ui_font;
        let (title, total, current_idx) = match kind {
            SelectorKind::Reciter => ("اختر القارئ", self.reciter_opts.len(), self.selected_reciter),
            SelectorKind::Moshaf => ("اختر المصحف / الرواية", self.moshaf_opts.len(), self.selected_moshaf),
            SelectorKind::Surah => ("اختر السورة", self.surah_opts.len(), self.surah),
            SelectorKind::Counting => ("اختر العدّ", self.counting_opts.len(), if self.counting == Counting::Hafs { 0 } else { 1 }),
        };
        let filtered = self.filtered_selector_options(kind);
        let shown = filtered.len();

        let mut list_col = column![].spacing(3);
        if filtered.is_empty() {
            list_col = list_col.push(
                container(ar("لا توجد نتائج مطابقة", ui).size(ty::BODY).color(p.muted))
                    .padding(20)
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            );
        }
        for (i, opt) in filtered.into_iter().enumerate() {
            let is_current = opt.idx == current_idx;
            let is_hl = i == self.selector_highlight;
            list_col = list_col.push(
                button(
                    row![
                        text(if is_current { "✓" } else { "" })
                            .size(ty::BODY)
                            .shaping(text::Shaping::Advanced)
                            .color(if is_hl { p.accent_ink } else { p.accent }),
                        Space::with_width(Length::Fill),
                        ar(&opt.raw_label, ui).size(ty::BODY),
                    ]
                    .align_y(alignment::Vertical::Center),
                )
                .width(Length::Fill)
                .padding([9, 12])
                .on_press(Message::SelectorPick(kind, opt.idx))
                .style(move |_, status| button::Style {
                    background: Some(
                        (if is_hl {
                            p.accent
                        } else if status == button::Status::Hovered || is_current {
                            p.soft
                        } else {
                            Color::TRANSPARENT
                        })
                        .into(),
                    ),
                    text_color: if is_hl { p.accent_ink } else { p.ink },
                    border: border::Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }

        let search = self
            .styled_input(&rtl("ابحث بالاسم أو الرقم…"), &self.selector_query, Message::SelectorQueryChanged, ty::BODY, [10, 12], Length::Fill)
            .align_x(alignment::Horizontal::Right)
            .id(search_input_id())
            .on_submit(Message::SelectorSubmit);
        let mut clear = button(icon(Icon::X, 14.0, p.muted)).padding([10, 10]).style(move |_, _| button::Style {
            background: Some(p.panel2.into()),
            border: border::Border {
                color: p.line,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });
        if !self.selector_query.is_empty() {
            clear = clear.on_press(Message::ClearSelectorQuery);
        }

        let card = container(
            column![
                row![
                    self.icon_btn(Icon::X, "إغلاق  Esc", Message::CloseSelector, 14.0),
                    self.pill(ar(format!("{shown} من {total}"), ui).size(ty::CAPTION).color(p.muted), p.track),
                    Space::with_width(Length::Fill),
                    ar(title, ui).size(16.0),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center),
                row![clear, search].spacing(6).align_y(alignment::Vertical::Center),
                self.styled_scrollable(container(list_col).padding(Padding { top: 0.0, right: 8.0, bottom: 0.0, left: 0.0 }))
                    .height(Length::Fixed(320.0)),
            ]
            .spacing(10),
        )
        .width(Length::Fixed(520.0))
        .padding(16)
        .style(move |_| container::Style {
            background: Some(p.panel.into()),
            border: border::Border {
                radius: 16.0.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
                offset: Vector::new(0.0, 12.0),
                blur_radius: 32.0,
            },
            ..Default::default()
        });

        container(mouse_area(card).on_press(Message::Noop))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .padding(Padding { top: 70.0, right: 0.0, bottom: 16.0, left: 0.0 })
            .into()
    }
}

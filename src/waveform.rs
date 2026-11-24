use iced::event::Status;
use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

#[derive(Debug, Clone)]
pub enum WaveEvent {
    Seek(u64),
    MoveMark(usize, u64),
    WheelZoom(f32),
    WheelScroll(f32),
}

#[derive(Debug, Clone)]
pub struct WaveColors {
    pub bg: Color,
    pub line: Color,
    pub muted: Color,
    pub wave: Color,
    pub wave_played: Color,
    pub verse: Color,
    pub mark: Color,
    pub now: Color,
}

#[derive(Debug, Default)]
pub struct WaveState {
    dragging_ayah: Option<usize>,
    hovering_mark: bool,
}

pub struct WaveformProgram<'a> {
    pub peaks: &'a [f32],
    pub duration_ms: u64,
    pub starts: &'a [Option<u64>],
    pub now_ms: u64,
    pub current_ayah: usize,
    pub zoom: f32,
    pub scroll_frac: f32,
    pub colors: WaveColors,
}

impl<'a> WaveformProgram<'a> {
    fn total_w(&self, view_w: f32) -> f32 {
        view_w.max(10.0) * self.zoom.max(1.0)
    }

    fn scroll_px(&self, view_w: f32) -> f32 {
        let max_scroll = (self.total_w(view_w) - view_w).max(0.0);
        self.scroll_frac.clamp(0.0, 1.0) * max_scroll
    }

    fn x_of(&self, ms: u64, view_w: f32) -> f32 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        (ms as f32 / self.duration_ms as f32) * self.total_w(view_w) - self.scroll_px(view_w)
    }

    fn ms_of(&self, sx: f32, view_w: f32) -> u64 {
        if self.duration_ms == 0 {
            return 0;
        }
        let wx = sx + self.scroll_px(view_w);
        let ratio = (wx / self.total_w(view_w)).clamp(0.0, 1.0);
        (ratio * self.duration_ms as f32).round() as u64
    }

    fn hit_mark(&self, sx: f32, view_w: f32) -> Option<usize> {
        let mut best = None;
        let mut best_d = 8.0f32;
        for (a, st) in self.starts.iter().enumerate().skip(1) {
            let Some(ms) = st else { continue };
            let d = (self.x_of(*ms, view_w) - sx).abs();
            if d < best_d {
                best_d = d;
                best = Some(a);
            }
        }
        best
    }
}

impl<'a> canvas::Program<WaveEvent> for WaveformProgram<'a> {
    type State = WaveState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (Status, Option<WaveEvent>) {
        let view_w = bounds.width;
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if self.peaks.is_empty() || self.duration_ms == 0 {
                        return (Status::Ignored, None);
                    }
                    if let Some(ayah) = self.hit_mark(pos.x, view_w) {
                        state.dragging_ayah = Some(ayah);
                        return (Status::Captured, None);
                    }
                    let ms = self.ms_of(pos.x, view_w);
                    return (Status::Captured, Some(WaveEvent::Seek(ms)));
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(ayah) = state.dragging_ayah {
                    if let Some(pos) = cursor.position_in(Rectangle {
                        x: bounds.x,
                        y: bounds.y,
                        width: bounds.width,
                        height: bounds.height,
                    }) {
                        let ms = self.ms_of(pos.x, view_w);
                        return (Status::Captured, Some(WaveEvent::MoveMark(ayah, ms)));
                    }
                } else if let Some(pos) = cursor.position_in(bounds) {
                    state.hovering_mark = self.hit_mark(pos.x, view_w).is_some();
                } else {
                    state.hovering_mark = false;
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.dragging_ayah.is_some() {
                    state.dragging_ayah = None;
                    return (Status::Captured, None);
                }
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.is_over(bounds) && !self.peaks.is_empty() {
                    let (dx, dy) = match delta {
                        mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                        mouse::ScrollDelta::Pixels { x, y } => (x, y),
                    };
                    if dx.abs() > dy.abs() {
                        return (Status::Captured, Some(WaveEvent::WheelScroll(-dx)));
                    } else if dy.abs() > 0.0 {
                        return (Status::Captured, Some(WaveEvent::WheelZoom(dy)));
                    }
                }
            }
            _ => {}
        }
        (Status::Ignored, None)
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging_ayah.is_some() || state.hovering_mark {
            mouse::Interaction::ResizingHorizontally
        } else if cursor.is_over(bounds) && !self.peaks.is_empty() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;
        let ruler_h = 18.0f32;
        let wave_h = (h - ruler_h).max(20.0);

        frame.fill_rectangle(Point::ORIGIN, bounds.size(), self.colors.bg);

        if self.peaks.is_empty() || self.duration_ms == 0 {
            frame.fill_text(Text {
                content: "حمّل تلاوة لرسم موجتها".into(),
                position: Point::new(w / 2.0, h / 2.0 - 7.0),
                color: self.colors.muted,
                size: iced::Pixels(13.0),
                horizontal_alignment: iced::alignment::Horizontal::Center,
                ..Text::default()
            });
            return vec![frame.into_geometry()];
        }

        let total_w = self.total_w(w);
        let off = self.scroll_px(w);

        // Active verse highlight
        let cur_start = if self.current_ayah == 0 {
            Some(0)
        } else {
            self.starts.get(self.current_ayah).copied().flatten()
        };
        if let Some(a_ms) = cur_start {
            let b_ms = self
                .starts
                .iter()
                .skip(self.current_ayah + 1)
                .find_map(|v| *v)
                .unwrap_or(self.duration_ms);
            let x0 = self.x_of(a_ms, w).max(0.0);
            let x1 = self.x_of(b_ms, w).min(w);
            if x1 > x0 {
                frame.fill_rectangle(Point::new(x0, 0.0), Size::new(x1 - x0, wave_h), self.colors.verse);
            }
        }

        // Waveform bars
        let n = self.peaks.len();
        let played_wx = (self.now_ms as f32 / self.duration_ms as f32) * total_w;
        let mid = wave_h / 2.0;
        let max_amp = (wave_h - 12.0) * 0.45;
        let cols = w.ceil() as usize;
        for sx in 0..cols {
            let wx = sx as f32 + off;
            if wx >= total_w {
                break;
            }
            let idx = ((wx / total_w) * n as f32).floor() as usize;
            let idx = idx.min(n - 1);
            let v = (self.peaks[idx] * max_amp).max(1.0);
            let col = if wx < played_wx {
                self.colors.wave_played
            } else {
                self.colors.wave
            };
            frame.fill_rectangle(Point::new(sx as f32, mid - v), Size::new(1.0, v * 2.0), col);
        }

        // Time ruler
        frame.fill_rectangle(Point::new(0.0, wave_h), Size::new(w, 1.0), self.colors.line);
        let pps = total_w / (self.duration_ms as f32 / 1000.0).max(1.0);
        let steps = [1u64, 2, 5, 10, 15, 30, 60, 120, 300, 600];
        let step = steps.iter().copied().find(|&s| s as f32 * pps >= 70.0).unwrap_or(600);
        let t0 = (self.ms_of(0.0, w) / 1000 / step) * step;
        let t1 = (self.ms_of(w, w) / 1000) + step;
        let mut sec = t0;
        while sec <= t1 {
            let sx = self.x_of(sec * 1000, w);
            if sx >= -20.0 && sx <= w {
                frame.fill_rectangle(Point::new(sx, wave_h), Size::new(1.0, 5.0), self.colors.line);
                let m = sec / 60;
                let r = sec % 60;
                frame.fill_text(Text {
                    content: format!("{m}:{r:02}"),
                    position: Point::new(sx + 3.0, wave_h + 2.0),
                    color: self.colors.muted,
                    size: iced::Pixels(10.0),
                    ..Text::default()
                });
            }
            sec += step;
        }

        // Verse marks
        for (a, st) in self.starts.iter().enumerate().skip(1) {
            let Some(ms) = st else { continue };
            let sx = self.x_of(*ms, w);
            if sx < -40.0 || sx > w + 2.0 {
                continue;
            }
            frame.fill_rectangle(Point::new(sx - 1.0, 0.0), Size::new(2.0, wave_h), self.colors.mark);
            let lbl = a.to_string();
            let bw = (lbl.len() as f32 * 7.5 + 8.0).max(22.0);
            frame.fill_rectangle(Point::new(sx - 1.0, 2.0), Size::new(bw, 15.0), self.colors.mark);
            frame.fill_text(Text {
                content: lbl,
                position: Point::new(sx + 3.0, 3.0),
                color: Color::WHITE,
                size: iced::Pixels(11.0),
                ..Text::default()
            });
        }

        // Playhead
        let px = played_wx - off;
        if px >= -4.0 && px <= w + 4.0 {
            frame.fill_rectangle(Point::new(px - 1.0, 0.0), Size::new(2.0, wave_h), self.colors.now);
            let tri = Path::new(|b| {
                b.move_to(Point::new(px - 5.0, 0.0));
                b.line_to(Point::new(px + 5.0, 0.0));
                b.line_to(Point::new(px, 6.0));
                b.close();
            });
            frame.fill(&tri, self.colors.now);
            let _ = Stroke::default();
        }

        vec![frame.into_geometry()]
    }
}

pub fn view_waveform<'a>(prog: WaveformProgram<'a>) -> Element<'a, WaveEvent> {
    Canvas::new(prog).width(Length::Fill).height(Length::Fixed(170.0)).into()
}

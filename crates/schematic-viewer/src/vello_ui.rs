//! Theme, icons, native text, and selectable UI primitives.

use super::*;

pub(super) const STATUS_HEIGHT: f64 = 40.0;
pub(super) const SIDEBAR_WIDTH: f64 = 96.0;
pub(super) const FILMSTRIP_HEIGHT: f64 = 180.0;
pub(super) const TIMELINE_HEIGHT: f64 = 50.0;
pub(super) const PAGE_PADDING: f64 = 16.0;
pub(super) const THUMBNAIL_WIDTH: f64 = 150.0;
pub(super) const THUMBNAIL_GAP: f64 = 8.0;
pub(super) const HISTORY_LIMIT: usize = 200;
pub(super) const DIAGNOSTICS_PANEL_WIDTH: f64 = 620.0;
pub(super) const DIAGNOSTICS_HEADER_HEIGHT: f64 = 38.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ScreenRect {
    pub(super) x0: f64,
    pub(super) y0: f64,
    pub(super) x1: f64,
    pub(super) y1: f64,
}

impl ScreenRect {
    pub(super) fn width(self) -> f64 {
        self.x1 - self.x0
    }

    pub(super) fn height(self) -> f64 {
        self.y1 - self.y0
    }

    pub(super) fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    pub(super) fn center(self) -> (f64, f64) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    pub(super) fn as_kurbo(self) -> Rect {
        Rect::new(self.x0, self.y0, self.x1, self.y1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Theme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) app: Color,
    pub(crate) toolbar: Color,
    pub(crate) filmstrip: Color,
    pub(crate) page: Color,
    pub(crate) fill: Color,
    pub(crate) card: Color,
    pub(crate) card_border: Color,
    pub(crate) accent: Color,
    pub(crate) border: Color,
    pub(crate) bus: Color,
    pub(crate) junction: Color,
    pub(crate) label: Color,
    pub(crate) no_connect: Color,
    pub(crate) pin: Color,
    pub(crate) sheet_file: Color,
    pub(crate) symbol: Color,
    pub(crate) text: Color,
    pub(crate) wire: Color,
    pub(crate) selection: Color,
}

impl Theme {
    pub(super) fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette {
                // Dark mode is a purpose-built high-contrast palette rather
                // than KiCad's light-canvas colors placed on a dark page.
                // Every semantic foreground clears 4.5:1 against `page`;
                // text clears 12:1 against all UI surfaces.
                app: rgb(18, 22, 29),
                toolbar: rgb(24, 29, 38),
                filmstrip: rgb(15, 19, 26),
                page: rgb(31, 35, 42),
                fill: rgb(49, 55, 66),
                card: rgb(36, 42, 52),
                card_border: rgb(103, 116, 137),
                accent: rgb(125, 211, 252),
                border: rgb(184, 194, 209),
                bus: rgb(126, 164, 255),
                junction: rgb(113, 218, 151),
                label: rgb(238, 242, 248),
                no_connect: rgb(192, 167, 255),
                pin: rgb(255, 172, 153),
                sheet_file: rgb(255, 209, 102),
                symbol: rgb(255, 143, 143),
                text: rgb(242, 244, 248),
                wire: rgb(113, 218, 151),
                selection: rgb(255, 202, 92),
            },
            Self::Light => Palette {
                app: rgb(226, 231, 242),
                toolbar: rgb(245, 247, 252),
                filmstrip: rgb(235, 239, 247),
                page: rgb(245, 244, 239),
                fill: rgb(255, 255, 194),
                card: rgb(250, 251, 254),
                card_border: rgb(153, 164, 190),
                accent: rgb(190, 24, 72),
                border: rgb(132, 0, 0),
                bus: rgb(0, 0, 194),
                junction: rgb(0, 150, 0),
                label: rgb(15, 15, 15),
                no_connect: rgb(0, 0, 132),
                pin: rgb(169, 0, 0),
                sheet_file: rgb(114, 86, 0),
                symbol: rgb(132, 0, 0),
                text: rgb(0, 100, 100),
                wire: rgb(0, 150, 0),
                selection: rgb(215, 28, 84),
            },
        }
    }
}

pub(super) const fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::from_rgb8(red, green, blue)
}

pub(super) fn draw_ui_icon(scene: &mut Scene, icon: UiIcon, rect: ScreenRect, color: Color) {
    let (cx, cy) = rect.center();
    let radius = rect.width().min(rect.height()) * 0.27;
    let stroke = Stroke::new(1.8);
    let mut path = BezPath::new();
    match icon {
        UiIcon::Add => {
            path.move_to((cx - radius, cy));
            path.line_to((cx + radius, cy));
            path.move_to((cx, cy - radius));
            path.line_to((cx, cy + radius));
        }
        UiIcon::Commit => {
            path.move_to((cx - radius, cy));
            path.line_to((cx - radius * 0.25, cy + radius * 0.72));
            path.line_to((cx + radius, cy - radius * 0.72));
        }
        UiIcon::Delete => {
            path.move_to((cx - radius * 0.72, cy - radius * 0.55));
            path.line_to((cx - radius * 0.5, cy + radius));
            path.line_to((cx + radius * 0.5, cy + radius));
            path.line_to((cx + radius * 0.72, cy - radius * 0.55));
            path.move_to((cx - radius, cy - radius * 0.72));
            path.line_to((cx + radius, cy - radius * 0.72));
            path.move_to((cx - radius * 0.36, cy - radius));
            path.line_to((cx + radius * 0.36, cy - radius));
        }
        UiIcon::Discard => {
            path.move_to((cx - radius * 0.75, cy - radius * 0.75));
            path.line_to((cx + radius * 0.75, cy + radius * 0.75));
            path.move_to((cx + radius * 0.75, cy - radius * 0.75));
            path.line_to((cx - radius * 0.75, cy + radius * 0.75));
        }
        UiIcon::Duplicate => {
            path.move_to((cx - radius, cy - radius));
            path.line_to((cx + radius * 0.45, cy - radius));
            path.line_to((cx + radius * 0.45, cy + radius * 0.45));
            path.line_to((cx - radius, cy + radius * 0.45));
            path.close_path();
            path.move_to((cx - radius * 0.45, cy - radius * 0.45));
            path.line_to((cx + radius, cy - radius * 0.45));
            path.line_to((cx + radius, cy + radius));
            path.line_to((cx - radius * 0.45, cy + radius));
        }
        UiIcon::Edit => {
            path.move_to((cx - radius * 0.85, cy + radius * 0.85));
            path.line_to((cx - radius * 0.55, cy + radius * 0.15));
            path.line_to((cx + radius * 0.55, cy - radius * 0.95));
            path.line_to((cx + radius * 0.95, cy - radius * 0.55));
            path.line_to((cx - radius * 0.15, cy + radius * 0.55));
            path.close_path();
        }
        UiIcon::External | UiIcon::Transform => {
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                color,
                None,
                &Circle::new((cx, cy), radius * 0.75),
            );
            path.move_to((cx + radius * 0.22, cy - radius * 0.92));
            path.line_to((cx + radius * 0.85, cy - radius * 0.8));
            path.line_to((cx + radius * 0.68, cy - radius * 0.2));
            if icon == UiIcon::External {
                path.move_to((cx - radius, cy));
                path.line_to((cx + radius, cy));
                path.move_to((cx, cy - radius));
                path.line_to((cx, cy + radius));
            }
        }
        UiIcon::Fit => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                path.move_to((cx + sx * radius, cy + sy * radius * 0.45));
                path.line_to((cx + sx * radius, cy + sy * radius));
                path.line_to((cx + sx * radius * 0.45, cy + sy * radius));
            }
        }
        UiIcon::Follow => {
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                color,
                None,
                &Circle::new((cx, cy), radius * 0.62),
            );
            for (x0, y0, x1, y1) in [
                (cx - radius, cy, cx - radius * 0.45, cy),
                (cx + radius * 0.45, cy, cx + radius, cy),
                (cx, cy - radius, cx, cy - radius * 0.45),
                (cx, cy + radius * 0.45, cx, cy + radius),
            ] {
                path.move_to((x0, y0));
                path.line_to((x1, y1));
            }
        }
        UiIcon::Grid => {
            for offset in [-0.62, 0.0, 0.62] {
                path.move_to((cx - radius, cy + offset * radius));
                path.line_to((cx + radius, cy + offset * radius));
                path.move_to((cx + offset * radius, cy - radius));
                path.line_to((cx + offset * radius, cy + radius));
            }
        }
        UiIcon::Highlight => {
            path.move_to((cx - radius, cy));
            path.curve_to(
                (cx - radius * 0.5, cy - radius * 0.8),
                (cx + radius * 0.5, cy - radius * 0.8),
                (cx + radius, cy),
            );
            path.curve_to(
                (cx + radius * 0.5, cy + radius * 0.8),
                (cx - radius * 0.5, cy + radius * 0.8),
                (cx - radius, cy),
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                color,
                None,
                &Circle::new((cx, cy), radius * 0.26),
            );
        }
        UiIcon::Move => {
            path.move_to((cx - radius, cy));
            path.line_to((cx + radius, cy));
            path.move_to((cx, cy - radius));
            path.line_to((cx, cy + radius));
            for (x, y, dx, dy) in [
                (cx - radius, cy, 0.35, -0.35),
                (cx - radius, cy, 0.35, 0.35),
                (cx + radius, cy, -0.35, -0.35),
                (cx + radius, cy, -0.35, 0.35),
                (cx, cy - radius, -0.35, 0.35),
                (cx, cy - radius, 0.35, 0.35),
                (cx, cy + radius, -0.35, -0.35),
                (cx, cy + radius, 0.35, -0.35),
            ] {
                path.move_to((x, y));
                path.line_to((x + dx * radius, y + dy * radius));
            }
        }
        UiIcon::Undo | UiIcon::Redo => {
            let direction = if icon == UiIcon::Undo { -1.0 } else { 1.0 };
            path.move_to((cx - direction * radius * 0.75, cy - radius * 0.7));
            path.line_to((cx + direction * radius * 0.1, cy - radius * 0.7));
            path.curve_to(
                (cx + direction * radius, cy - radius * 0.7),
                (cx + direction * radius, cy + radius * 0.7),
                (cx + direction * radius * 0.1, cy + radius * 0.7),
            );
            path.move_to((cx - direction * radius * 0.75, cy - radius * 0.7));
            path.line_to((cx - direction * radius * 0.25, cy - radius));
            path.move_to((cx - direction * radius * 0.75, cy - radius * 0.7));
            path.line_to((cx - direction * radius * 0.25, cy - radius * 0.35));
        }
        UiIcon::Scale => {
            path.move_to((cx - radius, cy + radius));
            path.line_to((cx, cy - radius));
            path.line_to((cx + radius, cy + radius));
            path.move_to((cx - radius * 0.55, cy + radius * 0.2));
            path.line_to((cx + radius * 0.55, cy + radius * 0.2));
        }
        UiIcon::Snap => {
            path.move_to((cx - radius, cy - radius));
            path.line_to((cx - radius, cy + radius * 0.35));
            path.curve_to(
                (cx - radius, cy + radius),
                (cx + radius, cy + radius),
                (cx + radius, cy + radius * 0.35),
            );
            path.line_to((cx + radius, cy - radius));
            path.move_to((cx - radius, cy - radius * 0.35));
            path.line_to((cx - radius * 0.4, cy - radius * 0.35));
            path.move_to((cx + radius * 0.4, cy - radius * 0.35));
            path.line_to((cx + radius, cy - radius * 0.35));
        }
        UiIcon::Theme => {
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                color,
                None,
                &Circle::new((cx, cy), radius * 0.5),
            );
            for index in 0..8 {
                let angle = index as f64 * std::f64::consts::FRAC_PI_4;
                path.move_to((
                    cx + angle.cos() * radius * 0.68,
                    cy + angle.sin() * radius * 0.68,
                ));
                path.line_to((cx + angle.cos() * radius, cy + angle.sin() * radius));
            }
        }
        UiIcon::TextSelect => {
            path.move_to((cx - radius * 0.65, cy - radius));
            path.line_to((cx + radius * 0.65, cy - radius));
            path.move_to((cx, cy - radius));
            path.line_to((cx, cy + radius));
            path.move_to((cx - radius * 0.65, cy + radius));
            path.line_to((cx + radius * 0.65, cy + radius));
        }
        UiIcon::Wire => {
            path.move_to((cx - radius, cy + radius * 0.65));
            path.line_to((cx - radius * 0.25, cy + radius * 0.65));
            path.line_to((cx + radius * 0.25, cy - radius * 0.65));
            path.line_to((cx + radius, cy - radius * 0.65));
            for (x, y) in [
                (cx - radius, cy + radius * 0.65),
                (cx + radius, cy - radius * 0.65),
            ] {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Circle::new((x, y), 2.2),
                );
            }
        }
        UiIcon::ZoomIn | UiIcon::ZoomOut => {
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                color,
                None,
                &Circle::new((cx - radius * 0.2, cy - radius * 0.2), radius * 0.65),
            );
            path.move_to((cx + radius * 0.28, cy + radius * 0.28));
            path.line_to((cx + radius, cy + radius));
            path.move_to((cx - radius * 0.52, cy - radius * 0.2));
            path.line_to((cx + radius * 0.12, cy - radius * 0.2));
            if icon == UiIcon::ZoomIn {
                path.move_to((cx - radius * 0.2, cy - radius * 0.52));
                path.line_to((cx - radius * 0.2, cy + radius * 0.12));
            }
        }
    }
    if !path.is_empty() {
        scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
    }
}

pub(super) fn change_icon(kind: ChangeKind) -> UiIcon {
    match kind {
        ChangeKind::Add => UiIcon::Add,
        ChangeKind::Delete => UiIcon::Delete,
        ChangeKind::Duplicate => UiIcon::Duplicate,
        ChangeKind::Edit => UiIcon::Edit,
        ChangeKind::External => UiIcon::External,
        ChangeKind::Move => UiIcon::Move,
        ChangeKind::Redo => UiIcon::Redo,
        ChangeKind::Transform => UiIcon::Transform,
        ChangeKind::Undo => UiIcon::Undo,
        ChangeKind::Wire => UiIcon::Wire,
    }
}

pub(super) struct NativeFont {
    pub(super) data: FontData,
    pub(super) ui_scale: f32,
}

#[derive(Clone, Copy)]
pub(super) struct TextRun {
    pub(super) size: f32,
    pub(super) position: (f64, f64),
    pub(super) rotation_deg: f64,
    pub(super) align: TextAlign,
    pub(super) color: Color,
}

#[derive(Debug, Clone)]
pub(super) struct SelectableText {
    pub(super) text: String,
    pub(super) rect: ScreenRect,
    pub(super) character_x: Vec<f64>,
    pub(super) select_whole: bool,
}

impl SelectableText {
    pub(super) fn whole(text: impl Into<String>, rect: ScreenRect) -> Self {
        Self {
            text: text.into(),
            rect,
            character_x: vec![rect.x0, rect.x1],
            select_whole: true,
        }
    }

    pub(super) fn character_count(&self) -> usize {
        self.text.chars().count()
    }

    pub(super) fn character_at(&self, x: f64) -> usize {
        if self.select_whole {
            return self.character_count();
        }
        self.character_x
            .windows(2)
            .position(|window| x < (window[0] + window[1]) / 2.0)
            .unwrap_or_else(|| self.character_count())
    }

    pub(super) fn selected_text(&self, start: usize, end: usize) -> String {
        let (start, end) = if self.select_whole {
            (0, self.character_count())
        } else {
            (start.min(end), start.max(end))
        };
        self.text
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }

    pub(super) fn selection_rect(&self, start: usize, end: usize) -> ScreenRect {
        if self.select_whole {
            return self.rect;
        }
        let (start, end) = (start.min(end), start.max(end));
        ScreenRect {
            x0: self.character_x[start.min(self.character_x.len().saturating_sub(1))],
            y0: self.rect.y0,
            x1: self.character_x[end.min(self.character_x.len().saturating_sub(1))],
            y1: self.rect.y1,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct TextDrag {
    pub(super) target: SelectableText,
    pub(super) anchor: usize,
    pub(super) current: usize,
}

#[derive(Debug, Clone)]
pub(super) struct TextSelection {
    pub(super) target: SelectableText,
    pub(super) start: usize,
    pub(super) end: usize,
}

impl NativeFont {
    pub(super) fn load() -> Result<Self> {
        let mut database = Database::new();
        database.load_system_fonts();
        #[cfg(target_os = "windows")]
        let preferred = [Family::Name("Segoe UI"), Family::SansSerif];
        #[cfg(target_os = "macos")]
        let preferred = [Family::Name("SF Pro Text"), Family::SansSerif];
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let preferred = [Family::Name("Noto Sans"), Family::SansSerif];
        let id = database
            .query(&Query {
                families: &preferred,
                ..Query::default()
            })
            .or_else(|| database.faces().next().map(|face| face.id))
            .ok_or_else(|| anyhow!("no system font is available"))?;
        let (bytes, index) = database
            .with_face_data(id, |data, index| (data.to_vec(), index))
            .ok_or_else(|| anyhow!("failed to load the selected system font"))?;
        Ok(Self {
            data: FontData::new(Blob::new(Arc::new(bytes)), index),
            ui_scale: 1.0,
        })
    }

    pub(super) fn measure_and_glyphs(&self, size: f32, text: &str) -> (Vec<Glyph>, f32) {
        let Some(face) = ShapingFace::from_slice(self.data.data.as_ref(), self.data.index) else {
            return (Vec::new(), 0.0);
        };
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let shaped = shape(&face, &[], buffer);
        let scale = size / face.units_per_em() as f32;
        let mut advance = 0.0_f32;
        let glyphs = shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
            .map(|(info, position)| {
                let glyph = Glyph {
                    id: info.glyph_id,
                    x: advance + position.x_offset as f32 * scale,
                    y: -(position.y_offset as f32) * scale,
                };
                advance += position.x_advance as f32 * scale;
                glyph
            })
            .collect();
        (glyphs, advance)
    }

    pub(super) fn draw_with_target(
        &self,
        scene: &mut Scene,
        text: &str,
        run: TextRun,
    ) -> Option<SelectableText> {
        let size = run.size * self.ui_scale;
        if text.is_empty() || size <= 0.0 {
            return None;
        }
        let (glyphs, width) = self.measure_and_glyphs(size, text);
        if glyphs.is_empty() {
            return None;
        }
        let offset = match run.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => f64::from(width) / 2.0,
            TextAlign::Right => f64::from(width),
        };
        let transform =
            Affine::translate((run.position.0, run.position.1 + f64::from(size) * 0.34))
                * Affine::rotate(-run.rotation_deg.to_radians())
                * Affine::translate((-offset, 0.0));
        let target = (run.rotation_deg.abs() <= f64::EPSILON).then(|| {
            let x0 = run.position.0 - offset;
            let count = text.chars().count();
            let character_x = (0..=count)
                .map(|index| x0 + f64::from(width) * index as f64 / count.max(1) as f64)
                .collect();
            SelectableText {
                text: text.to_owned(),
                rect: ScreenRect {
                    x0,
                    y0: run.position.1 - f64::from(size) * 0.58,
                    x1: x0 + f64::from(width),
                    y1: run.position.1 + f64::from(size) * 0.58,
                },
                character_x,
                select_whole: false,
            }
        });
        scene
            .draw_glyphs(&self.data)
            .font_size(size)
            .transform(transform)
            .brush(run.color)
            .draw(Fill::NonZero, glyphs.into_iter());
        target
    }
}

pub(super) fn draw_selectable_text(
    font: &NativeFont,
    scene: &mut Scene,
    targets: &mut Vec<SelectableText>,
    text: &str,
    run: TextRun,
) {
    if let Some(target) = font.draw_with_target(scene, text, run) {
        targets.push(target);
    }
}

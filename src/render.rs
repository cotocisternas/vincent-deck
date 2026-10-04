//! Opaque CRT surfaces. Coordinates follow 04-visual-spec.md; no host text widgets.
use anyhow::{Context, Result, bail};
use fontdue::{Font, FontSettings};
use std::collections::BTreeMap;
use tiny_skia::*;

const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/TerminessNerdFontMono-Bold.ttf");
pub const ACCENTS: [&str; 8] = [
    "red", "orange", "yellow", "green", "cyan", "blue", "purple", "magenta",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette(pub BTreeMap<String, [u8; 3]>);

impl Default for Palette {
    fn default() -> Self {
        let mut colors = BTreeMap::new();
        for (name, value) in [("bg", 32), ("darker_bg", 16), ("fg", 235), ("muted", 155)] {
            colors.insert(name.into(), [value; 3]);
        }
        for name in ACCENTS {
            colors.insert(name.into(), [195; 3]);
        }
        Self(colors)
    }
}

impl Palette {
    pub fn parse(text: &str) -> Result<Self> {
        let mut colors = BTreeMap::new();
        for line in text.lines() {
            let Some((name, hex)) = line.split_once('\t') else {
                continue;
            };
            if !hex.starts_with('#') || hex.len() != 7 {
                continue;
            }
            let value = u32::from_str_radix(&hex[1..], 16)
                .with_context(|| format!("invalid palette color {name}"))?;
            colors.insert(
                name.into(),
                [(value >> 16) as u8, (value >> 8) as u8, value as u8],
            );
        }
        for name in ["bg", "darker_bg", "fg", "muted"]
            .into_iter()
            .chain(ACCENTS)
        {
            if !colors.contains_key(name) {
                bail!("missing palette color {name}")
            }
        }
        Ok(Self(colors))
    }

    fn color(&self, name: &str) -> Color {
        let [r, g, b] = self.0[name];
        Color::from_rgba8(r, g, b, 255)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Terminal,
    Browser,
    Screenshot,
    Record,
    Agent,
    Clipboard,
    Night,
    Lock,
    Volume,
    Mic,
    Workspace,
    Theme,
    Cpu,
    Memory,
    Disk,
    Network,
}

impl Action {
    pub const ALL: [Self; 16] = [
        Self::Terminal,
        Self::Browser,
        Self::Screenshot,
        Self::Record,
        Self::Agent,
        Self::Clipboard,
        Self::Night,
        Self::Lock,
        Self::Volume,
        Self::Mic,
        Self::Workspace,
        Self::Theme,
        Self::Cpu,
        Self::Memory,
        Self::Disk,
        Self::Network,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Browser => "browser",
            Self::Screenshot => "screenshot",
            Self::Record => "record",
            Self::Agent => "agent",
            Self::Clipboard => "clipboard",
            Self::Night => "night",
            Self::Lock => "lock",
            Self::Volume => "volume",
            Self::Mic => "mic",
            Self::Workspace => "workspace",
            Self::Theme => "theme",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
            Self::Network => "network",
        }
    }
    pub fn panel(self) -> bool {
        matches!(
            self,
            Self::Volume
                | Self::Mic
                | Self::Workspace
                | Self::Theme
                | Self::Cpu
                | Self::Memory
                | Self::Disk
                | Self::Network
        )
    }
    pub fn stats(self) -> bool {
        matches!(self, Self::Cpu | Self::Memory | Self::Disk | Self::Network)
    }
    fn style(self) -> (&'static str, &'static str, char, u8) {
        match self {
            Self::Terminal => ("TTY", "green", '\u{f120}', 0),
            Self::Browser => ("NET", "blue", '\u{f0ac}', 1),
            Self::Screenshot => ("SNAP", "cyan", '\u{f030}', 2),
            Self::Record => ("REC", "red", '\u{f111}', 3),
            Self::Agent => ("AGENT", "orange", '\u{f06a9}', 4),
            Self::Clipboard => ("CLIP", "yellow", '\u{f0ea}', 5),
            Self::Night => ("NIGHT", "purple", '\u{f186}', 6),
            Self::Lock => ("LOCK", "magenta", '\u{f023}', 7),
            Self::Workspace => ("WORKSPACE", "blue", '\u{f2d2}', 8),
            Self::Theme => ("THEME", "purple", '\u{f1fc}', 9),
            Self::Volume => ("VOL", "blue", '\u{f028}', 10),
            Self::Mic => ("MIC", "green", '\u{f130}', 11),
            Self::Cpu => ("CPU", "orange", '\u{f2db}', 12),
            Self::Memory => ("MEM", "purple", '\u{f2db}', 13),
            Self::Disk => ("DISK", "yellow", '\u{f0a0}', 14),
            Self::Network => ("NET", "cyan", '\u{f0ac}', 15),
        }
    }
}

/// Visible content only: no unbounded timestamps or hidden backend data in cache keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    pub stale: bool,
    pub theme_stale: bool,
    pub pending: bool,
    pub failed: bool,
    pub active: bool,
    pub elapsed: String,
    pub percent: Option<u32>,
    pub muted: bool,
    pub audio_device: String,
    pub workspace: Option<i32>,
    pub occupied: [bool; 10],
    pub window: String,
    pub theme: String,
    pub position: String,
    pub graph: Option<crate::metrics::Graph>,
    pub power_profile: String,
    pub power_stale: bool,
}

pub fn elapsed_label(seconds: u64) -> String {
    if seconds < 6000 {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    } else {
        format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60)
    }
}

pub struct Renderer {
    font: Font,
}

impl Renderer {
    /// Loads the bundled font embedded in the executable; no filesystem lookup.
    pub fn new() -> Result<Self> {
        let font =
            Font::from_bytes(FONT_BYTES, FontSettings::default()).map_err(anyhow::Error::msg)?;
        for glyph in Action::ALL
            .into_iter()
            .map(|a| a.style().2)
            .chain(['\u{f026}', '\u{f131}'])
        {
            if font.lookup_glyph_index(glyph) == 0 {
                bail!("font lacks glyph U+{:04X}", glyph as u32)
            }
        }
        Ok(Self { font })
    }

    fn width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }

    fn fit(&self, text: &str, size: f32, width: f32) -> String {
        if self.width(text, size) <= width {
            return text.into();
        }
        let mut text: String = text.chars().take(128).collect();
        while self.width(&format!("{text}~"), size) > width && !text.is_empty() {
            text.pop();
        }
        format!("{text}~")
    }

    /// Top-aligned using font ascent, preserving a shared baseline across glyphs.
    fn text(&self, canvas: &mut Pixmap, text: &str, size: f32, x: f32, top: f32, color: Color) {
        let ascent = self.font.horizontal_line_metrics(size).unwrap().ascent;
        self.baseline(canvas, text, size, x, top + ascent, color);
    }

    fn baseline(
        &self,
        canvas: &mut Pixmap,
        text: &str,
        size: f32,
        mut x: f32,
        baseline: f32,
        color: Color,
    ) {
        for ch in text.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, size);
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let alpha = bitmap[row * metrics.width + col];
                    if alpha == 0 {
                        continue;
                    }
                    let px = x.round() as i32 + metrics.xmin + col as i32;
                    let py =
                        baseline.round() as i32 - metrics.ymin - metrics.height as i32 + row as i32;
                    if px < 0
                        || py < 0
                        || px >= canvas.width() as i32
                        || py >= canvas.height() as i32
                    {
                        continue;
                    }
                    let idx = (py as usize * canvas.width() as usize + px as usize) * 4;
                    let dst = &mut canvas.data_mut()[idx..idx + 4];
                    let a = alpha as f32 / 255.0 * color.alpha();
                    for (i, src) in [color.red(), color.green(), color.blue()]
                        .into_iter()
                        .enumerate()
                    {
                        dst[i] = (src * 255.0 * a + dst[i] as f32 * (1.0 - a)).round() as u8;
                    }
                    dst[3] = 255;
                }
            }
            x += metrics.advance_width;
        }
    }

    fn glyph(&self, canvas: &mut Pixmap, ch: char, size: f32, cx: f32, cy: f32, color: Color) {
        let m = self.font.metrics(ch, size);
        let x = cx - m.width as f32 / 2.0 - m.xmin as f32;
        let baseline = cy + m.height as f32 / 2.0 + m.ymin as f32;
        self.baseline(canvas, &ch.to_string(), size, x, baseline, color);
    }

    pub fn render(&self, action: Action, palette: &Palette, content: &Content) -> Result<Vec<u8>> {
        let panel = action.panel();
        let (w, h) = if panel { (200, 100) } else { (144, 144) };
        let mut canvas = Pixmap::new(w, h).context("allocate surface")?;
        let (label, accent, mut glyph, index) = action.style();
        let c = palette.color(accent);
        let fg = palette.color("fg");
        let muted = palette.color("muted");
        let bg = palette.color("bg");
        let dark = palette.color("darker_bg");
        // Opaque sRGB interpolation, independent of host transparency handling.
        for y in 0..h {
            let t = y as f32 / (h - 1) as f32;
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                for (i, (a, b)) in [bg.red(), bg.green(), bg.blue()]
                    .into_iter()
                    .zip([dark.red(), dark.green(), dark.blue()])
                    .enumerate()
                {
                    canvas.data_mut()[idx + i] = ((a * (1.0 - t) + b * t) * 255.0).round() as u8;
                }
                canvas.data_mut()[idx + 3] = 255;
            }
        }
        let (x, y, right, bottom, radius, stroke) = if panel {
            (3., 3., 196., 96., 12., 2.)
        } else {
            (5., 5., 138., 138., 16., 3.)
        };
        let frame = rounded_rect(x, y, right, bottom, radius);
        canvas.stroke_path(
            &frame,
            &paint(c),
            &Stroke {
                width: stroke,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
        let (start, end, left, width) = if panel {
            (6, 94, 6., 189.)
        } else {
            (8, 136, 8., 129.)
        };
        for y in (start..end).step_by(4) {
            rect(
                &mut canvas,
                left,
                y as f32,
                width,
                1.,
                Color::from_rgba(0., 0., 0., 0.22).unwrap(),
            );
        }
        let text_color = if content.stale { muted } else { fg };
        if !panel {
            self.glyph(
                &mut canvas,
                glyph,
                78.,
                72.,
                64.,
                if content.stale { muted } else { c },
            );
            let active = content.active && !content.stale;
            let text = if content.failed {
                "ERROR".into()
            } else if content.pending {
                "WAIT".into()
            } else if content.stale {
                "STALE".into()
            } else if active && action == Action::Night {
                "NIGHT ON".into()
            } else if active && action == Action::Lock {
                "LOCKED".into()
            } else if active && action == Action::Record {
                content.elapsed.clone()
            } else {
                label.into()
            };
            let metrics = self.font.horizontal_line_metrics(20.).unwrap();
            let top = 144. - 13. - (metrics.ascent - metrics.descent);
            if active {
                rect(&mut canvas, 12., top - 2., 120., 26., c);
            }
            self.text(
                &mut canvas,
                &text,
                20.,
                (144. - self.width(&text, 20.)) / 2.,
                top,
                if active { dark } else { text_color },
            );
            self.text(&mut canvas, &format!("0x{index:02X}"), 18., 14., 11., muted);
            let dot = PathBuilder::from_circle(126., 18., if active { 6. } else { 3.5 }).unwrap();
            canvas.fill_path(
                &dot,
                &paint(c),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
            if content.pending {
                rect(&mut canvas, 118., 11., 12., 12., muted);
            }
        } else if action.stats() {
            self.stats_panel(&mut canvas, action, palette, content);
        } else {
            if content.muted {
                glyph = if action == Action::Mic {
                    '\u{f131}'
                } else {
                    '\u{f026}'
                };
            }
            let state_color = if content.stale {
                muted
            } else if content.muted {
                palette.color("red")
            } else {
                c
            };
            let m = self.font.metrics(glyph, 52.);
            self.glyph(
                &mut canvas,
                glyph,
                52.,
                16. + m.width as f32 / 2.,
                46.,
                state_color,
            );
            let tag = format!("0x{index:02X}");
            self.text(
                &mut canvas,
                &tag,
                12.,
                190. - self.width(&tag, 12.),
                7.,
                muted,
            );
            match action {
                Action::Volume | Action::Mic => {
                    self.text(&mut canvas, label, 16., 72., 7., text_color);
                    self.text(
                        &mut canvas,
                        &self.fit(&content.audio_device, 11., 114.),
                        11.,
                        72.,
                        25.,
                        muted,
                    );
                    let number = content
                        .percent
                        .map(|n| format!("{n}%"))
                        .unwrap_or_else(|| "--%".into());
                    self.text(
                        &mut canvas,
                        &self.fit(&number, 28., 118.),
                        28.,
                        72.,
                        39.,
                        state_color,
                    );
                    rect(&mut canvas, 72., 70., 114., 5., muted);
                    rect(
                        &mut canvas,
                        72.,
                        70.,
                        114. * content.percent.unwrap_or(0).min(100) as f32 / 100.,
                        5.,
                        state_color,
                    );
                    let status = if content.stale {
                        "STALE"
                    } else if content.muted {
                        "MUTED"
                    } else {
                        ""
                    };
                    self.text(&mut canvas, status, 14., 72., 78., text_color);
                }
                Action::Workspace => {
                    self.text(&mut canvas, label, 20., 72., 18., text_color);
                    let number = content
                        .workspace
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "--".into());
                    self.text(&mut canvas, &number, 28., 72., 38., text_color);
                    for i in 0..10 {
                        let color = if content.workspace == Some(i + 1) && !content.stale {
                            c
                        } else {
                            muted
                        };
                        let x = 72. + i as f32 * 11.5;
                        let filled =
                            content.occupied[i as usize] || content.workspace == Some(i + 1);
                        if filled {
                            rect(&mut canvas, x, 68., 8., 5., color);
                        } else {
                            let path =
                                PathBuilder::from_rect(Rect::from_xywh(x, 68., 8., 5.).unwrap());
                            canvas.stroke_path(
                                &path,
                                &paint(color),
                                &Stroke {
                                    width: 1.,
                                    ..Default::default()
                                },
                                Transform::identity(),
                                None,
                            );
                        }
                    }
                    let line = if content.stale {
                        "STALE"
                    } else if content.window.is_empty() {
                        "empty"
                    } else {
                        &content.window
                    };
                    self.text(
                        &mut canvas,
                        &self.fit(line, 14., 118.),
                        14.,
                        72.,
                        78.,
                        muted,
                    );
                }
                Action::Theme => {
                    self.text(&mut canvas, "THEME", 14., 72., 17., muted);
                    let name = if content.theme.is_empty() {
                        "Unavailable"
                    } else {
                        &content.theme
                    };
                    let size = [20., 18., 16., 14.]
                        .into_iter()
                        .find(|s| self.width(name, *s) <= 118.)
                        .unwrap_or(14.);
                    self.text(
                        &mut canvas,
                        &self.fit(name, size, 118.),
                        size,
                        72.,
                        37.,
                        text_color,
                    );
                    for (i, accent) in ACCENTS.into_iter().enumerate() {
                        rect(
                            &mut canvas,
                            72. + i as f32 * 14.,
                            65.,
                            10.,
                            10.,
                            palette.color(accent),
                        );
                    }
                    self.text(
                        &mut canvas,
                        if content.stale {
                            "STALE"
                        } else {
                            &content.position
                        },
                        14.,
                        72.,
                        78.,
                        muted,
                    );
                }
                _ => unreachable!(),
            }
            if content.failed {
                rect(&mut canvas, 68., 77., 121., 17., dark);
                self.text(&mut canvas, "ERROR", 14., 72., 78., fg);
            }
        }
        if content.theme_stale && action != Action::Theme {
            rect(
                &mut canvas,
                12.,
                if panel { 80. } else { 93. },
                49.,
                14.,
                dark,
            );
            self.text(
                &mut canvas,
                "THEME?",
                12.,
                13.,
                if panel { 80. } else { 93. },
                muted,
            );
        }
        Ok(canvas.encode_png()?)
    }

    fn stats_panel(
        &self,
        canvas: &mut Pixmap,
        action: Action,
        palette: &Palette,
        content: &Content,
    ) {
        let (label, accent, _, index) = action.style();
        let color = if content.stale {
            palette.color("muted")
        } else {
            palette.color(accent)
        };
        let secondary = palette.color("fg");
        let muted = palette.color("muted");
        self.text(canvas, label, 20., 12., 7., color);
        let tag = format!("0x{index:02X}");
        self.text(canvas, &tag, 12., 190. - self.width(&tag, 12.), 7., muted);
        let Some(graph) = &content.graph else {
            self.text(canvas, "WARMING UP", 14., 12., 35., muted);
            self.text(
                canvas,
                if content.failed {
                    "ERROR"
                } else if content.stale {
                    "STALE"
                } else {
                    "WAIT"
                },
                14.,
                72.,
                78.,
                muted,
            );
            return;
        };
        let summary = self.fit(&graph.value, 18., 176.);
        self.text(canvas, &summary, 18., 12., 28., color);
        // Full-width plot, with space for status and the left-column theme badge.
        for y in [51., 62., 73.] {
            rect(canvas, 12., y, 176., 1., palette.color("darker_bg"));
        }
        draw_trace(canvas, &graph.primary, graph.scale, color, false);
        if graph.paired {
            draw_trace(
                canvas,
                &graph.secondary,
                graph.scale,
                if content.stale { muted } else { secondary },
                true,
            );
        }
        let detail = if content.failed {
            "ERROR"
        } else if content.stale {
            "STALE"
        } else if action == Action::Cpu {
            if content.power_stale || content.power_profile.is_empty() {
                "POWER?"
            } else {
                &content.power_profile
            }
        } else {
            &graph.detail
        };
        // The plot itself carries both series; labels identify R/W or D/U.
        let size = if action == Action::Cpu { 14. } else { 12. };
        self.text(
            canvas,
            &self.fit(detail, size, 176.),
            size,
            12.,
            if action == Action::Cpu { 78. } else { 79. },
            muted,
        );
        if content.theme_stale {
            rect(canvas, 12., 79., 176., 15., palette.color("darker_bg"));
            self.text(canvas, "THEME?", 12., 13., 80., muted);
            self.text(
                canvas,
                if content.stale { "STALE" } else { "LIVE" },
                12.,
                72.,
                80.,
                muted,
            );
        }
    }
}

fn draw_trace(
    canvas: &mut Pixmap,
    samples: &std::collections::VecDeque<u64>,
    scale: u64,
    color: Color,
    dashed: bool,
) {
    let mut path = PathBuilder::new();
    for (i, value) in samples.iter().enumerate() {
        let x = 188. - (samples.len() - 1 - i) as f32 * (176. / 59.);
        let y = 73. - (*value as f64 / scale.max(1) as f64).min(1.0) as f32 * 22.;
        if i == 0 || (dashed && i % 4 == 0) {
            path.move_to(x, y);
        } else {
            path.line_to(x, y);
        }
        rect(canvas, x - 1., y - 1., 2., 2., color);
    }
    if let Some(path) = path.finish() {
        canvas.stroke_path(
            &path,
            &paint(color),
            &Stroke {
                width: 1.5,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

fn rect(canvas: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, color: Color) {
    if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        canvas.fill_rect(rect, &paint(color), Transform::identity(), None);
    }
}

fn rounded_rect(x: f32, y: f32, right: f32, bottom: f32, r: f32) -> Path {
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(right - r, y);
    p.quad_to(right, y, right, y + r);
    p.line_to(right, bottom - r);
    p.quad_to(right, bottom, right - r, bottom);
    p.line_to(x + r, bottom);
    p.quad_to(x, bottom, x, bottom - r);
    p.line_to(x, y + r);
    p.quad_to(x, y, x + r, y);
    p.close();
    p.finish().unwrap()
}

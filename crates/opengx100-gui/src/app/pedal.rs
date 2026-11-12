//! A stompbox drawn for the selected effect: enclosure colour, graphics, knobs that show the live
//! parameter values, an LED and a footswitch that toggles the effect.
//!
//! The pictures are our own drawings made with egui's painter (nothing copied from anywhere). What the
//! pedal looks like comes from the small table in [`look`]: (effect, TYPE label) → colours and shape.
//! Add a model there to give it its own look. Effects without an entry get a plain pedal in their block colour;
//! amps, wah/volume treadles and the graphic EQ have their own drawings. Routing blocks get nothing.

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};
use opengx100_protocol::tables::{FxField, FxParamDef, FxTypeDef};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// Narrow and tall, like a compact pedal.
    Compact,
    /// Wide and low, like a rectangular distortion or muff.
    Wide,
    /// Egg-shaped, like a vintage fuzz.
    Round,
    /// An amplifier: control panel with knobs above a speaker grille.
    Amp,
    /// An expression/wah treadle seen from above; the plate follows the pedal position.
    Treadle,
}

impl Shape {
    fn size(self) -> egui::Vec2 {
        match self {
            Shape::Compact => vec2(132.0, 226.0),
            Shape::Wide => vec2(196.0, 206.0),
            Shape::Round => vec2(170.0, 210.0),
            Shape::Amp => vec2(252.0, 206.0),
            Shape::Treadle => vec2(140.0, 236.0),
        }
    }
    fn corner(self) -> f32 {
        match self {
            Shape::Compact => 7.0,
            Shape::Wide => 5.0,
            Shape::Round => 80.0,
            Shape::Amp => 6.0,
            Shape::Treadle => 12.0,
        }
    }
}

/// Which knobs a pedal shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Knobs {
    /// The effect's main control (DRIVE, FUZZ, DIST, BOOST…), TONE, LEVEL.
    Drive,
    /// SUSTAIN, ATTACK, LEVEL, TONE.
    Comp,
    /// The first few continuous parameters of the effect.
    Auto,
    /// A fader for every parameter (graphic EQ).
    Faders,
    /// GAIN, BASS, MIDDLE, TREBLE, PRESENCE, LEVEL.
    Amp,
}

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub body: Color32,
    /// Text and knob legends.
    pub ink: Color32,
    /// A band across the enclosure.
    pub trim: Option<Color32>,
    /// Knob caps.
    pub knob: Color32,
    pub shape: Shape,
    pub knobs: Knobs,
    /// Amps: the control panel. Treadles: the plate.
    pub panel: Color32,
    /// Amps: the speaker grille cloth.
    pub grille: Color32,
}

fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

const BLACK: Color32 = Color32::from_rgb(20, 20, 22);
const CREAM: Color32 = Color32::from_rgb(240, 232, 205);
const WHITE: Color32 = Color32::from_rgb(245, 245, 245);

impl Look {
    const fn new(body: Color32, ink: Color32) -> Self {
        Look {
            body,
            ink,
            trim: None,
            knob: BLACK,
            shape: Shape::Compact,
            knobs: Knobs::Drive,
            panel: BLACK,
            grille: BLACK,
        }
    }
    const fn trim(mut self, c: Color32) -> Self {
        self.trim = Some(c);
        self
    }
    const fn knob(mut self, c: Color32) -> Self {
        self.knob = c;
        self
    }
    const fn shape(mut self, s: Shape) -> Self {
        self.shape = s;
        self
    }
    const fn comp(mut self) -> Self {
        self.knobs = Knobs::Comp;
        self
    }
    const fn auto(mut self) -> Self {
        self.knobs = Knobs::Auto;
        self
    }
    /// A treadle: `plate` is the colour of the moving part.
    const fn treadle(mut self, plate: Color32) -> Self {
        self.shape = Shape::Treadle;
        self.panel = plate;
        self
    }
    /// An amp: tolex (the body), control panel, grille cloth.
    const fn amp(tolex: Color32, panel: Color32, grille: Color32) -> Self {
        let ink = if lum(panel) > 120 { BLACK } else { CREAM };
        let knob = if lum(panel) > 120 { BLACK } else { Color32::from_rgb(222, 222, 222) };
        Look { body: tolex, ink, trim: None, knob, shape: Shape::Amp, knobs: Knobs::Amp, panel, grille }
    }
}

/// Rough brightness, 0–255.
const fn lum(c: Color32) -> u32 {
    (c.r() as u32 * 3 + c.g() as u32 * 6 + c.b() as u32) / 10
}

/// Effects drawn as a pedal even when the model has no entry of its own (the colour of its block is used).
const GENERIC: &[&str] = &[
    "BASS OVERDRIVE",
    "X OVERDRIVE",
    "X BASS OVERDRIVE",
    "X DISTORTION",
    "BASS DISTORTION",
    "BASS FUZZ",
    "BASS METAL DISTORTION",
    "X COMPRESSOR",
    "X BASS COMPRESSOR",
];

/// The pedal for an effect and its TYPE (`None` = the effect has no TYPE). `block` is the effect's block colour.
pub fn look(fx: &str, ty: Option<&str>, block: Option<[u8; 3]>) -> Option<Look> {
    let ty = ty.unwrap_or("");
    let l = match (fx, ty) {
        // Overdrives
        ("OVERDRIVE", "NATURAL OD") => Look::new(hex(0xE9DDA8), BLACK).trim(hex(0xC9A24A)),
        ("OVERDRIVE", "WARM OD") => Look::new(hex(0xD9822B), BLACK),
        ("OVERDRIVE", "BLUES OD") => Look::new(hex(0x2F6FC0), WHITE).knob(hex(0xDDE3EA)),
        ("OVERDRIVE", "OD-1") => Look::new(hex(0xE99A3A), BLACK),
        ("OVERDRIVE", "SD-1") => Look::new(hex(0xF2D21B), BLACK),
        ("OVERDRIVE", "CRUNCH") => Look::new(hex(0xB98A3E), BLACK).trim(BLACK),
        ("OVERDRIVE", "T-SCREAM") => Look::new(hex(0x3FA34D), WHITE).trim(hex(0x2B7A38)).knob(hex(0x1D1D1D)),
        ("OVERDRIVE", "TURBO OD") => Look::new(hex(0x1B1B1F), hex(0xF08A24)).trim(hex(0xF08A24)),
        ("OVERDRIVE", "CENTA OD") => Look::new(hex(0xC9A24A), BLACK).knob(hex(0x6B4E1A)).shape(Shape::Round),
        // Distortions
        ("DISTORTION", "DIST") => Look::new(hex(0xDADADA), BLACK).trim(hex(0xE58A1F)),
        ("DISTORTION", "DS-1") => Look::new(hex(0xEE7A1E), BLACK),
        ("DISTORTION", "A-DIST") => Look::new(hex(0x3C3F46), hex(0xE58A1F)),
        ("DISTORTION", "FAT DS") => Look::new(hex(0x8B2E2E), CREAM),
        ("DISTORTION", "LEAD DS") => Look::new(hex(0x5A2B7A), WHITE),
        ("DISTORTION", "RAT") => {
            Look::new(hex(0x161616), WHITE).trim(hex(0x8A8A8A)).shape(Shape::Wide).knob(hex(0xE8E8E8))
        }
        ("DISTORTION", "GUV DS") => Look::new(hex(0x20252B), WHITE).trim(hex(0xC8342B)).shape(Shape::Wide),
        ("DISTORTION", "DIST+") => Look::new(hex(0xF2E34B), BLACK),
        // Fuzz
        ("FUZZ", "OCT FUZZ") => Look::new(hex(0x5A3B22), CREAM),
        ("FUZZ", "'60S FUZZ") => Look::new(hex(0xB9BEC4), BLACK).shape(Shape::Round),
        ("FUZZ", "MUFF FUZZ") => Look::new(hex(0xD0D0D0), BLACK).trim(BLACK).shape(Shape::Wide),
        // Boosters
        ("BOOSTER", "MID BOOST") => Look::new(hex(0x3A7D6A), WHITE),
        ("BOOSTER", "CLEAN BOOST") => Look::new(hex(0x5CC2E3), BLACK),
        ("BOOSTER", "TREBLE BOOST") => Look::new(hex(0xC9A24A), BLACK).knob(hex(0x2A2418)),
        // Compressors
        ("COMPRESSOR", "BOSS COMP") => Look::new(hex(0x4B7FD0), WHITE).comp(),
        ("COMPRESSOR", "D-COMP") => Look::new(hex(0xC63A2D), WHITE).comp().knob(hex(0xE8E8E8)),
        ("COMPRESSOR", "ORANGE") => Look::new(hex(0xE5742A), BLACK).comp(),
        // Metal
        ("METAL DISTORTION", "METAL DS") => Look::new(hex(0x2A2A2E), hex(0xE04A3A)),
        ("METAL DISTORTION", "METAL ZONE") => {
            Look::new(hex(0x1C1C1E), hex(0xC9CCD0)).trim(hex(0xC9CCD0)).shape(Shape::Wide)
        }
        ("METAL DISTORTION", "HM-2") => Look::new(hex(0xD86A1B), BLACK),
        ("METAL DISTORTION", "METAL CORE") => Look::new(hex(0x2E3E9C), WHITE).trim(hex(0xC8342B)),
        // Amps: tolex, panel, grille.
        ("AIRD PREAMP", t) => amp_look(t),
        ("AIRD BASS PREAMP", t) => bass_amp_look(t),
        // Treadles.
        ("WAH", "CRY WAH") => Look::new(hex(0x1A1A1A), WHITE).treadle(hex(0x9A9A9E)),
        ("WAH", "VO WAH") => Look::new(hex(0xB9BDC2), BLACK).treadle(hex(0x2A2A2A)),
        ("WAH", "FAT WAH") => Look::new(hex(0x7A1F1F), CREAM).treadle(hex(0xB8B8BC)),
        ("WAH", "LIGHT WAH") => Look::new(hex(0xC8D0D8), BLACK).treadle(hex(0x55595F)),
        ("WAH", "7STRING WAH") => Look::new(hex(0x2E3A4A), WHITE).treadle(hex(0x9A9A9E)),
        ("WAH", "RESO WAH") => Look::new(hex(0x5E2E8A), WHITE).treadle(hex(0xC8C8CC)),
        ("FOOT VOLUME", _) => Look::new(hex(0x1E1E22), WHITE).treadle(hex(0x3C3C46)),
        ("WAH" | "BASS WAH" | "PEDAL BEND" | "BASS PEDAL BEND", _) => {
            let c = block_color(block);
            Look::new(mix(c, BLACK, 0.55), WHITE).treadle(mix(c, WHITE, 0.15))
        }
        // Specific colours for some of the rest.
        ("GRAPHIC EQUALIZER", _) => {
            let mut l = Look::new(hex(0x1E3E3B), WHITE).shape(Shape::Wide).knob(hex(0xDDEEEA));
            l.knobs = Knobs::Faders;
            l
        }
        ("ANALOG DELAY", _) => Look::new(hex(0x6A4A9C), WHITE).auto(),
        ("DELAY", _) => Look::new(hex(0xE8E8E8), BLACK).trim(hex(0x4B7FD0)).auto(),
        ("SPACE ECHO", _) => {
            Look::new(hex(0x2B2622), hex(0xD9B25A)).shape(Shape::Wide).knob(CREAM).trim(hex(0xD9B25A)).auto()
        }
        ("TERA ECHO", _) => Look::new(hex(0x1E5F6B), WHITE).auto(),
        ("SCRIPT PHASER", _) => Look::new(hex(0xE8731E), BLACK).auto(),
        ("CLASSIC-VIBE", _) => Look::new(hex(0x6C4AA8), WHITE).auto(),
        ("ROTARY", _) => Look::new(hex(0x8A5A33), CREAM).shape(Shape::Wide).knob(CREAM).auto(),
        // Routing blocks aren't pedals.
        ("DIVIDER" | "SPLITTER" | "MIXER", _) => return None,
        // The drive family: the colour of the block when the model isn't listed above.
        _ if GENERIC.contains(&fx)
            || matches!(fx, "OVERDRIVE" | "DISTORTION" | "FUZZ" | "BOOSTER" | "METAL DISTORTION" | "COMPRESSOR") =>
        {
            let l = Look::new(block_color(block), BLACK);
            if fx.contains("COMPRESSOR") {
                l.comp()
            } else {
                l
            }
        }
        // Everything else: a plain pedal in the effect's colour.
        _ => {
            let c = block_color(block);
            Look::new(c, if lum(c) > 120 { BLACK } else { WHITE }).auto()
        }
    };
    Some(l)
}

fn block_color(block: Option<[u8; 3]>) -> Color32 {
    let [r, g, b] = block.unwrap_or([160, 160, 160]);
    Color32::from_rgb(r, g, b)
}

fn amp_look(ty: &str) -> Look {
    let (t, p, g) = match ty {
        "TRANSPARENT" => (0x2B2D31, 0xC9CCD0, 0x3A3D42),
        "NATURAL" => (0x3A2E24, 0xD8C9A3, 0xB79F6B),
        "BOUTIQUE" => (0x1F2A38, 0x2B2B2B, 0xC8B27A),
        "SUPREME" => (0x15171A, 0xB8923A, 0x101214),
        "MAXIMUM" => (0x0F0F10, 0x2A0E0E, 0x1A1A1A),
        "JUGGERNAUT" => (0x181818, 0x7A1E1E, 0x0B0B0B),
        "X-CRUNCH" => (0x202225, 0x25507A, 0x17181A),
        "X-HI GAIN" => (0x202225, 0x7A2525, 0x17181A),
        "X-MODDED" => (0x202225, 0x8A5A1E, 0x17181A),
        "X-ULTRA" => (0x202225, 0x2E6B3A, 0x17181A),
        "X-OPTIMA" => (0x202225, 0x5A2E8A, 0x17181A),
        "X-TITAN" => (0x202225, 0xA7ABB0, 0x17181A),
        "JC-120" => (0x1B1B1D, 0xCFD2D6, 0x9DA3AA),
        "TWIN COMBO" => (0x17171A, 0xBDBFC2, 0xA9AEB4),
        "DELUXE COMBO" => (0x1A1A1C, 0xB7B9BC, 0x8F9499),
        "TWEED COMBO" => (0xC9A86A, 0x5B4630, 0x6B4A2B),
        "DIAMOND AMP" => (0x3B2F2A, 0x1B1B1B, 0xB8935A),
        "BRIT STACK" => (0x131313, 0xC9A24A, 0x0B0B0B),
        "RECTI STACK" => (0x151515, 0x8E9298, 0x1E1E1E),
        "MATCH COMBO" => (0x22262A, 0x1D1D1D, 0xC2B497),
        "BG COMBO" => (0x1D1D20, 0xA7ABB0, 0x2B2B2E),
        "ORNG STACK" => (0xD9731F, 0x2A2A2A, 0x3A2A1E),
        "BGNR UB METAL" => (0x101012, 0x3A3A3F, 0x0A0A0B),
        _ => (0x24262A, 0x9A9DA3, 0x30333A),
    };
    Look::amp(hex(t), hex(p), hex(g))
}

fn bass_amp_look(ty: &str) -> Look {
    let (t, p, g) = match ty {
        "NATURAL BASS" => (0x3A2E24, 0xD8C9A3, 0xB79F6B),
        "X-DRIVE BASS" => (0x202225, 0x25507A, 0x17181A),
        "CONCERT" => (0x1E1E20, 0xC9CCD0, 0x2B2B2B),
        "STUDIO BASS" => (0x2F3E4F, 0xD8D8D8, 0x8D8F93),
        "SILVER TUBE" => (0xB8BCC2, 0x1F1F1F, 0x3B3B3D),
        "CLASSIC BLUE" => (0x1F3F73, 0xCFCFCF, 0x2A2A2A),
        "SOLID STACK" => (0x1B1B1B, 0x7C7F85, 0x111111),
        "FAT TUBE" => (0x5A3A22, 0xE6D9B0, 0x9C7F4F),
        "DARK DRV" => (0x0F0F10, 0x4A1414, 0x151515),
        _ => (0x24262A, 0x9A9DA3, 0x30333A),
    };
    Look::amp(hex(t), hex(p), hex(g))
}

/// Parameters that make poor knobs.
const NOT_KNOBS: &[&str] = &[
    "direct-level",
    "direct-mix",
    "pedal-position",
    "pedal-min",
    "pedal-max",
    "low-cut",
    "high-cut",
    "pre-delay",
    "carry-over",
    "carryover",
    "trigger",
    "triger",
    "waveform",
    "output-mode",
    "color",
    "intelligent",
    "invert",
];

/// The knobs to show: parameters of `fx` (found by unique name), in order.
fn knob_params(fx: &'static FxTypeDef, kind: Knobs) -> Vec<&'static FxParamDef> {
    if kind == Knobs::Amp {
        return ["gain", "bass", "middle", "treble", "presence", "level"].iter().filter_map(|k| fx.param(k)).collect();
    }
    if kind == Knobs::Faders {
        return fx.params.iter().filter(|p| matches!(p.field, FxField::Param(_))).collect();
    }
    if kind == Knobs::Auto {
        return fx
            .params
            .iter()
            .filter(|p| matches!(p.field, FxField::Param(_)) && p.max - p.min >= 3)
            .filter(|p| p.values.is_empty() || p.values.len() >= 30)
            .filter(|p| !NOT_KNOBS.contains(&p.unique_name))
            .filter(|p| !["mod-", "duck-", "solo-"].iter().any(|x| p.unique_name.starts_with(x)))
            .take(4)
            .collect();
    }
    let main = ["drive", "fuzz", "dist", "boost"].iter().find_map(|k| fx.param(k));
    let names: &[&str] = match kind {
        Knobs::Drive => &["tone", "level"],
        Knobs::Comp => &["attack", "level", "tone"],
        _ => &[],
    };
    let first = if kind == Knobs::Comp { fx.param("sustain").or_else(|| fx.param("threshold")) } else { main };
    first.into_iter().chain(names.iter().filter_map(|k| fx.param(k))).collect()
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Draws the pedal (or amp). `value` gives the live value of a parameter. Returns true when its footswitch
/// (an amp's power switch) was clicked.
pub fn show(
    ui: &mut egui::Ui,
    l: &Look,
    fx: &'static FxTypeDef,
    ty: Option<&str>,
    on: bool,
    value: &dyn Fn(&FxParamDef) -> Option<i32>,
) -> bool {
    match l.shape {
        Shape::Amp => amp(ui, l, fx, ty, on, value),
        Shape::Treadle => treadle(ui, l, fx, ty, on, value),
        _ => stomp(ui, l, fx, ty, on, value),
    }
}

fn stomp(
    ui: &mut egui::Ui,
    l: &Look,
    fx: &'static FxTypeDef,
    ty: Option<&str>,
    on: bool,
    value: &dyn Fn(&FxParamDef) -> Option<i32>,
) -> bool {
    let (outer, _) = ui.allocate_exact_size(l.shape.size(), Sense::hover());
    let p = ui.painter_at(outer.expand(8.0));
    let body_rect = outer.shrink2(vec2(7.0, 0.0));
    let (w, h) = (body_rect.width(), body_rect.height());
    let at = |fx_: f32, fy: f32| pos2(body_rect.left() + w * fx_, body_rect.top() + h * fy);
    let dim = if on { 1.0 } else { 0.78 };
    let body = l.body.gamma_multiply(dim);
    let ink = l.ink.gamma_multiply(if on { 1.0 } else { 0.85 });
    let corner = l.shape.corner();

    // Jacks on the sides, then a soft shadow and the enclosure with a lighter top edge.
    for side in [-1.0f32, 1.0] {
        let x = if side < 0.0 { body_rect.left() - 6.0 } else { body_rect.right() };
        p.rect_filled(Rect::from_min_size(pos2(x, at(0.0, 0.3).y), vec2(6.0, 22.0)), 2.0, Color32::from_gray(110));
    }
    p.rect_filled(body_rect.translate(vec2(3.0, 4.0)), corner, Color32::from_black_alpha(70));
    p.rect(body_rect, corner, body, Stroke::new(2.0, mix(body, Color32::BLACK, 0.55)), egui::StrokeKind::Inside);
    p.rect_stroke(
        body_rect.shrink(3.5),
        (corner - 3.0).max(0.0),
        Stroke::new(1.0, mix(body, WHITE, 0.25)),
        egui::StrokeKind::Inside,
    );
    if let Some(t) = l.trim {
        let y = at(0.0, 0.5).y;
        p.rect_filled(
            Rect::from_min_max(pos2(body_rect.left() + 3.0, y), pos2(body_rect.right() - 3.0, y + 3.0)),
            0.0,
            t.gamma_multiply(dim),
        );
    }

    // Knobs.
    let params = knob_params(fx, l.knobs);
    let n = params.len();
    let per_row = if n == 4 { 2 } else { n.max(1) };
    let rows = n.div_ceil(per_row);
    let radius = (w * if per_row == 2 { 0.115 } else { 0.125 }).min(20.0);
    if l.knobs == Knobs::Faders {
        faders(&p, body_rect, &params, l.knob, ink, value);
    }
    for (i, prm) in params.iter().enumerate().filter(|_| l.knobs != Knobs::Faders) {
        let (row, col) = (i / per_row, i % per_row);
        let in_row = per_row.min(n - row * per_row);
        let fx_ = (col as f32 + 0.5) / in_row as f32;
        let fy = if rows == 1 { 0.2 } else { 0.16 + row as f32 * 0.23 };
        let c = at(fx_ * 0.76 + 0.12, fy);
        knob(&p, c, radius, w * 0.76 / in_row as f32, l.knob, ink, prm, value(prm));
    }

    // Names: the model large, the effect small.
    let title = ty.unwrap_or(fx.name);
    let size = (w * 0.9 / (title.chars().count() as f32 * 0.64)).clamp(8.0, 17.0);
    p.text(at(0.5, 0.585), Align2::CENTER_CENTER, title, FontId::proportional(size), ink);
    if ty.is_some() {
        p.text(at(0.5, 0.66), Align2::CENTER_CENTER, fx.name, FontId::proportional(8.5), ink.gamma_multiply(0.75));
    }

    // LED and footswitch.
    led(&p, at(0.5, 0.735), on);
    let pad = Rect::from_min_max(at(0.17, 0.79), at(0.83, 0.95));
    let id = egui::Id::unique(("pedal-sw", fx.id));
    let resp = ui.interact(pad, id, Sense::click());
    let pad_col = if resp.hovered() { Color32::from_gray(170) } else { Color32::from_gray(150) };
    p.rect(pad, 4.0, pad_col, Stroke::new(1.5, Color32::from_gray(60)), egui::StrokeKind::Inside);
    p.rect_filled(
        Rect::from_min_size(pad.min + vec2(2.0, 2.0), vec2(pad.width() - 4.0, pad.height() * 0.4)),
        3.0,
        Color32::from_white_alpha(60),
    );
    p.line_segment(
        [pad.left_center() + vec2(4.0, 0.0), pad.right_center() - vec2(4.0, 0.0)],
        Stroke::new(1.0, Color32::from_black_alpha(60)),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if on { "Click to turn off" } else { "Click to turn on" })
        .clicked()
}

/// A knob: cap, pointer showing the value (7 o'clock = min, 5 o'clock = max), tick marks, legend above.
#[allow(clippy::too_many_arguments)]
fn knob(p: &egui::Painter, c: Pos2, r: f32, room: f32, cap: Color32, ink: Color32, prm: &FxParamDef, v: Option<i32>) {
    let dir = |frac: f32| {
        let a = (-135.0 + frac * 270.0f32).to_radians();
        vec2(a.sin(), -a.cos())
    };
    for i in 0..=10 {
        let d = dir(i as f32 / 10.0);
        p.line_segment([c + d * (r + 2.5), c + d * (r + 5.0)], Stroke::new(1.0, ink.gamma_multiply(0.7)));
    }
    p.circle_filled(c + vec2(1.0, 2.0), r, Color32::from_black_alpha(90));
    p.circle(c, r, cap, Stroke::new(1.5, mix(cap, Color32::BLACK, 0.6)));
    p.circle(c, r * 0.62, mix(cap, WHITE, 0.14), Stroke::NONE);
    let frac = v.map(|v| ((v - prm.min) as f32 / (prm.max - prm.min).max(1) as f32).clamp(0.0, 1.0)).unwrap_or(0.5);
    let tip = mix(cap, WHITE, 0.9);
    p.line_segment([c + dir(frac) * (r * 0.2), c + dir(frac) * (r * 0.92)], Stroke::new(2.2, tip));
    let name = if prm.name == "EFFECT LEVEL" { "E.LEVEL" } else { prm.name };
    let size = (room * 0.95 / (name.chars().count() as f32 * 0.62)).clamp(5.5, 8.5);
    p.text(c - vec2(0.0, r + 11.0), Align2::CENTER_CENTER, name, FontId::proportional(size), ink);
}

fn frac(prm: &FxParamDef, v: Option<i32>) -> f32 {
    v.map(|v| ((v - prm.min) as f32 / (prm.max - prm.min).max(1) as f32).clamp(0.0, 1.0)).unwrap_or(0.5)
}

/// One vertical fader per parameter.
fn faders(
    p: &egui::Painter,
    r: Rect,
    params: &[&FxParamDef],
    cap: Color32,
    ink: Color32,
    value: &dyn Fn(&FxParamDef) -> Option<i32>,
) {
    let n = params.len().max(1) as f32;
    let (top, bottom) = (r.top() + r.height() * 0.1, r.top() + r.height() * 0.46);
    for (i, prm) in params.iter().enumerate() {
        let x = r.left() + r.width() * (i as f32 + 0.5) / n;
        p.line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(2.5, Color32::from_black_alpha(160)));
        let y = bottom - (bottom - top) * frac(prm, value(prm));
        let c = Rect::from_center_size(pos2(x, y), vec2(11.0, 6.0));
        p.rect(c, 1.5, cap, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Inside);
        p.line_segment([c.left_center(), c.right_center()], Stroke::new(1.0, Color32::from_black_alpha(140)));
        let name = prm.name.replace("Hz", "").replace("LEVEL", "LVL");
        p.text(pos2(x, bottom + 9.0), Align2::CENTER_CENTER, name, FontId::proportional(6.5), ink);
    }
}

fn led(p: &egui::Painter, c: Pos2, on: bool) {
    if on {
        p.circle_filled(c, 8.0, Color32::from_rgba_unmultiplied(255, 50, 40, 55));
    }
    let col = if on { Color32::from_rgb(255, 50, 40) } else { Color32::from_rgb(70, 20, 20) };
    p.circle(c, 3.6, col, Stroke::new(1.0, Color32::from_black_alpha(160)));
}

/// An amplifier: handle, control panel with knobs, speaker grille with the model on a badge.
fn amp(
    ui: &mut egui::Ui,
    l: &Look,
    fx: &'static FxTypeDef,
    ty: Option<&str>,
    on: bool,
    value: &dyn Fn(&FxParamDef) -> Option<i32>,
) -> bool {
    let (outer, _) = ui.allocate_exact_size(l.shape.size(), Sense::hover());
    let p = ui.painter_at(outer.expand(8.0));
    let dim = if on { 1.0 } else { 0.8 };
    let body = Rect::from_min_max(outer.min + vec2(2.0, 12.0), outer.max - vec2(2.0, 2.0));
    let (w, h) = (body.width(), body.height());
    let tolex = l.body.gamma_multiply(dim);
    // Handle, shadow, cabinet.
    p.rect(
        Rect::from_center_size(pos2(body.center().x, outer.top() + 7.0), vec2(w * 0.3, 10.0)),
        4.0,
        Color32::from_gray(26),
        Stroke::new(1.0, Color32::from_gray(70)),
        egui::StrokeKind::Inside,
    );
    p.rect_filled(body.translate(vec2(3.0, 4.0)), 6.0, Color32::from_black_alpha(70));
    p.rect(body, 6.0, tolex, Stroke::new(2.0, mix(tolex, Color32::BLACK, 0.6)), egui::StrokeKind::Inside);
    // Control panel.
    let panel = Rect::from_min_max(body.min + vec2(6.0, 6.0), pos2(body.right() - 6.0, body.top() + h * 0.3));
    p.rect(
        panel,
        3.0,
        l.panel.gamma_multiply(dim),
        Stroke::new(1.0, mix(l.panel, Color32::BLACK, 0.5)),
        egui::StrokeKind::Inside,
    );
    let ink = l.ink.gamma_multiply(dim);
    let params = knob_params(fx, l.knobs);
    let n = params.len().max(1) as f32;
    let span = panel.width() * 0.8;
    for (i, prm) in params.iter().enumerate() {
        let c = pos2(panel.left() + span * (i as f32 + 0.5) / n + 4.0, panel.center().y + 6.0);
        knob(&p, c, 9.0, span / n, l.knob, ink, prm, value(prm));
    }
    // Power switch with its jewel lamp.
    let jewel = pos2(panel.right() - 15.0, panel.center().y);
    led(&p, jewel, on);
    let resp = ui.interact(
        Rect::from_center_size(jewel, vec2(26.0, 26.0)),
        egui::Id::unique(("amp-power", fx.id)),
        Sense::click(),
    );
    if resp.hovered() {
        p.circle_stroke(jewel, 11.0, Stroke::new(1.5, ink));
    }
    // Grille cloth with piping, cross-hatched, and the badge.
    let grille = Rect::from_min_max(pos2(body.left() + 10.0, panel.bottom() + 8.0), body.max - vec2(10.0, 10.0));
    let g = l.grille.gamma_multiply(dim);
    p.rect_filled(grille, 2.0, g);
    let line = Stroke::new(1.0, mix(g, if lum(g) > 120 { Color32::BLACK } else { WHITE }, 0.18));
    let mut x = grille.left() + 4.0;
    while x < grille.right() {
        p.line_segment([pos2(x, grille.top()), pos2(x, grille.bottom())], line);
        x += 5.0;
    }
    let mut y = grille.top() + 4.0;
    while y < grille.bottom() {
        p.line_segment([pos2(grille.left(), y), pos2(grille.right(), y)], line);
        y += 5.0;
    }
    p.rect_stroke(grille, 2.0, Stroke::new(2.0, mix(l.panel, g, 0.4)), egui::StrokeKind::Inside);
    let title = ty.unwrap_or(fx.name);
    let badge = Rect::from_center_size(
        pos2(grille.center().x, grille.top() + 22.0),
        vec2(title.len() as f32 * 9.0 + 18.0, 22.0),
    );
    p.rect(badge, 3.0, Color32::from_black_alpha(200), Stroke::new(1.0, l.panel), egui::StrokeKind::Inside);
    p.text(badge.center(), Align2::CENTER_CENTER, title, FontId::proportional(13.0), mix(l.panel, WHITE, 0.4));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if on { "Click to turn off" } else { "Click to turn on" })
        .clicked()
}

/// A wah / volume treadle from above: the plate's shading and ribs follow the pedal position.
fn treadle(
    ui: &mut egui::Ui,
    l: &Look,
    fx: &'static FxTypeDef,
    ty: Option<&str>,
    on: bool,
    value: &dyn Fn(&FxParamDef) -> Option<i32>,
) -> bool {
    let (outer, _) = ui.allocate_exact_size(l.shape.size(), Sense::hover());
    let p = ui.painter_at(outer.expand(8.0));
    let dim = if on { 1.0 } else { 0.8 };
    let body = outer.shrink2(vec2(6.0, 0.0));
    let (w, h) = (body.width(), body.height());
    let base = l.body.gamma_multiply(dim);
    p.rect_filled(body.translate(vec2(3.0, 4.0)), 12.0, Color32::from_black_alpha(70));
    p.rect(body, 12.0, base, Stroke::new(2.0, mix(base, Color32::BLACK, 0.6)), egui::StrokeKind::Inside);
    let pos = fx.param("pedal-position").map(|prm| frac(prm, value(prm))).unwrap_or(0.5);
    // The plate, as bands that are lighter towards the toe when the pedal is rocked forward.
    let plate = Rect::from_min_max(body.min + vec2(14.0, 14.0), pos2(body.right() - 14.0, body.top() + h * 0.7));
    let bands = 10;
    for k in 0..bands {
        let t = k as f32 / (bands - 1) as f32;
        let s = (pos - 0.5) * 0.7 * (1.0 - 2.0 * t);
        let c = l.panel.gamma_multiply(dim);
        let c = if s > 0.0 { mix(c, WHITE, s) } else { mix(c, Color32::BLACK, -s) };
        let y0 = plate.top() + plate.height() * k as f32 / bands as f32;
        let r =
            Rect::from_min_max(pos2(plate.left(), y0), pos2(plate.right(), y0 + plate.height() / bands as f32 + 0.5));
        p.rect_filled(r, if k == 0 || k == bands - 1 { 8.0 } else { 0.0 }, c);
    }
    let ribs = 13;
    for k in 1..ribs {
        let t = k as f32 / ribs as f32;
        // Ribs bunch up towards the end that is rocked away.
        let t = t + (pos - 0.5) * 0.18 * (t * (1.0 - t) * 4.0);
        let y = plate.top() + plate.height() * t;
        p.line_segment(
            [pos2(plate.left() + 6.0, y), pos2(plate.right() - 6.0, y)],
            Stroke::new(2.0, Color32::from_black_alpha(90)),
        );
    }
    p.rect_stroke(plate, 8.0, Stroke::new(1.5, Color32::from_black_alpha(140)), egui::StrokeKind::Inside);
    // Pivot screws, name, LED, toe switch.
    for dx in [-1.0, 1.0] {
        p.circle(
            pos2(body.center().x + dx * (w * 0.5 - 7.0), plate.center().y),
            3.0,
            Color32::from_gray(120),
            Stroke::new(1.0, Color32::from_gray(40)),
        );
    }
    let ink = l.ink.gamma_multiply(dim);
    let title = ty.unwrap_or(fx.name);
    let size = (w * 0.9 / (title.chars().count() as f32 * 0.64)).clamp(8.0, 15.0);
    p.text(pos2(body.center().x, body.top() + h * 0.77), Align2::CENTER_CENTER, title, FontId::proportional(size), ink);
    led(&p, pos2(body.center().x, body.top() + h * 0.84), on);
    let pad = Rect::from_center_size(pos2(body.center().x, body.top() + h * 0.93), vec2(w * 0.5, h * 0.07));
    let resp = ui.interact(pad, egui::Id::unique(("treadle-sw", fx.id)), Sense::click());
    p.rect(
        pad,
        3.0,
        if resp.hovered() { Color32::from_gray(170) } else { Color32::from_gray(140) },
        Stroke::new(1.0, Color32::from_gray(50)),
        egui::StrokeKind::Inside,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if on { "Click to turn off" } else { "Click to turn on" })
        .clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_have_a_pedal() {
        for (fx, ty) in [
            ("OVERDRIVE", "T-SCREAM"),
            ("DISTORTION", "RAT"),
            ("FUZZ", "MUFF FUZZ"),
            ("BOOSTER", "CLEAN BOOST"),
            ("COMPRESSOR", "ORANGE"),
            ("METAL DISTORTION", "HM-2"),
        ] {
            assert!(look(fx, Some(ty), None).is_some(), "{fx} {ty}");
        }
        assert!(look("REVERB", Some("HALL S"), None).is_some());
        assert!(look("DIVIDER", None, None).is_none());
        assert_eq!(look("AIRD PREAMP", Some("JC-120"), None).unwrap().shape, Shape::Amp);
        assert_eq!(look("WAH", Some("CRY WAH"), None).unwrap().shape, Shape::Treadle);
        assert!(look("X OVERDRIVE", None, Some([1, 2, 3])).is_some());
    }

    #[test]
    fn a_new_model_falls_back_to_the_block_colour() {
        let l = look("OVERDRIVE", Some("SOMETHING NEW"), Some([10, 20, 30])).unwrap();
        assert_eq!(l.body, Color32::from_rgb(10, 20, 30));
    }

    #[test]
    fn every_effect_is_a_pedal_except_routing() {
        for t in opengx100_protocol::tables::FX_TYPES.iter() {
            let ty = t.param("type").and_then(|p| p.values.first().copied());
            let l = look(t.name, ty, t.color);
            let routing = matches!(t.name, "DIVIDER" | "SPLITTER" | "MIXER");
            assert_eq!(l.is_none(), routing, "{}", t.name);
            if let Some(l) = l.filter(|l| l.shape == Shape::Amp) {
                assert!(knob_params(t, l.knobs).len() >= 4, "{} has amp knobs", t.name);
            }
        }
    }
}

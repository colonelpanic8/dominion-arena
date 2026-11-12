//! Where things go on a card, as fractions of its width and height, and what a card is
//! for drawing purposes (landscape, basic, Night, frame colours).

use super::CardSize;
use dominion_protocol::api::CardInfo;

/// Height over width.
pub const FULL_RATIO: f32 = 994.0 / 630.0;
pub const LANDSCAPE_RATIO: f32 = 630.0 / 994.0;
pub const MINI_RATIO: f32 = 594.0 / 630.0;
pub const MICRO_RATIO: f32 = 112.0 / 616.0;

const LANDSCAPE_KINDS: [&str; 12] = [
    "event", "landmark", "project", "way", "ally", "trait", "prophecy", "study", "boon", "hex",
    "state", "artifact",
];

/// A box as fractions of the card's width (`x`, `w`) and height (`y`, `h`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

const fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

impl Rect {
    pub fn css(&self) -> String {
        format!(
            "left:{:.3}%;top:{:.3}%;width:{:.3}%;height:{:.3}%;",
            self.x * 100.0,
            self.y * 100.0,
            self.w * 100.0,
            self.h * 100.0
        )
    }

    /// Width in `cqw` (hundredths of the card's width).
    pub fn width_cqw(&self) -> f32 {
        self.w * 100.0
    }

    /// Height in `cqw`, for a card of this height-over-width `ratio`.
    pub fn height_cqw(&self, ratio: f32) -> f32 {
        self.h * ratio * 100.0
    }
}

/// What decides a card's look.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub landscape: bool,
    /// Copper, Estate, Curse and the other basic Supply cards: tall art, no text box.
    pub basic: bool,
    pub treasure_only: bool,
    pub heirloom: bool,
    pub dark_text: bool,
    pub dark_title: bool,
    pub curse: bool,
}

impl Shape {
    pub fn of(c: &CardInfo) -> Shape {
        let is = |t: &str| c.types.iter().any(|x| x == t);
        let night = is("Night");
        Shape {
            landscape: is_landscape(c),
            basic: c.basic,
            treasure_only: is("Treasure") && !is("Victory") && !is("Action"),
            heirloom: c.text.lines().any(|l| l.starts_with("(Heirloom:")),
            dark_text: night,
            dark_title: night && !(is("Action") || is("Duration")),
            curse: is("Curse"),
        }
    }

    /// Unknown cards: a plain Action-shaped card.
    pub fn plain() -> Shape {
        Shape {
            landscape: false,
            basic: false,
            treasure_only: false,
            heirloom: false,
            dark_text: false,
            dark_title: false,
            curse: false,
        }
    }

    pub fn ratio(&self, size: CardSize) -> f32 {
        ratio(self.landscape, size)
    }

    /// The frame image's box: inset by 2% of the card's height all round.
    pub fn template(&self, size: CardSize) -> Rect {
        let ratio = self.ratio(size);
        let dx = 0.02 * ratio;
        r(dx, 0.02, 1.0 - 2.0 * dx, 0.96)
    }

    pub fn art(&self, size: CardSize) -> Option<Rect> {
        match size {
            CardSize::Micro => None,
            _ if self.landscape => Some(r(0.04, 0.13, 0.92, 0.6)),
            CardSize::Mini => Some(r(0.055, 0.18, 0.9, 0.7)),
            CardSize::Full if self.basic => Some(r(0.059, 0.14, 0.87, 0.74)),
            CardSize::Full => Some(r(0.059, 0.11, 0.88, 0.41)),
        }
    }

    pub fn name(&self, size: CardSize) -> Rect {
        match size {
            CardSize::Full if self.landscape => r(0.32, 0.04, 0.36, 0.085),
            CardSize::Full if self.curse && self.basic => r(0.09, 0.061, 0.82, 0.097),
            CardSize::Full if self.basic && self.treasure_only => r(0.21, 0.06, 0.58, 0.097),
            CardSize::Full if self.basic => r(0.09, 0.057, 0.82, 0.097),
            CardSize::Full if self.treasure_only => r(0.21, 0.045, 0.58, 0.087),
            CardSize::Full => r(0.09, 0.049, 0.82, 0.082),
            CardSize::Mini if self.landscape => r(0.09, 0.72, 0.82, 0.24),
            CardSize::Mini => r(0.09, 0.06, 0.82, 0.145),
            CardSize::Micro => r(0.17, 0.2, 0.76, 0.6),
        }
    }

    /// The type line; landscapes have it in the corner at full size, turned 45°.
    pub fn types(&self, size: CardSize) -> Option<Rect> {
        match size {
            CardSize::Full if self.landscape => Some(r(0.85, 0.068, 0.13, 0.06)),
            CardSize::Mini if self.landscape => Some(r(0.32, 0.012, 0.36, 0.07)),
            CardSize::Full if self.basic && self.treasure_only => Some(r(0.23, 0.85, 0.54, 0.097)),
            CardSize::Full if self.basic => Some(r(0.23, 0.86, 0.54, 0.097)),
            CardSize::Full => Some(r(0.23, 0.88, 0.54, 0.068)),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<Rect> {
        if self.landscape {
            Some(r(0.09, 0.72, 0.815, 0.21))
        } else if self.basic {
            None
        } else if self.heirloom {
            Some(r(0.1, 0.55, 0.795, 0.27))
        } else {
            Some(r(0.1, 0.55, 0.795, 0.31))
        }
    }

    pub fn heirloom_band(&self) -> Rect {
        r(0.12, 0.828, 0.76, 0.04)
    }

    /// The cost's box; its height is the coin's size.
    pub fn cost(&self, size: CardSize) -> Rect {
        let ratio = self.ratio(size);
        // Square boxes: a width in card widths equal to the height.
        let sq = |x: f32, y: f32, h: f32| r(x, y, h * ratio, h);
        match size {
            CardSize::Full if self.landscape => sq(0.01, 0.02, 0.145),
            CardSize::Mini if self.landscape => sq(0.01, 0.02, 0.2),
            CardSize::Full if self.basic && self.treasure_only => sq(0.039, 0.84, 0.1),
            CardSize::Full => sq(0.039, 0.86, 0.1),
            CardSize::Mini => sq(0.015, 0.79, 0.2),
            CardSize::Micro => sq(0.025, 0.14, 0.72),
        }
    }

    /// The two coins beside a basic Treasure's name, showing what it makes.
    pub fn production(&self) -> [Rect; 2] {
        let n = self.name(CardSize::Full);
        let h = 0.1;
        let w = h * FULL_RATIO;
        let y = n.y - 0.01;
        [r(0.02, y, w, h), r(0.98 - w, y, w, h)]
    }

    pub fn set_icon(&self) -> Rect {
        if self.landscape {
            r(0.925, 0.72, 0.03, 0.03 / LANDSCAPE_RATIO)
        } else {
            let s = 0.039;
            r(0.9, 0.89, s * FULL_RATIO, s)
        }
    }
}

pub fn ratio(landscape: bool, size: CardSize) -> f32 {
    match size {
        CardSize::Full if landscape => LANDSCAPE_RATIO,
        CardSize::Mini if landscape => LANDSCAPE_RATIO,
        CardSize::Full => FULL_RATIO,
        CardSize::Mini => MINI_RATIO,
        CardSize::Micro => MICRO_RATIO,
    }
}

pub fn is_landscape(c: &CardInfo) -> bool {
    LANDSCAPE_KINDS.contains(&c.kind.as_str())
}

/// The type line as printed.
pub fn type_line(c: &CardInfo) -> String {
    c.types
        .iter()
        .filter(|t| *t != "Randomizer")
        .cloned()
        .collect::<Vec<_>>()
        .join(" - ")
}

/// Frame colours for a card drawn without its template, one per coloured type.
pub fn colors(c: &CardInfo) -> Vec<&'static str> {
    let by_kind = match c.kind.as_str() {
        "event" => Some("#c9c7c2"),
        "landmark" => Some("#6fa864"),
        "project" => Some("#e0958a"),
        "way" => Some("#bcd9f2"),
        "ally" => Some("#d9cba2"),
        "trait" => Some("#b3adc6"),
        "prophecy" => Some("#4eb0cc"),
        "study" => Some("#a99bd1"),
        "boon" => Some("#e6d75a"),
        "hex" => Some("#7b70d8"),
        "state" => Some("#b9714f"),
        "artifact" => Some("#c6936a"),
        _ => None,
    };
    if let Some(color) = by_kind {
        return vec![color];
    }
    let mut out = vec![];
    for t in &c.types {
        let color = match t.as_str() {
            "Action" | "Command" => "#dcdcd6",
            "Treasure" => "#f0d64e",
            "Victory" => "#86c25c",
            "Reaction" => "#6f9bd8",
            "Duration" => "#f0a347",
            "Reserve" => "#d6b98a",
            "Night" => "#3d4654",
            "Curse" => "#a35cc0",
            "Shelter" => "#c9614f",
            "Ruins" => "#8c6a49",
            _ => continue,
        };
        if !out.contains(&color) {
            out.push(color);
        }
    }
    // Duration, Reserve, Ruins and Shelter colour an Action's whole frame; Treasure,
    // Victory, Reaction and Night share it.
    let shares = ["Treasure", "Victory", "Reaction", "Night"];
    if out.len() > 1 && out[0] == "#dcdcd6" && !c.types.iter().any(|t| shares.contains(&t.as_str()))
    {
        out.remove(0);
    }
    if out.is_empty() {
        out.push("#dcdcd6");
    }
    out
}

/// A CSS background splitting the frame between its colours.
pub fn frame_background(colors: &[&str]) -> String {
    match colors {
        [] => "#dcdcd6".into(),
        [one] => one.to_string(),
        many => {
            let step = 100.0 / many.len() as f32;
            let stops: Vec<String> = many
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{c} {:.0}% {:.0}%", i as f32 * step, (i + 1) as f32 * step))
                .collect();
            format!("linear-gradient(115deg, {})", stops.join(", "))
        }
    }
}

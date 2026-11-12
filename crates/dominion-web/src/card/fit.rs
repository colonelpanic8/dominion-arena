//! Fitting text into a card's boxes: words wrapped into rows and the largest font size
//! at which they fit. Sizes are in whatever unit the box is given in (the renderer uses
//! `cqw`, a hundredth of the card's width), so a layout holds at every card size.

use super::text::{Line, LineKind, Piece, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Body,
    BodyBold,
    BodyItalic,
    /// The numbers on coins, Debt and VP.
    Number,
    Title,
}

impl Font {
    pub fn of(style: Style) -> Font {
        match style {
            Style::Normal => Font::Body,
            Style::Bold => Font::BodyBold,
            Style::Italic => Font::BodyItalic,
        }
    }
}

/// Text widths at a font size of 1.
pub trait Measure {
    fn width(&self, text: &str, font: Font) -> f32;
}

/// Row height of a text line, in ems of its own size.
pub const LINE_HEIGHT: f32 = 1.06;
/// A bonus line's size relative to the text.
pub const BONUS_SCALE: f32 = 1.3;
/// A Treasure's value or a Victory card's VP alone on its line.
pub const HUGE_SCALE: f32 = 2.0;
pub const HUGE_LINE_HEIGHT: f32 = 1.1;
/// The dividing line with its margins.
pub const SEPARATOR_HEIGHT: f32 = 0.8;
/// The space between paragraphs.
pub const GAP: f32 = 0.35;
/// Room above and below the text for icons standing out of their rows.
pub const PADDING: f32 = 0.1;

/// Icon widths in ems, margins included; must agree with cards.css.
pub const COIN_WIDTH: f32 = 1.2;
pub const DEBT_WIDTH: f32 = 1.2;
pub const POTION_WIDTH: f32 = 0.85;
pub const SUN_WIDTH: f32 = 1.1;
pub const SHIELD_WIDTH: f32 = 1.05;

/// Rows are filled to at most this share of the box, for what measuring can't see
/// (kerning, rounding).
const WIDTH_SLACK: f32 = 0.96;
/// Text is laid out to at most this share of its box's height, for rounding.
const HEIGHT_SLACK: f32 = 0.97;

#[derive(Clone, Debug, PartialEq)]
pub struct LaidLine {
    /// `None` for the dividing line.
    pub kind: Option<LineKind>,
    /// A paragraph gap goes before this line.
    pub gap_before: bool,
    pub rows: Vec<Vec<Piece>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    /// The body text's font size.
    pub size: f32,
    pub lines: Vec<LaidLine>,
}

fn scale(kind: Option<LineKind>) -> f32 {
    match kind {
        Some(LineKind::Bonus) => BONUS_SCALE,
        Some(LineKind::Huge) => HUGE_SCALE,
        _ => 1.0,
    }
}

fn row_height(kind: Option<LineKind>) -> f32 {
    match kind {
        Some(LineKind::Huge) => HUGE_SCALE * HUGE_LINE_HEIGHT,
        k => scale(k) * LINE_HEIGHT,
    }
}

fn gap_between(prev: Option<LineKind>, cur: Option<LineKind>) -> bool {
    use LineKind::*;
    match (prev, cur) {
        (Some(Bonus), Some(Bonus)) => false,
        (Some(Huge), _) | (_, Some(Huge)) => false,
        (Some(_), Some(_)) => true,
        _ => false,
    }
}

/// The width of one piece at size 1.
pub fn piece_width(p: &Piece, m: &dyn Measure) -> f32 {
    match p {
        Piece::Text(t, style) => m.width(t, Font::of(*style)),
        Piece::Coin(_) => COIN_WIDTH,
        Piece::Debt(_) => DEBT_WIDTH,
        Piece::Potion => POTION_WIDTH,
        Piece::Sun => SUN_WIDTH,
        Piece::Vp(n) => {
            SHIELD_WIDTH + n.map_or(0.0, |n| m.width(&n.to_string(), Font::Number) + 0.05)
        }
    }
}

/// Splits pieces into words: what lies between spaces, icons included.
fn words(pieces: &[Piece]) -> Vec<Vec<Piece>> {
    let mut words = vec![];
    let mut cur: Vec<Piece> = vec![];
    for p in pieces {
        match p {
            Piece::Text(t, style) => {
                let mut parts = t.split(' ').peekable();
                while let Some(part) = parts.next() {
                    if !part.is_empty() {
                        cur.push(Piece::Text(part.into(), *style));
                    }
                    if parts.peek().is_some() && !cur.is_empty() {
                        words.push(std::mem::take(&mut cur));
                    }
                }
            }
            p => cur.push(p.clone()),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

fn word_width(w: &[Piece], m: &dyn Measure) -> f32 {
    w.iter().map(|p| piece_width(p, m)).sum()
}

/// Joins words into a row, with a space between them carried by the text around it.
fn join(words: &[Vec<Piece>]) -> Vec<Piece> {
    let mut row: Vec<Piece> = vec![];
    for (i, w) in words.iter().enumerate() {
        for (j, p) in w.iter().enumerate() {
            let p = if i > 0 && j == 0 {
                match p {
                    Piece::Text(t, s) => Piece::Text(format!(" {t}"), *s),
                    p => {
                        push(&mut row, Piece::Text(" ".into(), Style::Normal));
                        p.clone()
                    }
                }
            } else {
                p.clone()
            };
            push(&mut row, p);
        }
    }
    row
}

fn push(row: &mut Vec<Piece>, p: Piece) {
    if let (Some(Piece::Text(prev, ps)), Piece::Text(t, s)) = (row.last_mut(), &p)
        && ps == s
    {
        prev.push_str(t);
        return;
    }
    row.push(p);
}

/// Greedy wrapping into rows of at most `width` (in ems of the line's size). `None` if
/// a single word is wider.
fn wrap(pieces: &[Piece], width: f32, m: &dyn Measure) -> Option<Vec<Vec<Piece>>> {
    let words = words(pieces);
    let space = m.width(" ", Font::Body);
    let mut rows = vec![];
    let mut start = 0;
    let mut used = 0.0;
    for (i, w) in words.iter().enumerate() {
        let ww = word_width(w, m);
        if ww > width {
            return None;
        }
        if i > start && used + space + ww > width {
            rows.push(join(&words[start..i]));
            start = i;
            used = ww;
        } else {
            used += if i > start { space + ww } else { ww };
        }
    }
    if start < words.len() || rows.is_empty() {
        rows.push(join(&words[start..]));
    }
    Some(rows)
}

fn lay_out(lines: &[Line], size: f32, width: f32, m: &dyn Measure) -> Option<(f32, Vec<LaidLine>)> {
    let mut height = 2.0 * PADDING * size;
    let mut out = vec![];
    let mut prev = None;
    for line in lines {
        let kind = line.kind();
        let gap_before = !out.is_empty() && gap_between(prev, kind);
        if gap_before {
            height += GAP * size;
        }
        let rows = match line {
            Line::Separator => {
                height += SEPARATOR_HEIGHT * size;
                vec![]
            }
            Line::Text { pieces, .. } => {
                let s = size * scale(kind);
                let rows = wrap(pieces, width * WIDTH_SLACK / s, m)?;
                height += rows.len() as f32 * row_height(kind) * size;
                rows
            }
        };
        out.push(LaidLine {
            kind,
            gap_before,
            rows,
        });
        prev = kind;
    }
    Some((height, out))
}

/// The largest body size, up to `max`, at which `lines` fit a `width` × `height` box.
pub fn fit_text(lines: &[Line], width: f32, height: f32, max: f32, m: &dyn Measure) -> TextLayout {
    let fits = |s: f32| lay_out(lines, s, width, m).filter(|(h, _)| *h <= height * HEIGHT_SLACK);
    if let Some((_, lines)) = fits(max) {
        return TextLayout { size: max, lines };
    }
    let (mut lo, mut hi) = (0.0f32, max);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if fits(mid).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let size = lo.max(max / 1000.0);
    let lines = lay_out(lines, size, width, m).map_or_else(Vec::new, |(_, l)| l);
    TextLayout { size, lines }
}

/// The size of a one-line `text` in a `width` × `height` box: `fill` of the height,
/// or less if it would be too wide.
pub fn fit_line(
    text: &str,
    font: Font,
    width: f32,
    height: f32,
    fill: f32,
    m: &dyn Measure,
) -> f32 {
    let w = m.width(text, font);
    let by_height = fill * height;
    if w <= 0.0 {
        return by_height;
    }
    by_height.min(width * WIDTH_SLACK / w)
}

/// Rough Times-like widths, for native builds and tests.
pub struct Approx;

impl Measure for Approx {
    fn width(&self, text: &str, font: Font) -> f32 {
        let bold = matches!(font, Font::BodyBold | Font::Number);
        text.chars()
            .map(|c| {
                let w = match c {
                    ' ' => 0.25,
                    'i' | 'j' | 'l' | '\'' | '.' | ',' | ';' | ':' | '!' | '|' => 0.28,
                    'f' | 't' | 'r' | 'I' | '(' | ')' | '-' => 0.35,
                    'm' | 'w' | 'M' | 'W' => 0.8,
                    c if c.is_ascii_uppercase() => 0.68,
                    c if c.is_ascii_digit() => 0.5,
                    _ => 0.48,
                };
                match font {
                    Font::Title => w * 1.25,
                    _ if bold => w * 1.06,
                    _ => w,
                }
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::text::parse;

    fn total_height(l: &TextLayout) -> f32 {
        l.lines
            .iter()
            .map(|line| {
                let gap = if line.gap_before { GAP } else { 0.0 };
                let body = match line.kind {
                    None => SEPARATOR_HEIGHT,
                    k => line.rows.len() as f32 * row_height(k),
                };
                (gap + body) * l.size
            })
            .sum()
    }

    #[test]
    fn short_texts_take_the_largest_size_and_long_ones_shrink_to_fit() {
        let village = fit_text(&parse("+1 Card\n+2 Actions"), 80.0, 49.0, 7.0, &Approx);
        assert_eq!(village.size, 7.0);
        assert_eq!(village.lines.len(), 2);

        let long = "The player to your left takes an extra turn after this one (but not a 2nd extra turn in a row), in which you can see all cards they can and make all decisions for them. Any cards or Debt they would gain on that turn, you gain instead; any cards of theirs that are trashed are set aside and put in their discard pile at end of turn.";
        let l = fit_text(&parse(long), 80.0, 49.0, 7.0, &Approx);
        assert!(l.size < 5.5, "{}", l.size);
        let h = total_height(&l);
        assert!(h <= 49.0 && h > 49.0 * 0.85, "{h}");
        for row in &l.lines[0].rows {
            let w: f32 = row.iter().map(|p| piece_width(p, &Approx)).sum();
            assert!(w * l.size <= 80.0, "{w}");
        }
    }

    #[test]
    fn rows_keep_icons_with_their_punctuation() {
        let l = fit_text(
            &parse("Gain a card costing up to $4."),
            30.0,
            40.0,
            5.0,
            &Approx,
        );
        let rows = &l.lines[0].rows;
        assert!(rows.len() > 1);
        let last = rows.last().unwrap();
        assert_eq!(
            last[last.len() - 2..],
            [Piece::Coin(Some(4)), Piece::Text(".".into(), Style::Normal)]
        );
    }
}

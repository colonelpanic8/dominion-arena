//! Where everything on the table goes, in pixels, from the window's size: dominion.games'
//! layout, in which every zone is a fixed box measured in hundredths of the window's
//! height (cards scale with it) and width (horizontal ranges). Pure, so that the worst
//! boards are checked natively.

/// Height over width of a full card, a mini (pile: art and name), a landscape and a
/// micro strip.
pub const FULL: f64 = 994.0 / 630.0;
pub const MINI: f64 = 594.0 / 630.0;
pub const WIDE: f64 = 630.0 / 994.0;
pub const MICRO: f64 = 112.0 / 616.0;

/// The right-hand column (the log) and the left one (player bars, basic piles), in vw.
pub const RIGHT_COLUMN: f64 = 23.0;
pub const LEFT_COLUMN: f64 = 15.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    /// The CSS placing an absolutely positioned box here.
    pub fn style(&self) -> String {
        format!(
            "left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px;",
            self.x, self.y, self.w, self.h
        )
    }

    /// The box a pile stack is drawn in inside its cell.
    fn inset(&self) -> Rect {
        Rect::new(
            self.x + 0.005 * self.w,
            self.y + 0.015 * self.h,
            0.97 * self.w,
            0.97 * self.h,
        )
    }
}

/// What the layout depends on besides the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub opponents: usize,
    /// Basic Victory piles and Curses (left column) and basic Treasures (right column).
    pub victory: usize,
    pub treasures: usize,
    pub kingdom: usize,
    /// Landscapes and other wide things (Artifacts, Boons and Hexes).
    pub landscapes: usize,
    /// Non-Supply piles, drawn as micro strips.
    pub non_supply: usize,
    /// An opponent's turn with "move kingdom and play area" on, and nothing for you to
    /// decide: the kingdom drops and their play row goes above it.
    pub lowered: bool,
}

/// A row of cards: where each card's left edge is, its size and its top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub xs: Vec<f64>,
    pub w: f64,
    pub h: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    /// 1vw and 1vh in pixels.
    pub vw: f64,
    pub vh: f64,
    /// Basic piles: the Victory column top to bottom, then the Treasure column.
    pub victory: Vec<Rect>,
    pub treasures: Vec<Rect>,
    /// Kingdom piles, cheapest first (filled from the bottom row up, left to right).
    pub kingdom: Vec<Rect>,
    pub landscapes: Vec<Rect>,
    pub non_supply: Vec<Rect>,
    pub kingdom_box: Rect,
    /// The play row: cards from `play_x` to `play_right`, their tops at `play_y`.
    pub play_x: f64,
    pub play_right: f64,
    pub play_y: f64,
    pub play_h: f64,
    pub status: Rect,
    /// The centre and bottom of the button row above the status bar, and a button.
    pub buttons_cx: f64,
    pub buttons_bottom: f64,
    pub button: (f64, f64),
    /// The phase-ending button inside the status bar's right end.
    pub end_button: Rect,
    /// The side column of buttons: its right edge and top.
    pub side_right: f64,
    pub side_top: f64,
    pub hand_x: f64,
    pub hand_right: f64,
    pub hand_y: f64,
    pub hand_h: f64,
    pub your_bar: Rect,
    pub your_discard: Rect,
    pub your_deck: Rect,
    pub opponent_bars: Vec<Rect>,
    /// The active opponent's piles, peeking in from the top edge upside down.
    pub opp_discard: Rect,
    pub opp_deck: Rect,
    pub opp_hand_x: f64,
    pub opp_hand_right: f64,
    pub opp_hand_y: f64,
    pub opp_hand_h: f64,
    /// The log, and what our client keeps under it.
    pub log: Rect,
    pub below_log: Rect,
}

/// The left edges of `n` cards of width `w` between `x0` and `x1`, at most `max_gap`
/// apart, overlapping evenly when they don't fit: centred, or from the left.
pub fn spread(n: usize, w: f64, x0: f64, x1: f64, max_gap: f64, centred: bool) -> Vec<f64> {
    if n == 0 {
        return vec![];
    }
    let room = x1 - x0;
    let gap = if n == 1 {
        0.0
    } else {
        ((room - w * n as f64) / (n - 1) as f64).min(max_gap)
    };
    let total = w * n as f64 + gap * (n - 1) as f64;
    let start = if centred {
        x0 + (room - total) / 2.0
    } else {
        x0
    };
    (0..n).map(|i| start + i as f64 * (w + gap)).collect()
}

impl Geometry {
    /// The hand's cards: `n` stacks, centred, at most 3vh apart.
    pub fn hand_row(&self, n: usize) -> Row {
        let w = self.hand_h / FULL;
        Row {
            xs: spread(n, w, self.hand_x, self.hand_right, 3.0 * self.vh, true),
            w,
            h: self.hand_h,
            y: self.hand_y,
        }
    }

    /// The play row's cards: from the left, each over the last by at least 2vh. Their
    /// tops step down 0, 4 and 8vh in turn ([`Geometry::play_top`]).
    pub fn play_row(&self, n: usize) -> Row {
        let w = self.play_h / FULL;
        Row {
            xs: spread(n, w, self.play_x, self.play_right, -2.0 * self.vh, false),
            w,
            h: self.play_h,
            y: self.play_y,
        }
    }

    pub fn play_top(&self, i: usize) -> f64 {
        self.play_y + (i % 3) as f64 * 4.0 * self.vh
    }

    /// The active opponent's hand: backs overlapping, centred.
    pub fn opp_hand_row(&self, n: usize) -> Row {
        let w = self.opp_hand_h / FULL;
        Row {
            xs: spread(
                n,
                w,
                self.opp_hand_x,
                self.opp_hand_right,
                -8.0 * self.vh,
                true,
            ),
            w,
            h: self.opp_hand_h,
            y: self.opp_hand_y,
        }
    }

    /// A window of `units` card widths (a landscape counts as two): centred over the
    /// table, below the kingdom's top or, when a pile is to be clicked, over the hand.
    pub fn window(&self, units: f64, low: bool) -> Rect {
        let w = (48.0 * self.vw).min(30.0 * self.vh / FULL * (2.0 + 0.8 * units));
        let h = 32.0 * self.vh;
        let x = ((100.0 - RIGHT_COLUMN + LEFT_COLUMN) * self.vw - w) / 2.0;
        let y = if low { 67.0 * self.vh } else { 17.5 * self.vh };
        Rect::new(x, y.min(100.0 * self.vh - h), w, h)
    }

    /// The cards inside a window: `n` slots over 5% to 95% of its width.
    pub fn window_row(&self, win: &Rect, n: usize, wide: &[bool]) -> Row {
        let h = (30.0 * self.vh).min(0.6 * win.h);
        let w = h / FULL;
        let units: f64 = (0..n)
            .map(|i| if wide.get(i) == Some(&true) { 2.0 } else { 1.0 })
            .sum();
        let room = 0.9 * win.w;
        let max_gap = 1.5 * self.vh;
        let gap = if n <= 1 {
            0.0
        } else {
            ((room - w * units) / (n - 1) as f64).min(max_gap)
        };
        let total = w * units + gap * n.saturating_sub(1) as f64;
        let mut x = win.x + 0.05 * win.w + (room - total).max(0.0) / 2.0;
        let mut xs = vec![];
        for i in 0..n {
            xs.push(x);
            let k = if wide.get(i) == Some(&true) { 2.0 } else { 1.0 };
            x += w * k + gap;
        }
        Row {
            xs,
            w,
            h,
            y: win.y + (win.h - h) / 2.0 + 0.075 * h,
        }
    }
}

/// One way of arranging the kingdom box, in units: a pile is one column wide (a pile's
/// width) and one row tall.
struct Grid {
    cols: f64,
    rows: f64,
    /// (column, row, columns, rows) per pile, landscape and strip.
    kingdom: Vec<(f64, f64, f64, f64)>,
    landscapes: Vec<(f64, f64, f64, f64)>,
    strips: Vec<(f64, f64, f64, f64)>,
}

/// A strip is a fifth of a row tall; four go in a pile's cell.
const STRIP_ROWS: f64 = 0.25;

fn grid(
    kingdom: usize,
    landscapes: usize,
    strips: usize,
    rows: usize,
    up: bool,
    micro_rows: usize,
) -> Grid {
    let rows_f = rows as f64;
    let cols = kingdom.div_ceil(rows).max(1) as f64;
    let mut g = Grid {
        cols,
        rows: rows_f,
        kingdom: vec![],
        landscapes: vec![],
        strips: vec![],
    };
    // Kingdom piles: cheapest at the bottom left, filled bottom row first.
    for i in 0..kingdom {
        let col = (i % cols as usize) as f64;
        let row = rows_f - 1.0 - (i / cols as usize) as f64;
        g.kingdom.push((col, row, 1.0, 1.0));
    }
    // Empty cells that take strips: (column, row, columns wide).
    let mut free_cells: Vec<(f64, f64, f64)> = vec![];
    // The partial top row's empty cells take strips.
    let used_top = kingdom % cols as usize;
    if kingdom > 0 && used_top != 0 {
        let top = rows_f - 1.0 - (kingdom / cols as usize) as f64;
        for c in used_top..cols as usize {
            free_cells.push((c as f64, top, 1.0));
        }
    }
    let mut right = cols;
    let mut bottom = rows_f;
    if landscapes > 0 {
        if up {
            if landscapes == 3 && rows == 2 {
                for i in 0..3 {
                    g.landscapes
                        .push((right, i as f64 * 2.0 / 3.0, 1.0, 2.0 / 3.0));
                }
                right += 1.0;
            } else {
                let per = rows;
                let lcols = landscapes.div_ceil(per);
                for i in 0..landscapes {
                    let c = right + 1.5 * (i / per) as f64;
                    let r = rows_f - 1.0 - (i % per) as f64;
                    g.landscapes.push((c, r, 1.5, 1.0));
                }
                let last = landscapes % per;
                if last != 0 {
                    let c = right + 1.5 * (lcols - 1) as f64;
                    for r in 0..(per - last) {
                        free_cells.push((c, r as f64, 1.5));
                    }
                }
                right += 1.5 * lcols as f64;
            }
        } else {
            let per = cols as usize;
            let lrows = landscapes.div_ceil(per);
            for i in 0..landscapes {
                let c = (i % per) as f64;
                let r = bottom + (i / per) as f64 * 2.0 / 3.0;
                g.landscapes.push((c, r, 1.0, 2.0 / 3.0));
            }
            bottom += lrows as f64 * 2.0 / 3.0;
        }
    }
    let mut placed = 0;
    'cells: for (c, r, w) in free_cells {
        for k in 0..4 {
            if placed == strips {
                break 'cells;
            }
            g.strips.push((c, r + k as f64 * STRIP_ROWS, w, STRIP_ROWS));
            placed += 1;
        }
    }
    let left = strips - placed;
    if left > 0 {
        let per_row = (right.floor() as usize).max(1);
        let rows_needed = left.div_ceil(per_row).min(micro_rows);
        let under = (rows_needed * per_row).min(left);
        for i in 0..under {
            let c = (i % per_row) as f64;
            let r = bottom + (i / per_row) as f64 * STRIP_ROWS;
            g.strips.push((c, r, 1.0, STRIP_ROWS));
        }
        if under > 0 {
            bottom += rows_needed as f64 * STRIP_ROWS;
        }
        let rest = left - under;
        if rest > 0 {
            let per_col = ((bottom / STRIP_ROWS).floor() as usize).max(1);
            for i in 0..rest {
                let c = right + (i / per_col) as f64;
                let r = (i % per_col) as f64 * STRIP_ROWS;
                g.strips.push((c, r, 1.0, STRIP_ROWS));
            }
            right += rest.div_ceil(per_col) as f64;
        }
    }
    g.cols = right;
    g.rows = bottom;
    g
}

/// The kingdom box's best arrangement: the one giving the tallest pile.
fn kingdom_layout(shape: &Shape, b: Rect) -> (Vec<Rect>, Vec<Rect>, Vec<Rect>) {
    let mut best: Option<(f64, Grid)> = None;
    for rows in 2..=5usize {
        for up in [true, false] {
            for micro in 0..=5usize {
                let g = grid(
                    shape.kingdom,
                    shape.landscapes,
                    shape.non_supply,
                    rows,
                    up,
                    micro,
                );
                let unit = (b.h / g.rows).min(b.w / g.cols * MINI);
                if best.as_ref().is_none_or(|(u, _)| unit > *u + 1e-6) {
                    best = Some((unit, g));
                }
            }
        }
    }
    let (unit, g) = best.expect("a layout");
    let cw = unit / MINI;
    let x0 = b.x + (b.w - g.cols * cw) / 2.0;
    let y0 = b.y + (b.h - g.rows * unit) / 2.0;
    let place = |&(c, r, w, h): &(f64, f64, f64, f64)| {
        Rect::new(x0 + c * cw, y0 + r * unit, w * cw, h * unit).inset()
    };
    (
        g.kingdom.iter().map(place).collect(),
        g.landscapes.iter().map(place).collect(),
        g.strips.iter().map(place).collect(),
    )
}

/// The whole table for a window of `width` × `height` pixels.
pub fn geometry(width: f64, height: f64, shape: &Shape) -> Geometry {
    let vw = width.max(1.0) / 100.0;
    let vh = height.max(1.0) / 100.0;
    let opps = shape.opponents;
    let base_rows = shape.victory.max(shape.treasures).max(1) as f64;
    let bw = (50.0 * vh / base_rows / MINI).min(7.5 * vw);
    let bh = bw * MINI;
    let bar_h = 5.5 * vh;
    let base_y = if opps > 0 {
        4.5 * vh + opps as f64 * bar_h
    } else {
        0.0
    };
    let column = |x: f64, n: usize| -> Vec<Rect> {
        (0..n)
            .map(|i| Rect::new(x, base_y + i as f64 * bh, bw, bh).inset())
            .collect()
    };
    let victory = column(0.0, shape.victory);
    let treasures = column(bw, shape.treasures);

    let r = (LEFT_COLUMN * vw).min(2.0 * bw);
    let right = (100.0 - RIGHT_COLUMN) * vw;
    let top = match (shape.lowered, opps > 0) {
        (true, _) => 38.0 * vh,
        (false, true) => 10.0 * vh,
        (false, false) => 3.5 * vh,
    };
    // Lowered, it ends above the status bar (dominion.games lets the two overlap).
    let height = if shape.lowered { 27.0 } else { 35.0 };
    let kingdom_box = Rect::new(r, top, right - r, height * vh);
    let (kingdom, landscapes, non_supply) = kingdom_layout(shape, kingdom_box);

    let play_y = if shape.lowered {
        10.0 * vh
    } else {
        kingdom_box.bottom()
    };
    let your_card_h = 18.0 * vh;
    let your_card_w = your_card_h / FULL;
    let opp_card_h = 17.0 * vh;
    let opp_card_w = opp_card_h / FULL;
    let log_h = 95.5 * vh * 0.6;
    Geometry {
        vw,
        vh,
        victory,
        treasures,
        kingdom,
        landscapes,
        non_supply,
        kingdom_box,
        play_x: 16.0 * vw,
        play_right: 76.0 * vw,
        play_y,
        play_h: 19.0 * vh,
        status: Rect::new(15.0 * vw, 66.0 * vh, 56.0 * vw, 8.0 * vh),
        buttons_cx: 43.0 * vw,
        buttons_bottom: 65.0 * vh,
        button: (21.0 * vh, 7.0 * vh),
        end_button: Rect::new(70.0 * vw - 21.0 * vh, 66.5 * vh, 21.0 * vh, 7.0 * vh),
        side_right: 76.0 * vw,
        side_top: 40.0 * vh,
        hand_x: 16.0 * vw,
        hand_right: 76.0 * vw,
        hand_y: 74.5 * vh,
        hand_h: 24.5 * vh,
        your_bar: Rect::new(0.0, 79.5 * vh, 12.5 * vw, 6.0 * vh),
        your_discard: Rect::new(0.5 * vw, 86.0 * vh, your_card_w, your_card_h),
        your_deck: Rect::new(1.0 * vw + your_card_w, 86.0 * vh, your_card_w, your_card_h),
        opponent_bars: (0..opps)
            .map(|i| Rect::new(0.0, 4.0 * vh + i as f64 * bar_h, 12.5 * vw, bar_h))
            .collect(),
        opp_discard: Rect::new(16.0 * vw, -10.0 * vh, opp_card_w, opp_card_h),
        opp_deck: Rect::new(16.5 * vw + opp_card_w, -10.0 * vh, opp_card_w, opp_card_h),
        opp_hand_x: 17.0 * vw + 36.0 * vh / FULL,
        opp_hand_right: 76.0 * vw,
        opp_hand_y: -12.0 * vh,
        opp_hand_h: 20.0 * vh,
        log: Rect::new(right, 0.0, RIGHT_COLUMN * vw, log_h),
        below_log: Rect::new(right, log_h, RIGHT_COLUMN * vw, 100.0 * vh - log_h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(kingdom: usize, landscapes: usize, non_supply: usize) -> Shape {
        Shape {
            opponents: 1,
            victory: 4,
            treasures: 3,
            kingdom,
            landscapes,
            non_supply,
            lowered: false,
        }
    }

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.x < b.right() - 0.5
            && b.x < a.right() - 0.5
            && a.y < b.bottom() - 0.5
            && b.y < a.bottom() - 0.5
    }

    #[test]
    fn ten_piles_are_two_rows_of_five_cheapest_bottom_left() {
        let g = geometry(1920.0, 1080.0, &shape(10, 0, 0));
        assert_eq!(g.kingdom.len(), 10);
        // Cells of 200.5 × 189 (as dominion.games at 1080p), drawn inset.
        assert!(
            (g.kingdom[0].h - 189.0 * 0.97).abs() < 1.0,
            "{:?}",
            g.kingdom[0]
        );
        assert!(g.kingdom[0].y > g.kingdom[5].y);
        assert!(g.kingdom[0].x < g.kingdom[4].x);
        assert!((g.kingdom[0].x - g.kingdom[5].x).abs() < 0.1);
    }

    #[test]
    fn nothing_in_the_kingdom_box_overlaps_or_leaves_it() {
        for (w, h) in [
            (1920.0, 1080.0),
            (1280.0, 720.0),
            (1024.0, 768.0),
            (2560.0, 1080.0),
        ] {
            for (k, l, n) in [
                (10, 0, 0),
                (10, 2, 3),
                (17, 4, 12),
                (8, 1, 0),
                (12, 3, 30),
                (11, 7, 11),
                (10, 5, 9),
            ] {
                let g = geometry(w, h, &shape(k, l, n));
                let all: Vec<&Rect> = g
                    .kingdom
                    .iter()
                    .chain(&g.landscapes)
                    .chain(&g.non_supply)
                    .collect();
                assert_eq!(all.len(), k + l + n);
                let b = g.kingdom_box;
                for (i, a) in all.iter().enumerate() {
                    assert!(
                        a.x >= b.x - 0.5 && a.right() <= b.right() + 0.5,
                        "{w}x{h} {k}/{l}/{n} {a:?} {b:?}"
                    );
                    assert!(
                        a.y >= b.y - 0.5 && a.bottom() <= b.bottom() + 0.5,
                        "{w}x{h} {k}/{l}/{n} {a:?} {b:?}"
                    );
                    for c in &all[i + 1..] {
                        assert!(!overlaps(a, c), "{w}x{h} {k}/{l}/{n} {a:?} {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_lowered_kingdom_ends_above_the_status_bar() {
        for (w, h) in [(2000.0, 772.0), (1920.0, 1080.0), (1280.0, 720.0)] {
            let mut s = shape(10, 2, 4);
            s.lowered = true;
            let g = geometry(w, h, &s);
            assert!(g.play_y < g.kingdom_box.y);
            for r in g.kingdom.iter().chain(&g.landscapes).chain(&g.non_supply) {
                assert!(r.bottom() <= g.status.y, "{w}x{h} {r:?} {:?}", g.status);
            }
        }
    }

    #[test]
    fn rows_stay_in_their_range() {
        let g = geometry(1280.0, 720.0, &shape(10, 0, 0));
        for n in [1, 5, 9, 20] {
            let r = g.hand_row(n);
            assert!(r.xs[0] >= g.hand_x - 0.1);
            assert!(r.xs[n - 1] + r.w <= g.hand_right + 0.1);
            let p = g.play_row(n);
            assert!((p.xs[0] - g.play_x).abs() < 0.1);
            assert!(p.xs[n - 1] + p.w <= g.play_right + 0.1 || n == 1);
            for w in p.xs.windows(2) {
                assert!(w[1] - w[0] <= p.w - 2.0 * g.vh + 0.1);
            }
        }
    }

    #[test]
    fn basic_piles_stack_below_the_opponents() {
        let mut s = shape(10, 0, 0);
        s.opponents = 3;
        s.victory = 5;
        let g = geometry(1920.0, 1080.0, &s);
        assert!(g.victory[0].y >= g.opponent_bars[2].bottom() - 1.0);
        assert!(g.treasures[0].x > g.victory[0].right() - 1.0);
        assert!(g.victory[4].bottom() <= 1080.0);
    }
}

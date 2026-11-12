//! How big everything in the middle row and the hand is, in pixels, from the measured
//! size of their boxes: the Supply band (basic piles, the Kingdom, landscapes and other
//! wide things, non-Supply piles) above the play area, and the cards of the play area
//! and the hand. Pure, so that the worst boards are checked natively.

/// Height over width of a full card, a mini (pile), a landscape and a micro strip.
pub const FULL: f64 = 994.0 / 630.0;
pub const MINI: f64 = 594.0 / 630.0;
pub const WIDE: f64 = 630.0 / 994.0;
pub const MICRO: f64 = 112.0 / 616.0;
/// The trash mat's height over its width.
pub const TRASH: f64 = 0.5;

/// What the Supply band holds, as far as its size goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SupplyShape {
    /// Basic piles: Treasures, then the rest (each group fills its own columns).
    pub treasures: usize,
    pub others: usize,
    /// Kingdom rows and the longest row.
    pub kingdom_rows: usize,
    pub kingdom_cols: usize,
    /// The wide items in order (landscapes, Artifacts, Boons and Hexes, the trash last),
    /// by kind.
    pub wide: Vec<WideItem>,
    pub non_supply: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WideItem {
    Landscape,
    /// A Boons or Hexes deck beside its discard pile: two half-width landscapes.
    Deck,
    Trash,
}

impl WideItem {
    fn height(self, w: f64, gap: f64) -> f64 {
        match self {
            WideItem::Landscape => w * WIDE,
            WideItem::Deck => (w - gap) / 2.0 * WIDE,
            WideItem::Trash => w * TRASH,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Middle {
    /// The Supply band's height; the play area gets the rest.
    pub supply_h: f64,
    pub play_h: f64,
    /// Kingdom and basic pile widths.
    pub k: f64,
    pub b: f64,
    /// Basic piles per column.
    pub basic_rows: usize,
    /// The wide items' width and the indices in each of their columns.
    pub wide_w: f64,
    pub wide_cols: Vec<Vec<usize>>,
    /// Non-Supply piles: their width, whether as micro strips, and per column.
    pub ns_w: f64,
    pub ns_micro: bool,
    pub ns_rows: usize,
    /// The band's width.
    pub width: f64,
}

/// Smallest pile widths worth drawing: below them names can't be read.
const MIN_K: f64 = 56.0;
const MIN_MICRO: f64 = 96.0;

fn columns_of(n: usize, rows: usize) -> usize {
    if n == 0 { 0 } else { n.div_ceil(rows.max(1)) }
}

/// Packs `items` top to bottom into columns of height `h`, in order.
fn pack(items: &[WideItem], w: f64, h: f64, gap: f64) -> Vec<Vec<usize>> {
    let mut cols: Vec<Vec<usize>> = vec![];
    let mut used = f64::INFINITY;
    for (i, it) in items.iter().enumerate() {
        let ih = it.height(w, gap);
        if used + gap + ih > h + 0.01 {
            cols.push(vec![i]);
            used = ih;
        } else {
            cols.last_mut().expect("a column").push(i);
            used += gap + ih;
        }
    }
    cols
}

/// The widest the wide items can be in at most `ncols` columns of height `h`, capped.
fn wide_width(items: &[WideItem], ncols: usize, h: f64, gap: f64, cap: f64) -> f64 {
    let fits = |w: f64| pack(items, w, h, gap).len() <= ncols;
    if fits(cap) {
        return cap;
    }
    let (mut lo, mut hi) = (0.0, cap);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) { lo = mid } else { hi = mid }
    }
    lo
}

struct Candidate {
    m: Middle,
    score: f64,
}

/// The middle row's layout for a box of `w` × `h` with gaps of `gap`, or the best that
/// fits if nothing is comfortable.
pub fn middle(shape: &SupplyShape, w: f64, h: f64, gap: f64) -> Middle {
    // Minimum sizes give way before the layout falls back to the crudest one.
    for relax in [1.0, 0.75, 0.5] {
        if let Some(m) = search(shape, w, h, gap, relax) {
            return m;
        }
    }
    fallback(shape, w, h, gap)
}

fn search(shape: &SupplyShape, w: f64, h: f64, gap: f64, relax: f64) -> Option<Middle> {
    let mut best: Option<Candidate> = None;
    let rows = shape.kingdom_rows.max(1);
    let basic_max = shape.treasures.max(shape.others).clamp(3, 5);
    for step in 0..=16 {
        let supply_h = h * (0.42 + 0.025 * f64::from(step));
        let play_h = h - supply_h - gap;
        if play_h < 0.0 {
            continue;
        }
        let pw = (play_h - 2.0 * gap) / FULL;
        let k_h = (supply_h - (rows - 1) as f64 * gap) / rows as f64 / MINI;
        // Treasures in one column and the rest in another where they fit.
        for basic_rows in [basic_max] {
            let bcols =
                columns_of(shape.treasures, basic_rows) + columns_of(shape.others, basic_rows);
            let b_h = (supply_h - (basic_rows - 1) as f64 * gap) / basic_rows as f64 / MINI;
            for wide_cols in 1..=3usize {
                if shape.wide.is_empty() && wide_cols > 1 {
                    continue;
                }
                for ns_micro in [false, true] {
                    if shape.non_supply == 0 && ns_micro {
                        continue;
                    }
                    for ns_rows_try in 1..=20usize {
                        let c = layout_for(
                            shape,
                            w,
                            supply_h,
                            play_h,
                            gap,
                            pw,
                            k_h,
                            basic_rows,
                            bcols,
                            b_h,
                            wide_cols,
                            ns_micro,
                            ns_rows_try,
                            relax,
                        );
                        if let Some(c) = c
                            && best.as_ref().is_none_or(|b| c.score > b.score)
                        {
                            best = Some(c);
                        }
                    }
                }
            }
        }
    }
    best.map(|c| c.m)
}

#[allow(clippy::too_many_arguments)]
fn layout_for(
    shape: &SupplyShape,
    w: f64,
    supply_h: f64,
    play_h: f64,
    gap: f64,
    pw: f64,
    k_h: f64,
    basic_rows: usize,
    bcols: usize,
    b_h: f64,
    wide_cols: usize,
    ns_micro: bool,
    ns_rows: usize,
    relax: f64,
) -> Option<Candidate> {
    let kcols = shape.kingdom_cols.max(1) as f64;
    let n = shape.non_supply;
    // Non-Supply piles: minis in a grid, or micro strips in columns.
    let ns_rows = if n == 0 { 0 } else { ns_rows.min(n) };
    let (ns_w, ns_cols) = if n == 0 {
        (0.0, 0)
    } else if ns_micro {
        let strip_h = (supply_h - (ns_rows - 1) as f64 * gap) / ns_rows as f64;
        (strip_h / MICRO, columns_of(n, ns_rows))
    } else {
        let mh = (supply_h - (ns_rows - 1) as f64 * gap) / ns_rows as f64;
        (mh / MINI, columns_of(n, ns_rows))
    };
    // Each extra block costs a gap before it.
    let mut k = k_h;
    let mut fixed;
    let mut iter = 0;
    loop {
        iter += 1;
        let b = b_h.min(0.8 * k);
        let ns_w = if ns_micro {
            ns_w.min(1.05 * k)
        } else {
            ns_w.min(0.8 * k)
        };
        let ww = if shape.wide.is_empty() {
            0.0
        } else {
            let cap = if shape.wide.iter().all(|&i| i == WideItem::Trash) {
                k
            } else {
                1.45 * k
            };
            wide_width(&shape.wide, wide_cols, supply_h, gap, cap)
        };
        let wide_used = if shape.wide.is_empty() {
            0
        } else {
            pack(&shape.wide, ww, supply_h, gap).len()
        };
        // Blocks are two gaps apart, columns within them one.
        fixed = bcols as f64 * b
            + bcols.saturating_sub(1) as f64 * gap
            + if wide_used > 0 {
                2.0 * gap + wide_used as f64 * ww + (wide_used - 1) as f64 * gap
            } else {
                0.0
            }
            + if ns_cols > 0 {
                2.0 * gap + ns_cols as f64 * ns_w + (ns_cols - 1) as f64 * gap
            } else {
                0.0
            }
            + if bcols > 0 { 2.0 * gap } else { 0.0 };
        let k_w = (w - fixed - (kcols - 1.0) * gap) / kcols;
        if k_w >= k - 0.01 || iter > 12 {
            k = k.min(k_w);
            let b = b_h.min(0.8 * k);
            if k < MIN_K * relax || b < 0.55 * MIN_K * relax {
                return None;
            }
            let ns_w_final = if ns_micro {
                ns_w.min(1.05 * k)
            } else {
                ns_w.min(0.8 * k)
            };
            if n > 0 && ns_micro && ns_w_final < MIN_MICRO * relax {
                return None;
            }
            if n > 0 && !ns_micro && ns_w_final < (0.6 * k).max(66.0) {
                return None;
            }
            if !shape.wide.is_empty() && ww < 0.8 * k * relax {
                return None;
            }
            let wide = if shape.wide.is_empty() {
                vec![]
            } else {
                pack(&shape.wide, ww, supply_h, gap)
            };
            let width = fixed + kcols * k + (kcols - 1.0) * gap;
            if width > w + 0.5 {
                return None;
            }
            // As big as possible, the play area's cards about as wide as the Kingdom's,
            // micro strips only when minis would be small.
            let mut score = k.min(pw * 1.1) + 0.02 * (k + pw) + 0.6 * b.min(0.7 * k)
                - 2.0 * bcols as f64
                - 0.3 * (pw - 1.3 * k).max(0.0);
            if !shape.wide.is_empty() {
                score += 0.3 * ww.min(1.2 * k);
            }
            if ns_micro {
                score -= 4.0;
            }
            score -= 0.5 * wide_cols as f64;
            return Some(Candidate {
                m: Middle {
                    supply_h,
                    play_h,
                    k,
                    b,
                    basic_rows,
                    wide_w: ww,
                    wide_cols: wide,
                    ns_w: ns_w_final,
                    ns_micro,
                    ns_rows,
                    width,
                },
                score,
            });
        }
        k = k_w;
    }
}

/// Everything at its smallest, when even that doesn't fit: better small than overflowing.
fn fallback(shape: &SupplyShape, w: f64, h: f64, gap: f64) -> Middle {
    let supply_h = h * 0.6;
    let rows = shape.kingdom_rows.max(1) as f64;
    let kcols = shape.kingdom_cols.max(1) as f64;
    let basic_rows = 5;
    let bcols = columns_of(shape.treasures, 5) + columns_of(shape.others, 5);
    let ns_rows = ((supply_h + gap) / (supply_h / 10.0)).floor().max(1.0) as usize;
    let ns_rows = ns_rows.min(shape.non_supply.max(1));
    let ns_cols = columns_of(shape.non_supply, ns_rows);
    let parts = kcols + bcols as f64 * 0.6 + ns_cols as f64 * 1.2 + 2.0;
    let k = ((w - 12.0 * gap) / parts)
        .min((supply_h - (rows - 1.0) * gap) / rows / MINI)
        .max(1.0);
    let b = (0.6 * k).min((supply_h - 4.0 * gap) / 5.0 / MINI);
    let ww = 2.0 * k;
    let wide = shape
        .wide
        .iter()
        .enumerate()
        .map(|(i, _)| vec![i])
        .collect::<Vec<_>>();
    let ww = if wide.is_empty() {
        0.0
    } else {
        ww.min((w * 0.25) / wide.len() as f64)
    };
    Middle {
        supply_h,
        play_h: h - supply_h - gap,
        k,
        b,
        basic_rows,
        wide_w: ww,
        wide_cols: wide,
        ns_w: (1.2 * k).max(1.0),
        ns_micro: true,
        ns_rows,
        width: w,
    }
}

/// How a row of `n` cards sits in a box: their width, how many rows, and how far apart
/// they start when they overlap.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub w: f64,
    /// Drawn as minis (art, name and cost) rather than full cards.
    pub mini: bool,
    pub rows: usize,
    /// When set, cards start this far apart (less than their width: they overlap).
    pub step: Option<f64>,
}

/// The play area: full cards in one row while they fit at nearly full size, then minis
/// in two or three rows, overlapping only beyond that.
pub fn play(n: usize, w: f64, h: f64, gap: f64, max_w: f64) -> Row {
    let w0 = (h / FULL).min(max_w).max(1.0);
    let n = n.max(1);
    let need = |cw: f64, k: usize| k as f64 * cw + k.saturating_sub(1) as f64 * gap;
    if need(w0, n) <= w {
        return Row {
            w: w0,
            mini: false,
            rows: 1,
            step: None,
        };
    }
    let shrunk = (w - (n - 1) as f64 * gap) / n as f64;
    if shrunk >= 0.72 * w0 {
        return Row {
            w: shrunk,
            mini: false,
            rows: 1,
            step: None,
        };
    }
    for rows in 2..=3usize {
        let mw = ((h - (rows - 1) as f64 * gap) / rows as f64 / MINI).min(w0);
        let per = n.div_ceil(rows);
        if need(mw, per) <= w {
            return Row {
                w: mw,
                mini: true,
                rows,
                step: None,
            };
        }
        let fit = (w - (per - 1) as f64 * gap) / per as f64;
        if rows == 2 && fit >= 0.8 * mw {
            return Row {
                w: fit,
                mini: true,
                rows,
                step: None,
            };
        }
    }
    let rows = 3;
    let mw = ((h - 2.0 * gap) / 3.0 / MINI).min(w0);
    overlapped(n.div_ceil(rows), w, mw, true, rows)
}

/// The least of a card left showing when cards overlap: more than half, so that a
/// card's centre is always its own.
const SHOWN: f64 = 0.52;

/// `n` cards of width `cw` overlapping to fit `w`, shrinking further if they would
/// cover more than half of each other.
fn overlapped(n: usize, w: f64, cw: f64, mini: bool, rows: usize) -> Row {
    let gaps = n.saturating_sub(1).max(1) as f64;
    let cw = cw.min(w / (1.0 + SHOWN * gaps));
    Row {
        w: cw,
        mini,
        rows,
        step: Some((w - cw) / gaps),
    }
}

/// The hand: full cards, shrinking to four fifths before they overlap.
pub fn hand(n: usize, w: f64, h: f64, gap: f64) -> Row {
    let w0 = (h / FULL).max(1.0);
    let n = n.max(1);
    let need = n as f64 * w0 + (n - 1) as f64 * gap;
    if need <= w {
        return Row {
            w: w0,
            mini: false,
            rows: 1,
            step: None,
        };
    }
    let shrunk = (w - (n - 1) as f64 * gap) / n as f64;
    if shrunk >= 0.8 * w0 {
        return Row {
            w: shrunk,
            mini: false,
            rows: 1,
            step: None,
        };
    }
    overlapped(n, w, 0.8 * w0, false, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worst() -> SupplyShape {
        let mut wide = vec![WideItem::Landscape; 8];
        wide.extend([WideItem::Deck, WideItem::Trash]);
        SupplyShape {
            treasures: 5,
            others: 7,
            kingdom_rows: 2,
            kingdom_cols: 5,
            wide,
            non_supply: 24,
        }
    }

    fn base() -> SupplyShape {
        SupplyShape {
            treasures: 3,
            others: 4,
            kingdom_rows: 2,
            kingdom_cols: 5,
            wide: vec![WideItem::Trash],
            non_supply: 0,
        }
    }

    fn check(shape: &SupplyShape, w: f64, h: f64) -> Middle {
        let g = 3.6;
        let m = middle(shape, w, h, g);
        assert!(m.width <= w + 0.5, "{m:?} wider than {w}");
        assert!(m.supply_h + g + m.play_h <= h + 0.5);
        let rows = shape.kingdom_rows as f64;
        assert!(rows * m.k * MINI + (rows - 1.0) * g <= m.supply_h + 0.5);
        let br = m.basic_rows as f64;
        assert!(br * m.b * MINI + (br - 1.0) * g <= m.supply_h + 0.5);
        for col in &m.wide_cols {
            let used: f64 = col
                .iter()
                .map(|&i| shape.wide[i].height(m.wide_w, g))
                .sum::<f64>()
                + (col.len() - 1) as f64 * g;
            assert!(used <= m.supply_h + 0.5, "{m:?}");
        }
        if shape.non_supply > 0 {
            let r = m.ns_rows as f64;
            let ih = if m.ns_micro { MICRO } else { MINI } * m.ns_w;
            assert!(r * ih + (r - 1.0) * g <= m.supply_h + 0.5);
        }
        m
    }

    #[test]
    fn the_worst_boards_fit_every_window() {
        // The middle row's box at 1280x720 .. 2560x1440.
        for (w, h) in [
            (1009.6, 392.4),
            (1010.0, 380.0),
            (1270.0, 480.0),
            (1530.0, 580.0),
            (2050.0, 780.0),
        ] {
            let m = check(&worst(), w, h);
            assert!(m.k >= MIN_K, "{m:?}");
            println!("worst {w}x{h}: {m:?}");
            let m = check(&base(), w, h);
            println!("base {w}x{h}: {m:?}");
            assert!(m.k > 0.2 * h, "{m:?}");
        }
    }

    #[test]
    fn an_absurd_board_still_fits() {
        let mut shape = worst();
        shape.non_supply = 40;
        shape.wide.extend([WideItem::Landscape; 4]);
        check(&shape, 1010.0, 380.0);
    }

    #[test]
    fn rows_shrink_before_they_overlap() {
        let r = play(3, 1000.0, 200.0, 4.0, 300.0);
        assert_eq!((r.mini, r.step), (false, None));
        assert!((r.w - 200.0 / FULL).abs() < 0.01);
        let r = play(20, 1000.0, 200.0, 4.0, 300.0);
        assert!(r.mini && r.rows >= 2);
        let r = hand(15, 600.0, 160.0, 4.0);
        let step = r.step.unwrap();
        assert!(step * 14.0 + r.w <= 600.01);
        assert!(step > r.w / 2.0);
        let r = hand(5, 600.0, 160.0, 4.0);
        assert_eq!(r.step, None);
    }
}

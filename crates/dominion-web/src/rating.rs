//! Ratings drawn in full: Arena's Glicko-2 ratings and dominion.games' levels as the
//! numbers they are made of, their uncertainty as a bell curve, and their history. Used
//! by a player's page, the Account tab and the table's ratings window. The arithmetic
//! and the shapes are pure.

use dioxus::prelude::*;
use dominion_protocol::api::{OnlineOutcome, OnlineRating};
use dominion_protocol::lobby::{Beyond, OfficialSkill, RatingInfo, SeatLevel, Stats};
use std::f64::consts::PI;

pub const CSS: &str = include_str!("../assets/rating.css");

/// A new player's deviation, the most a rating has.
pub const NEW_DEVIATION: f64 = 350.0;
/// A new player's volatility.
pub const NEW_VOLATILITY: f64 = 0.06;
/// Over this deviation a rating is provisional.
pub const PROVISIONAL: f64 = 150.0;
const GLICKO: f64 = 173.7178;

/// How much of a new player's uncertainty a rating has shed: 0 new, 1 certain.
pub fn certainty(deviation: f64, new: f64) -> f64 {
    (1.0 - deviation / new).clamp(0.0, 1.0)
}

/// Where a rating lies with about 95% confidence: within two deviations of it.
pub fn interval(rating: f64, deviation: f64) -> (f64, f64) {
    (rating - 2.0 * deviation, rating + 2.0 * deviation)
}

/// The deviation after `days` unplayed: each day is a Glicko-2 rating period without games.
pub fn idle_deviation(deviation: f64, volatility: f64, days: f64) -> f64 {
    let phi = deviation / GLICKO;
    ((phi * phi + days * volatility * volatility).sqrt() * GLICKO).min(NEW_DEVIATION)
}

/// The chance that a player rated `a` (rating, deviation) beats one rated `b`: Glicko-2's
/// expected score, counting both deviations.
pub fn expected_score(a: (f64, f64), b: (f64, f64)) -> f64 {
    let phi = ((a.1 / GLICKO).powi(2) + (b.1 / GLICKO).powi(2)).sqrt();
    let g = 1.0 / (1.0 + 3.0 * phi * phi / (PI * PI)).sqrt();
    1.0 / (1.0 + (-g * (a.0 - b.0) / GLICKO).exp())
}

/// A normal curve's height at `x`, 1 at its mean.
fn density(x: f64, mean: f64, deviation: f64) -> f64 {
    (-0.5 * ((x - mean) / deviation).powi(2)).exp()
}

/// A box of a chart: where values from `lo` to `hi` are drawn across `x0..x1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub lo: f64,
    pub hi: f64,
    pub x0: f64,
    pub x1: f64,
}

impl Axis {
    pub fn at(&self, v: f64) -> f64 {
        self.x0 + (v - self.lo) / (self.hi - self.lo).max(1e-9) * (self.x1 - self.x0)
    }
}

/// The closed outline of a normal curve between `from` and `to`, standing on `base`, its
/// mean `height` tall.
pub fn bell_path(
    axis: Axis,
    (mean, deviation): (f64, f64),
    (from, to): (f64, f64),
    base: f64,
    height: f64,
) -> String {
    let (from, to) = (from.max(axis.lo), to.min(axis.hi));
    if to <= from || deviation <= 0.0 {
        return String::new();
    }
    let steps = 80;
    let mut d = format!("M{:.1},{base:.1}", axis.at(from));
    for i in 0..=steps {
        let x = from + (to - from) * i as f64 / steps as f64;
        let y = base - density(x, mean, deviation) * height;
        d += &format!(" L{:.1},{y:.1}", axis.at(x));
    }
    d + &format!(" L{:.1},{base:.1}Z", axis.at(to))
}

/// Ticks for an axis from `lo` to `hi`: round values, about `n` of them.
pub fn ticks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let span = (hi - lo).max(1e-9);
    let raw = span / n.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    let mut v = (lo / step).ceil() * step;
    let mut out = vec![];
    while v <= hi + 1e-9 {
        out.push(v);
        v += step;
    }
    out
}

fn tick_text(v: f64, step: f64) -> String {
    if step >= 1.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// One point of a rating's history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub value: f64,
    /// Its uncertainty, drawn as a band around the line, where known.
    pub deviation: Option<f64>,
    /// A second, lower line (dominion.games' level under its skill).
    pub low: Option<f64>,
}

/// A rating on dominion.games' scale: "38.2", or "< 30.0" beyond the anchors.
pub fn official_text(o: &OfficialSkill) -> String {
    match o.beyond {
        None => format!("{:.1}", o.skill),
        Some(Beyond::Below) => format!("< {:.1}", o.skill),
        Some(Beyond::Above) => format!("> {:.1}", o.skill),
    }
}

/// "62%".
fn percent(of: u32, total: u32) -> String {
    if total == 0 {
        String::new()
    } else {
        format!("{:.0}%", of as f64 / total as f64 * 100.0)
    }
}

/// A Glicko-2 rating's bell curve: the 68% and 95% bands, the mean, and a new player's
/// curve behind it for scale.
#[component]
pub fn GlickoBell(rating: f64, deviation: f64) -> Element {
    let (w, base, top) = (440.0, 96.0, 80.0);
    let reach = (4.0 * deviation).max(250.0);
    let axis = Axis {
        lo: rating - reach,
        hi: rating + reach,
        x0: 6.0,
        x1: w - 6.0,
    };
    let step = ticks(axis.lo, axis.hi, 5);
    let tick_step = step.get(1).zip(step.first()).map_or(100.0, |(b, a)| b - a);
    let (lo95, hi95) = interval(rating, deviation);
    let full = (axis.lo, axis.hi);
    let new_player = bell_path(axis, (1500.0, NEW_DEVIATION), full, base, 14.0);
    let mid = axis.at(rating);
    let label_end = mid > w * 0.7;
    rsx! {
        svg { class: "rt-chart rt-bell", "viewBox": "0 0 {w} 118", role: "img",
            "aria-label": "Rating {rating:.0}, 95% likely between {lo95:.0} and {hi95:.0}",
            path { class: "rt-ghost", d: "{new_player}" }
            path { class: "rt-band", d: bell_path(axis, (rating, deviation), (lo95, hi95), base, top) }
            path { class: "rt-band-in", d: bell_path(axis, (rating, deviation), (rating - deviation, rating + deviation), base, top) }
            path { class: "rt-curve", d: bell_path(axis, (rating, deviation), full, base, top) }
            line { class: "rt-mean", x1: "{mid:.1}", x2: "{mid:.1}", y1: "{base - top - 4.0:.1}", y2: "{base}" }
            line { class: "rt-axis", x1: "0", x2: "{w}", y1: "{base}", y2: "{base}" }
            for v in step.iter().copied() {
                g { key: "{v}",
                line { class: "rt-axis", x1: "{axis.at(v):.1}", x2: "{axis.at(v):.1}", y1: "{base}", y2: "{base + 4.0}" }
                text { class: "rt-tick", x: "{axis.at(v):.1}", y: "{base + 15.0}", "text-anchor": "middle", {tick_text(v, tick_step)} }
                }
            }
            text {
                class: "rt-mean-label",
                x: if label_end { "{mid - 5.0:.1}" } else { "{mid + 5.0:.1}" },
                y: "{base - top + 4.0:.1}",
                "text-anchor": if label_end { "end" } else { "start" },
                "μ {rating:.0}"
            }
        }
    }
}

/// dominion.games' skill as a bell curve, with its level marked where the margin under it
/// puts it.
#[component]
pub fn LevelBell(skill: f64, deviation: f64, level: f64, best: Option<f64>) -> Element {
    let (w, base, top) = (440.0, 64.0, 50.0);
    let reach = (3.6 * deviation).max(4.0);
    let axis = Axis {
        lo: (skill - reach).min(level - 1.0),
        hi: skill + reach,
        x0: 8.0,
        x1: w - 8.0,
    };
    let step = ticks(axis.lo, axis.hi, 5);
    let tick_step = step.get(1).zip(step.first()).map_or(1.0, |(b, a)| b - a);
    let full = (axis.lo, axis.hi);
    let (mid, lv) = (axis.at(skill), axis.at(level));
    let margin = skill - level;
    let bracket = 94.0;
    rsx! {
        svg { class: "rt-chart rt-bell rt-teal", "viewBox": "0 0 {w} 124", role: "img",
            "aria-label": "Skill {skill:.2} ± {deviation:.2}; level {level:.2}",
            path { class: "rt-band", d: bell_path(axis, (skill, deviation), full, base, top) }
            path { class: "rt-band-in", d: bell_path(axis, (skill, deviation), (skill - deviation, skill + deviation), base, top) }
            path { class: "rt-curve", d: bell_path(axis, (skill, deviation), full, base, top) }
            line { class: "rt-axis", x1: "0", x2: "{w}", y1: "{base}", y2: "{base}" }
            for v in step.iter().copied() {
                text { key: "t{v}", class: "rt-tick", x: "{axis.at(v):.1}", y: "{base + 13.0}", "text-anchor": "middle", {tick_text(v, tick_step)} }
            }
            line { class: "rt-mean", x1: "{mid:.1}", x2: "{mid:.1}", y1: "{base - top - 4.0:.1}", y2: "{base}" }
            text { class: "rt-mean-label", x: "{mid + 5.0:.1}", y: "{base - top + 4.0:.1}", "skill {skill:.2}" }
            if let Some(b) = best.filter(|b| *b > axis.lo && *b < axis.hi) {
                line { class: "rt-best", x1: "{axis.at(b):.1}", x2: "{axis.at(b):.1}", y1: "{base - 7.0}", y2: "{base}" }
                text { class: "rt-best-label", x: "{axis.at(b):.1}", y: "{base - 10.0}", "text-anchor": "middle", "best" }
            }
            if margin > 0.0 {
                line { class: "rt-bracket", x1: "{lv:.1}", x2: "{mid:.1}", y1: "{bracket}", y2: "{bracket}" }
                line { class: "rt-bracket", x1: "{mid:.1}", x2: "{mid:.1}", y1: "{bracket - 4.0}", y2: "{bracket + 4.0}" }
                text { class: "rt-tick", x: "{(lv + mid) / 2.0:.1}", y: "{bracket - 5.0}", "text-anchor": "middle", "− {margin:.2}" }
            }
            line { class: "rt-level", x1: "{lv:.1}", x2: "{lv:.1}", y1: "{base - 10.0}", y2: "{bracket + 10.0}" }
            text { class: "rt-level-label", x: "{lv:.1}", y: "{bracket + 22.0}", "text-anchor": "middle", "level {level:.2}" }
        }
    }
}

/// A rating's history: its line, the band of its uncertainty where known, a lower line
/// where there is one, and each step up or down marked.
#[component]
pub fn History(
    points: Vec<Point>,
    first: String,
    last: String,
    #[props(default)] teal: bool,
    #[props(default)] steps: bool,
    #[props(default = 0)] digits: usize,
) -> Element {
    let (w, h, left, top, bottom) = (300.0, 110.0, 30.0, 6.0, 16.0);
    if points.len() < 2 {
        return rsx! {};
    }
    let all = points.iter().flat_map(|p| {
        let d = p.deviation.unwrap_or(0.0);
        [p.value - d, p.value + d, p.low.unwrap_or(p.value)]
    });
    let (mut lo, mut hi) = all.fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if hi - lo < 1e-6 {
        lo -= 1.0;
        hi += 1.0;
    }
    let pad = (hi - lo) * 0.06;
    let (lo, hi) = (lo - pad, hi + pad);
    let n = points.len();
    let x = |i: usize| left + i as f64 / (n - 1) as f64 * (w - left - 4.0);
    let y = |v: f64| top + (hi - v) / (hi - lo) * (h - top - bottom);
    let line = |f: &dyn Fn(&Point) -> Option<f64>| {
        (points.iter().enumerate())
            .filter_map(|(i, p)| f(p).map(|v| format!("{:.1},{:.1}", x(i), y(v))))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let band = if points.iter().all(|p| p.deviation.is_some()) {
        let up = line(&|p| Some(p.value + p.deviation.unwrap_or(0.0)));
        let mut down: Vec<String> = (points.iter().enumerate())
            .map(|(i, p)| format!("{:.1},{:.1}", x(i), y(p.value - p.deviation.unwrap_or(0.0))))
            .collect();
        down.reverse();
        Some(format!("{up} {}", down.join(" ")))
    } else {
        None
    };
    let low = points
        .iter()
        .all(|p| p.low.is_some())
        .then(|| line(&|p| p.low));
    let main = line(&|p| Some(p.value));
    let grid: Vec<f64> = (0..=3).map(|k| lo + (hi - lo) * k as f64 / 3.0).collect();
    let end = points[n - 1].value;
    let marks: Vec<(f64, f64, bool)> = if steps {
        (1..n)
            .map(|i| {
                (
                    x(i),
                    y(points[i].value),
                    points[i].value >= points[i - 1].value,
                )
            })
            .collect()
    } else {
        vec![]
    };
    rsx! {
        svg { class: if teal { "rt-chart rt-history rt-teal" } else { "rt-chart rt-history" }, "viewBox": "0 0 {w} {h}", role: "img",
            "aria-label": "From {points[0].value:.digits$} to {end:.digits$}",
            for v in grid.iter().copied() {
                g { key: "{v}",
                    line { class: "rt-grid", x1: "{left}", x2: "{w - 4.0}", y1: "{y(v):.1}", y2: "{y(v):.1}" }
                    text { class: "rt-tick", x: "{left - 4.0}", y: "{y(v) + 3.0:.1}", "text-anchor": "end", "{v:.digits$}" }
                }
            }
            if let Some(b) = band { polygon { class: "rt-band", points: "{b}" } }
            if let Some(l) = low { polyline { class: "rt-low", points: "{l}" } }
            polyline { class: "rt-line", points: "{main}" }
            for (i, (cx, cy, up)) in marks.iter().copied().enumerate() {
                circle { key: "{i}", class: if up { "rt-up" } else { "rt-down" }, cx: "{cx:.1}", cy: "{cy:.1}", r: "1.8" }
            }
            circle { class: "rt-end", cx: "{x(n - 1):.1}", cy: "{y(end):.1}", r: "3.2" }
            text { class: "rt-tick", x: "{left}", y: "{h - 3.0}", "{first}" }
            text { class: "rt-tick", x: "{w - 4.0}", y: "{h - 3.0}", "text-anchor": "end", "{last}" }
        }
    }
}

/// One of the numbers a rating is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub key: String,
    /// The Greek letter it goes by, if any.
    pub symbol: Option<&'static str>,
    pub value: String,
    pub unit: Option<String>,
    pub sub: String,
    /// A share drawn as a meter under it.
    pub meter: Option<f64>,
}

impl Cell {
    fn new(key: &str, symbol: Option<&'static str>, value: String, sub: String) -> Cell {
        Cell {
            key: key.into(),
            symbol,
            value,
            unit: None,
            sub,
            meter: None,
        }
    }
}

#[component]
pub fn Cells(cells: Vec<Cell>) -> Element {
    rsx! {
        div { class: "rt-cells",
            for c in cells {
                div { key: "{c.key}", class: "rt-cell",
                    span { class: "rt-key", "{c.key}"
                        if let Some(s) = c.symbol { span { class: "rt-sym", " {s}" } }
                    }
                    span { class: "rt-value", "{c.value}"
                        if let Some(u) = &c.unit { span { class: "rt-unit", " {u}" } }
                    }
                    if let Some(m) = c.meter {
                        span { class: "rt-meter", span { style: "width: {m * 100.0:.0}%" } }
                    }
                    span { class: "rt-sub", "{c.sub}" }
                }
            }
        }
    }
}

/// The numbers a Glicko-2 rating is made of.
pub fn glicko_cells(r: &RatingInfo) -> Vec<Cell> {
    let volatility = r.volatility.unwrap_or(NEW_VOLATILITY);
    let week = idle_deviation(r.deviation, volatility, 7.0) - r.deviation;
    let (floor, _) = interval(r.rating, r.deviation);
    let steady = if volatility <= NEW_VOLATILITY + 0.002 {
        "steady results"
    } else {
        "erratic results"
    };
    vec![
        Cell::new(
            "Rating",
            Some("μ"),
            format!("{:.0}", r.rating),
            "best estimate of strength".into(),
        ),
        Cell {
            unit: Some(format!("/ {NEW_DEVIATION:.0}")),
            meter: Some(certainty(r.deviation, NEW_DEVIATION)),
            ..Cell::new(
                "Deviation",
                Some("φ"),
                format!("{:.0}", r.deviation),
                format!(
                    "{:.0}% certain · +{week:.0} after a week unplayed",
                    certainty(r.deviation, NEW_DEVIATION) * 100.0
                ),
            )
        },
        Cell::new(
            "Volatility",
            Some("σ"),
            format!("{volatility:.3}"),
            format!("{steady} (new: {NEW_VOLATILITY:.2})"),
        ),
        Cell::new(
            "Conservative",
            None,
            format!("{floor:.0}"),
            "μ − 2φ: the floor it is surely above".into(),
        ),
    ]
}

/// A rating's big number with its deviation and range.
#[component]
fn Hero(value: String, plus: String, caption: Element, #[props(default)] teal: bool) -> Element {
    rsx! {
        div { class: if teal { "rt-hero rt-teal" } else { "rt-hero" },
            div { class: "rt-big", "{value}" span { class: "rt-plus", "{plus}" } }
            div { class: "rt-caption", {caption} }
        }
    }
}

/// A player's Arena rating in full: the number and its range, its curve, what it is made
/// of, its course over the latest rated games, the record and dominion.games' scale.
#[component]
pub fn ArenaRating(rating: RatingInfo, line: Vec<f64>, stats: Option<Stats>) -> Element {
    let r = rating;
    let (lo, hi) = interval(r.rating, r.deviation);
    let points: Vec<Point> = (line.iter())
        .map(|&value| Point {
            value,
            deviation: None,
            low: None,
        })
        .collect();
    let losses = r.games.saturating_sub(r.wins);
    rsx! {
        div { class: "rt rt-card",
            div { class: "rt-head",
                span { class: "rt-title", "Arena rating" }
                span { class: "rt-pill", "Glicko-2" }
                if r.deviation > PROVISIONAL { span { class: "rt-pill rt-warn", "provisional" } }
                span { class: "rt-spacer" }
                span { class: "rt-caption", "{r.games} rated games" }
            }
            div { class: "rt-top",
                Hero {
                    value: format!("{:.0}", r.rating),
                    plus: format!("± {:.0}", r.deviation),
                    caption: rsx! { "95% likely " b { "{lo:.0} – {hi:.0}" } },
                }
                GlickoBell { rating: r.rating, deviation: r.deviation }
            }
            Cells { cells: glicko_cells(&r) }
            div { class: "rt-pair",
                if points.len() >= 2 {
                    div { class: "rt-box",
                        div { class: "rt-box-title", "Rating over the latest rated games"
                            span { "{line[0]:.0} → {line[line.len() - 1]:.0}" }
                        }
                        History {
                            points,
                            first: "earlier".to_string(),
                            last: "latest".to_string(),
                            steps: true,
                        }
                    }
                }
                div { class: "rt-box",
                    div { class: "rt-box-title", "Rated record"
                        span { "{r.wins} – {losses} · {percent(r.wins, r.games)}" }
                    }
                    Record { wins: r.wins, losses, ties: 0 }
                    if let Some(s) = &stats {
                        div { class: "rt-caption", "All Arena games: {s.wins} won of {s.games}" }
                    }
                    if let Some(o) = r.official {
                        div { class: "rt-box-title rt-gap", "On dominion.games' scale"
                            span { "read off players rated on both" }
                        }
                        OfficialScale { official: o }
                    }
                }
            }
        }
    }
}

/// A win / loss / tie bar with its counts.
#[component]
pub fn Record(wins: u32, losses: u32, ties: u32) -> Element {
    rsx! {
        div { class: "rt-record",
            if wins > 0 { span { class: "rt-win", style: "flex: {wins}" } }
            if ties > 0 { span { class: "rt-tie", style: "flex: {ties}" } }
            if losses > 0 { span { class: "rt-loss", style: "flex: {losses}" } }
        }
        div { class: "rt-legend",
            span { class: "rt-win", "{wins} W" }
            span { class: "rt-loss", "{losses} L" }
            if ties > 0 { span { class: "rt-tie", "{ties} T" } }
        }
    }
}

/// An Arena rating read on dominion.games' scale: its skill and the deviation around it.
#[component]
fn OfficialScale(official: OfficialSkill) -> Element {
    let o = official;
    let axis = Axis {
        lo: o.skill - (3.0 * o.deviation).max(5.0),
        hi: o.skill + (3.0 * o.deviation).max(5.0),
        x0: 8.0,
        x1: 292.0,
    };
    let (a, b) = (
        axis.at(o.skill - o.deviation),
        axis.at(o.skill + o.deviation),
    );
    let mid = axis.at(o.skill);
    let text = official_text(&o);
    rsx! {
        svg { class: "rt-chart rt-scale rt-teal", "viewBox": "0 0 300 42", role: "img",
            "aria-label": "dominion.games skill {text} ± {o.deviation:.1}",
            rect { class: "rt-track", x: "8", y: "12", width: "284", height: "6", rx: "3" }
            rect { class: "rt-band-in", x: "{a:.1}", y: "10", width: "{(b - a).max(2.0):.1}", height: "10", rx: "5" }
            rect { class: "rt-tick-mark", x: "{mid - 1.5:.1}", y: "7", width: "3", height: "16", rx: "1.5" }
            text { class: "rt-scale-label", x: "150", y: "38", "text-anchor": "middle", "skill ≈ {text} ± {o.deviation:.1}" }
        }
    }
}

/// The numbers a dominion.games rating is made of.
pub fn level_cells(level: f64, skill: f64, deviation: f64, games: u32) -> Vec<Cell> {
    let margin = skill - level;
    let k = if deviation > 0.0 {
        margin / deviation
    } else {
        0.0
    };
    vec![
        Cell::new(
            "Skill",
            Some("μ"),
            format!("{skill:.2}"),
            "estimated strength".into(),
        ),
        Cell {
            meter: Some(certainty(deviation, 8.0)),
            ..Cell::new(
                "Deviation",
                Some("σ"),
                format!("{deviation:.2}"),
                "narrows with games played".into(),
            )
        },
        Cell {
            unit: Some(format!("{k:.1}σ")),
            ..Cell::new(
                "Margin",
                None,
                format!("−{margin:.2}"),
                "level = μ − margin".into(),
            )
        },
        Cell::new(
            "Rated games",
            None,
            games.to_string(),
            "counted in it".into(),
        ),
    ]
}

/// The Account tab's dominion.games rating in full.
#[component]
pub fn OnlineRatingCard(rating: OnlineRating) -> Element {
    let r = &rating;
    let mut cells = level_cells(r.level, r.skill, r.deviation, r.games);
    cells[3].sub = format!("{} W · {} L · {} T", r.wins, r.losses, r.ties);
    let points: Vec<Point> = (r.history.iter())
        .map(|p| Point {
            value: p.skill,
            deviation: Some(p.deviation),
            low: Some(p.level),
        })
        .collect();
    let first = r
        .history
        .first()
        .map(|p| short_date(p.date))
        .unwrap_or_default();
    let best_date = r.best_level_date.map(short_date);
    let streaks = format!(
        "streaks: best {} W · worst {} L",
        r.best_win_streak, r.best_loss_streak
    );
    rsx! {
        div { class: "rt rt-card rt-online",
            div { class: "rt-top",
                Hero {
                    teal: true,
                    value: format!("{:.2}", r.level),
                    plus: r.rank.map(|n| format!("#{n}")).unwrap_or_default(),
                    caption: rsx! {
                        "level"
                        if r.provisional { span { class: "rt-pill rt-warn", "provisional" } }
                        br {}
                        "best " b { "{r.best_level:.2}" }
                        if let Some(d) = best_date { " ({d})" }
                        if let Some(n) = r.best_rank { ", best rank " b { "#{n}" } }
                    },
                }
                LevelBell { skill: r.skill, deviation: r.deviation, level: r.level, best: Some(r.best_level) }
            }
            Cells { cells }
            div { class: "rt-pair",
                if points.len() >= 2 {
                    div { class: "rt-box",
                        div { class: "rt-box-title", "History" span { "skill ± σ, level dashed" } }
                        History { points, first, last: "now".to_string(), teal: true, digits: 0 }
                    }
                }
                div { class: "rt-box",
                    if let Some(p) = r.period {
                        div { class: "rt-box-title", "This period"
                            span { "not final · {p.results} games, {p.distinct_opponents} opponents" }
                        }
                        Period { results: p.results, score: p.score, expected: p.expected_score }
                        div { class: "rt-caption",
                            "Level " b { "{p.start_level:.2} → {r.level:.2}" }
                            span { class: if r.level >= p.start_level { "rt-up-text" } else { "rt-down-text" },
                                " {r.level - p.start_level:+.2}"
                            }
                        }
                    }
                    div { class: "rt-box-title rt-gap", "Recent games" span { "{streaks}" } }
                    div { class: "rt-chips",
                        for (i, g) in r.results.iter().take(20).enumerate() {
                            span {
                                key: "{i}",
                                class: match g.outcome { OnlineOutcome::Win => "rt-chip rt-win", OnlineOutcome::Loss => "rt-chip rt-loss", OnlineOutcome::Tie => "rt-chip rt-tie" },
                                title: if g.rated { format!("{:.2} vs {:.2}", g.level, g.opponent_level) } else { "unrated".to_string() },
                                match g.outcome { OnlineOutcome::Win => "W", OnlineOutcome::Loss => "L", OnlineOutcome::Tie => "T" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Points scored in the open period against those the ratings expected.
#[component]
fn Period(results: u32, score: f64, expected: f64) -> Element {
    let n = f64::from(results.max(1));
    let axis = Axis {
        lo: 0.0,
        hi: n,
        x0: 8.0,
        x1: 292.0,
    };
    let ahead = score >= expected;
    rsx! {
        svg { class: "rt-chart rt-period", "viewBox": "0 0 300 46", role: "img",
            "aria-label": "Scored {score} against {expected:.2} expected",
            rect { class: "rt-track", x: "8", y: "14", width: "284", height: "10", rx: "5" }
            rect { class: if ahead { "rt-fill-up" } else { "rt-fill-down" }, x: "8", y: "14", width: "{(axis.at(score) - 8.0).max(0.0):.1}", height: "10", rx: "5" }
            line { class: "rt-expect", x1: "{axis.at(expected):.1}", x2: "{axis.at(expected):.1}", y1: "9", y2: "29" }
            text { class: "rt-tick", x: "{axis.at(expected):.1}", y: "41", "text-anchor": "middle", "expected {expected:.2}" }
            text { class: if ahead { "rt-up-text" } else { "rt-down-text" }, x: "{axis.at(score).clamp(40.0, 260.0):.1}", y: "9", "text-anchor": "middle",
                "scored {score} ({score - expected:+.2})"
            }
        }
    }
}

/// A rated player of a game, as the table's ratings window shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct Rated {
    pub name: String,
    pub rating: SeatRating,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SeatRating {
    Arena(RatingInfo),
    Online(SeatLevel),
}

/// The table's ratings window: each rated player's rating side by side, and in an Arena
/// game each one's chance against the others.
#[component]
pub fn Ratings(players: Vec<Rated>) -> Element {
    let arena: Vec<(String, f64, f64)> = (players.iter())
        .filter_map(|p| match &p.rating {
            SeatRating::Arena(r) => Some((p.name.clone(), r.rating, r.deviation)),
            SeatRating::Online(_) => None,
        })
        .collect();
    let pairs: Vec<(String, String, f64)> = (0..arena.len())
        .flat_map(|i| ((i + 1)..arena.len()).map(move |j| (i, j)))
        .map(|(i, j)| {
            let (a, b) = (&arena[i], &arena[j]);
            (
                a.0.clone(),
                b.0.clone(),
                expected_score((a.1, a.2), (b.1, b.2)),
            )
        })
        .collect();
    rsx! {
        div { class: "rt rt-window",
            for (a, b, p) in pairs {
                div { key: "{a}-{b}", class: "rt-matchup",
                    span { class: "rt-matchup-name", "{a}" }
                    span { class: "rt-matchup-bar",
                        span { class: "rt-matchup-a", style: "flex: {p:.3}", "{p * 100.0:.0}%" }
                        span { class: "rt-matchup-b", style: "flex: {1.0 - p:.3}", "{(1.0 - p) * 100.0:.0}%" }
                    }
                    span { class: "rt-matchup-name", "{b}" }
                }
            }
            div { class: "rt-columns",
                for p in players {
                    div { key: "{p.name}", class: "rt-column",
                        match p.rating {
                            SeatRating::Arena(r) => rsx! { SeatArena { name: p.name.clone(), rating: r } },
                            SeatRating::Online(r) => rsx! { SeatOnline { name: p.name.clone(), rating: r } },
                        }
                    }
                }
            }
            p { class: "rt-note",
                if !arena.is_empty() {
                    "Glicko-2: the curve is where the rating likely lies, the shaded band ± 2 deviations (95%), the dashed curve a new player's 1500 ± 350. Chances count both players' uncertainty."
                } else {
                    "dominion.games: level is skill less a margin for its uncertainty, so it rises as the deviation narrows."
                }
            }
        }
    }
}

#[component]
fn SeatArena(name: String, rating: RatingInfo) -> Element {
    let r = rating;
    let (lo, hi) = interval(r.rating, r.deviation);
    let mut cells = glicko_cells(&r);
    cells.remove(0);
    cells.push(Cell::new(
        "Rated games",
        None,
        r.games.to_string(),
        format!("{} won · {}", r.wins, percent(r.wins, r.games)),
    ));
    if let Some(o) = r.official {
        cells.push(Cell::new(
            "dominion.games",
            None,
            official_text(&o),
            format!("skill, ± {:.1}", o.deviation),
        ));
    }
    rsx! {
        div { class: "rt-seat-name", "{name}"
            if r.deviation > PROVISIONAL { span { class: "rt-pill rt-warn", "provisional" } }
        }
        Hero {
            value: format!("{:.0}", r.rating),
            plus: format!("± {:.0}", r.deviation),
            caption: rsx! { "95% likely " b { "{lo:.0} – {hi:.0}" } },
        }
        GlickoBell { rating: r.rating, deviation: r.deviation }
        Cells { cells }
    }
}

#[component]
fn SeatOnline(name: String, rating: SeatLevel) -> Element {
    let r = rating;
    let mut cells = level_cells(r.level, r.skill, r.deviation, r.games);
    cells[3].sub = format!("{} W · {} L · {} T", r.wins, r.losses, r.ties);
    rsx! {
        div { class: "rt-seat-name", "{name}" }
        Hero {
            teal: true,
            value: format!("{:.2}", r.level),
            plus: r.rank.map(|n| format!("#{n}")).unwrap_or_default(),
            caption: rsx! { "dominion.games level" },
        }
        LevelBell { skill: r.skill, deviation: r.deviation, level: r.level, best: None }
        Cells { cells }
    }
}

/// A day as "Mar 2026".
fn short_date(ms: i64) -> String {
    let day = crate::lobby::day(ms.max(0) as u64);
    if day.is_empty() { String::new() } else { day }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_ratings_are_even_and_a_higher_one_is_favoured_less_when_uncertain() {
        assert!((expected_score((1500.0, 50.0), (1500.0, 300.0)) - 0.5).abs() < 1e-12);
        let sure = expected_score((1700.0, 50.0), (1500.0, 50.0));
        let unsure = expected_score((1700.0, 300.0), (1500.0, 300.0));
        assert!(sure > unsure && unsure > 0.5);
        let back = expected_score((1500.0, 50.0), (1700.0, 50.0));
        assert!((sure + back - 1.0).abs() < 1e-12);
    }

    #[test]
    fn an_unplayed_rating_grows_less_certain_up_to_a_new_players() {
        let week = idle_deviation(62.0, 0.06, 7.0);
        assert!(week > 62.0 && week < 80.0);
        assert_eq!(idle_deviation(340.0, 0.06, 10_000.0), NEW_DEVIATION);
    }

    #[test]
    fn ticks_are_round_and_inside_the_span() {
        assert_eq!(
            ticks(1434.0, 1934.0, 5),
            vec![1500.0, 1600.0, 1700.0, 1800.0, 1900.0]
        );
        assert_eq!(ticks(40.0, 52.0, 5), vec![40.0, 42.5, 45.0, 47.5, 50.0]);
    }
}

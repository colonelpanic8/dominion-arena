//! Official card text (`data/cards`) parsed into lines of inline pieces: words in a
//! style and the coin, Debt, Potion, VP and Sun icons the printed cards draw.

/// One line of card text, as printed.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// The dividing line (`---`).
    Separator,
    Text {
        kind: LineKind,
        pieces: Vec<Piece>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Normal,
    /// A line that is a single vanilla bonus (`+1 Card`, `+$2`): bold and larger.
    Bonus,
    /// A Treasure's value or a Victory card's VP alone on a line (`$2`, `2 VP`).
    Huge,
    /// A line in parentheses: reminder text.
    Italic,
    /// `(Heirloom: Goat)`, printed in its own band at the bottom.
    Heirloom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Normal,
    Bold,
    Italic,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text(String, Style),
    /// `$N`; `None` for a bare `$`.
    Coin(Option<u32>),
    /// `N Debt`; `None` for a bare `Debt`.
    Debt(Option<u32>),
    Potion,
    /// `N VP`; `None` for a bare `VP`.
    Vp(Option<u32>),
    Sun,
}

impl Line {
    pub fn kind(&self) -> Option<LineKind> {
        match self {
            Line::Separator => None,
            Line::Text { kind, .. } => Some(*kind),
        }
    }
}

const BONUS_WORDS: [&str; 12] = [
    "Card",
    "Cards",
    "Action",
    "Actions",
    "Buy",
    "Buys",
    "Villager",
    "Villagers",
    "Coffers",
    "Coffer",
    "Favor",
    "Favors",
];

/// Parses a card's official text.
pub fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Line {
    if line == "---" {
        return Line::Separator;
    }
    if line.starts_with("(Heirloom:") {
        return Line::Text {
            kind: LineKind::Heirloom,
            pieces: restyle(inline(line), Style::Italic),
        };
    }
    if line.starts_with('(') && line.ends_with(')') && !line[1..].contains('(') {
        return Line::Text {
            kind: LineKind::Italic,
            pieces: restyle(inline(line), Style::Italic),
        };
    }
    let pieces = inline(line);
    let kind = if is_huge(&pieces) {
        LineKind::Huge
    } else if is_bonus(line) {
        LineKind::Bonus
    } else {
        LineKind::Normal
    };
    let pieces = if kind == LineKind::Bonus {
        restyle(pieces, Style::Bold)
    } else {
        pieces
    };
    Line::Text { kind, pieces }
}

fn restyle(pieces: Vec<Piece>, style: Style) -> Vec<Piece> {
    pieces
        .into_iter()
        .map(|p| match p {
            Piece::Text(t, _) => Piece::Text(t, style),
            p => p,
        })
        .collect()
}

fn is_huge(pieces: &[Piece]) -> bool {
    matches!(pieces, [Piece::Coin(Some(_))] | [Piece::Vp(Some(_))])
}

/// `+1 Card`, `+2 Actions`, `+$2`, `+1 VP`, `+1 Sun`, ... alone on the line.
fn is_bonus(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('+') else {
        return false;
    };
    if let Some(n) = rest.strip_prefix('$') {
        return number(n).is_some_and(|(_, len)| len == n.len());
    }
    let Some((_, len)) = number(rest) else {
        return false;
    };
    let word = rest[len..].strip_prefix(' ').unwrap_or("");
    BONUS_WORDS.contains(&word) || ["VP", "Sun", "Debt"].contains(&word)
}

/// A leading run of digits: its value and length.
fn number(s: &str) -> Option<(u32, usize)> {
    let len = s.bytes().take_while(u8::is_ascii_digit).count();
    (len > 0).then(|| (s[..len].parse().unwrap_or(u32::MAX), len))
}

/// The word starting at `s`: letters, digits and apostrophes.
fn word(s: &str) -> &str {
    let len = s
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '\''))
        .map_or(s.len(), |(i, _)| i);
    &s[..len]
}

struct Inline {
    out: Vec<Piece>,
    text: String,
    style: Style,
}

impl Inline {
    fn push_text(&mut self, s: &str, style: Style) {
        if style != self.style {
            self.flush();
            self.style = style;
        }
        self.text.push_str(s);
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.out
                .push(Piece::Text(std::mem::take(&mut self.text), self.style));
        }
    }

    fn icon(&mut self, p: Piece) {
        self.flush();
        self.out.push(p);
    }

    /// The text written so far ends a word (or nothing has been written).
    fn at_word_start(&self) -> bool {
        let last = self.text.chars().last().or_else(|| {
            self.out.last().map(|p| match p {
                Piece::Text(t, _) => t.chars().last().unwrap_or(' '),
                _ => 'x',
            })
        });
        last.is_none_or(|c| !c.is_alphanumeric())
    }

    /// The last word written, for telling "has Potion" from "a Potion".
    fn previous_word(&self) -> &str {
        self.text.trim_end().rsplit(' ').next().unwrap_or("")
    }
}

/// Splits a line into styled text and icons.
fn inline(line: &str) -> Vec<Piece> {
    let mut st = Inline {
        out: vec![],
        text: String::new(),
        style: Style::Normal,
    };
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        let start = st.at_word_start();
        if start && let Some(len) = signed_icon(&mut st, rest) {
            i += len;
            continue;
        }
        if let Some(n) = rest.strip_prefix('$') {
            let num = number(n);
            st.icon(Piece::Coin(num.map(|(v, _)| v)));
            i += 1 + num.map_or(0, |(_, l)| l);
            continue;
        }
        if start {
            if let Some((n, len)) = number(rest)
                && let Some(after) = rest[len..].strip_prefix(' ')
            {
                let w = word(after);
                let piece = match w {
                    "VP" => Some(Piece::Vp(Some(n))),
                    "Debt" => Some(Piece::Debt(Some(n))),
                    _ => None,
                };
                if let Some(p) = piece {
                    st.icon(p);
                    i += len + 1 + w.len();
                    continue;
                }
            }
            let w = word(rest);
            let piece = match w {
                "VP" => Some(Piece::Vp(None)),
                "Debt" => Some(Piece::Debt(None)),
                "Sun" => Some(Piece::Sun),
                "Potion" if !["a", "an", "the", "A", "The"].contains(&st.previous_word()) => {
                    Some(Piece::Potion)
                }
                _ => None,
            };
            if let Some(p) = piece {
                st.icon(p);
                i += w.len();
                continue;
            }
        }
        let c = rest.chars().next().unwrap_or(' ');
        st.push_text(&rest[..c.len_utf8()], Style::Normal);
        i += c.len_utf8();
    }
    st.flush();
    st.out
}

/// `+N Card(s)` and the like in bold, `+$N`, `+N VP`, `+N Debt`, `+Debt`, `+N Sun` and
/// `-$N` as a bold sign and an icon. Returns the length consumed.
fn signed_icon(st: &mut Inline, rest: &str) -> Option<usize> {
    let sign = rest.chars().next().filter(|c| *c == '+' || *c == '-')?;
    let after = &rest[1..];
    if let Some(n) = after.strip_prefix('$') {
        let num = number(n);
        st.push_text(&sign.to_string(), Style::Bold);
        st.icon(Piece::Coin(num.map(|(v, _)| v)));
        return Some(2 + num.map_or(0, |(_, l)| l));
    }
    if sign == '+' && word(after) == "Debt" {
        st.push_text("+", Style::Bold);
        st.icon(Piece::Debt(None));
        return Some(1 + "Debt".len());
    }
    let (n, len) = number(after)?;
    let tail = after[len..].strip_prefix(' ')?;
    let w = word(tail);
    let used = 1 + len + 1 + w.len();
    let icon = match w {
        "VP" => Some(Piece::Vp(Some(n))),
        "Debt" => Some(Piece::Debt(Some(n))),
        _ => None,
    };
    if let Some(icon) = icon {
        st.push_text(&sign.to_string(), Style::Bold);
        st.icon(icon);
        return Some(used);
    }
    if sign == '+' && w == "Sun" {
        st.push_text(&format!("+{n} "), Style::Bold);
        st.icon(Piece::Sun);
        return Some(used);
    }
    if sign == '+' && BONUS_WORDS.contains(&w) {
        st.push_text(&rest[..used], Style::Bold);
        return Some(used);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use Piece::*;
    use Style::*;

    fn t(s: &str) -> Piece {
        Text(s.into(), Normal)
    }
    fn b(s: &str) -> Piece {
        Text(s.into(), Bold)
    }
    fn i(s: &str) -> Piece {
        Text(s.into(), Italic)
    }
    fn line(kind: LineKind, pieces: Vec<Piece>) -> Line {
        Line::Text { kind, pieces }
    }

    fn all_cards() -> Vec<(String, String)> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/cards");
        let mut cards = vec![];
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let list: Vec<serde_json::Value> =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for c in list {
                cards.push((
                    c["name"].as_str().unwrap().to_string(),
                    c["text"].as_str().unwrap().to_string(),
                ));
            }
        }
        assert!(cards.len() > 900);
        cards
    }

    /// The source text a piece stands for.
    fn source(p: &Piece) -> String {
        let n = |v: &Option<u32>| v.map(|v| v.to_string());
        match p {
            Text(s, _) => s.clone(),
            Coin(v) => format!("${}", n(v).unwrap_or_default()),
            Debt(v) => n(v).map_or("Debt".into(), |v| format!("{v} Debt")),
            Vp(v) => n(v).map_or("VP".into(), |v| format!("{v} VP")),
            Potion => "Potion".into(),
            Sun => "Sun".into(),
        }
    }

    #[test]
    fn every_card_text_parses_into_pieces_that_rebuild_it() {
        for (name, text) in all_cards() {
            let lines = parse(&text);
            let source_lines: Vec<&str> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            assert_eq!(lines.len(), source_lines.len(), "{name}");
            for (l, src) in lines.iter().zip(source_lines) {
                let rebuilt = match l {
                    Line::Separator => "---".to_string(),
                    Line::Text { pieces, .. } => pieces.iter().map(source).collect(),
                };
                assert_eq!(rebuilt, src, "{name}");
                let Line::Text { pieces, .. } = l else {
                    continue;
                };
                for p in pieces {
                    if let Text(s, _) = p {
                        assert!(!s.contains('$'), "{name}: stray $ in {s:?}");
                        for w in s.split(|c: char| !c.is_alphanumeric()) {
                            assert!(
                                !["VP", "Debt", "Sun"].contains(&w),
                                "{name}: {w} left as text in {s:?}"
                            );
                        }
                        assert!(!s.contains("---"), "{name}");
                    }
                }
            }
        }
    }

    #[test]
    fn representative_cards() {
        use LineKind::*;
        let cases: Vec<(&str, Vec<Line>)> = vec![
            (
                "+1 Card\n+2 Actions",
                vec![
                    line(Bonus, vec![b("+1 Card")]),
                    line(Bonus, vec![b("+2 Actions")]),
                ],
            ),
            (
                "+$2\nEach other player discards down to 3 cards in hand.",
                vec![
                    line(Bonus, vec![b("+"), Coin(Some(2))]),
                    line(
                        Normal,
                        vec![t("Each other player discards down to 3 cards in hand.")],
                    ),
                ],
            ),
            (
                "$2\n---\n2 VP",
                vec![
                    line(Huge, vec![Coin(Some(2))]),
                    Line::Separator,
                    line(Huge, vec![Vp(Some(2))]),
                ],
            ),
            (
                "Worth 1 VP per 10 cards you have (round down).",
                vec![line(
                    Normal,
                    vec![
                        t("Worth "),
                        Vp(Some(1)),
                        t(" per 10 cards you have (round down)."),
                    ],
                )],
            ),
            (
                "+1 Card\n+1 Action\nDiscard the top Boon. You may trash this to receive that Boon twice.\n(Heirloom: Goat)",
                vec![
                    line(Bonus, vec![b("+1 Card")]),
                    line(Bonus, vec![b("+1 Action")]),
                    line(
                        Normal,
                        vec![t(
                            "Discard the top Boon. You may trash this to receive that Boon twice.",
                        )],
                    ),
                    line(Heirloom, vec![i("(Heirloom: Goat)")]),
                ],
            ),
            (
                "Gain a card costing up to $4. You may trash this.",
                vec![line(
                    Normal,
                    vec![
                        t("Gain a card costing up to "),
                        Coin(Some(4)),
                        t(". You may trash this."),
                    ],
                )],
            ),
            (
                "Any cards or Debt they would gain",
                vec![line(
                    Normal,
                    vec![t("Any cards or "), Debt(None), t(" they would gain")],
                )],
            ),
            (
                "When you discard this from play, +6 Debt.",
                vec![line(
                    Normal,
                    vec![
                        t("When you discard this from play, "),
                        b("+"),
                        Debt(Some(6)),
                        t("."),
                    ],
                )],
            ),
            (
                "+2 Cards if it has Potion in its cost. Put the Coppers and Potions and a Potion into your hand.",
                vec![line(
                    Normal,
                    vec![
                        b("+2 Cards"),
                        t(" if it has "),
                        Potion,
                        t(" in its cost. Put the Coppers and Potions and a Potion into your hand."),
                    ],
                )],
            ),
            (
                "Choose one: +3 Cards; or +2 Actions.",
                vec![line(
                    Normal,
                    vec![
                        t("Choose one: "),
                        b("+3 Cards"),
                        t("; or "),
                        b("+2 Actions"),
                        t("."),
                    ],
                )],
            ),
            (
                "gain a card costing more $ than it. +Debt equal to the difference in $.",
                vec![line(
                    Normal,
                    vec![
                        t("gain a card costing more "),
                        Coin(None),
                        t(" than it. "),
                        b("+"),
                        Debt(None),
                        t(" equal to the difference in "),
                        Coin(None),
                        t("."),
                    ],
                )],
            ),
            (
                "+1 Sun\nWhen you remove the last Sun, take the VP from this.\n(This is not in the Supply.)",
                vec![
                    line(Bonus, vec![b("+1 "), Sun]),
                    line(
                        Normal,
                        vec![
                            t("When you remove the last "),
                            Sun,
                            t(", take the "),
                            Vp(None),
                            t(" from this."),
                        ],
                    ),
                    line(Italic, vec![i("(This is not in the Supply.)")]),
                ],
            ),
            (
                "Take your -$1 token. Each 2 VP.",
                vec![line(
                    Normal,
                    vec![
                        t("Take your "),
                        b("-"),
                        Coin(Some(1)),
                        t(" token. Each "),
                        Vp(Some(2)),
                        t("."),
                    ],
                )],
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(parse(text), expected, "{text}");
        }
    }
}

//! The plain game page: everything in the view as structured text, the log, and the
//! open decision as generic controls ([`DecisionControls`], usable beside any board).

use super::state::{Selection, answers_on_click, name_answer};
use super::use_game;
use super::wording;
use crate::app::use_cards;
use dioxus::prelude::*;
use dominion_protocol::{
    AgentDecision, AgentOption, AgentView, Answer, CardCount, Cost, LogEntry, PileView, PlayerView,
    Tokens,
};

pub fn cost_text(c: &Cost) -> String {
    let mut s = format!("${}", c.coins);
    if c.potions > 0 {
        s.push_str(&format!(" {}P", c.potions));
    }
    if c.debt > 0 {
        s.push_str(&format!(" {}D", c.debt));
    }
    s
}

fn player_name(view: &AgentView, seat: u8) -> String {
    view.you
        .iter()
        .chain(&view.opponents)
        .find(|p| p.index == seat)
        .map_or_else(|| format!("P{}", seat + 1), |p| p.name.clone())
}

/// The plain page for the seat [`super::GamePage`] shows.
#[component]
pub fn PlainGame() -> Element {
    let ctx = use_game();
    let st = ctx.state.read();
    let Some(view) = st.view.clone() else {
        return rsx! {
            if let Some(e) = st.error.clone() {
                p { class: "error", "{e}" }
            } else {
                p { class: "muted", "Loading the game…" }
            }
        };
    };
    let deciding = (*ctx.deciding.read()).map(|d| player_name(&view, d));
    let error = st.error.clone();
    let answering = st.answering;
    let log = st.log.clone();
    drop(st);
    rsx! {
        div { class: "plain-game",
            div { class: "board-col",
                TurnLine { view: view.clone() }
                if let Some(r) = view.game.result.clone() {
                    section { class: "panel result",
                        h2 { "Game over" }
                        if let Some(reason) = r.reason.clone() {
                            p { class: "muted", "{reason}" }
                        }
                        table {
                            thead { tr { th { "Rank" } th { "Player" } th { "VP" } th { "Turns" } } }
                            tbody {
                                for p in r.players.iter() {
                                    tr { key: "{p.index}",
                                        td { "{p.rank}" }
                                        td { "{p.name}" if p.resigned { " (resigned)" } }
                                        td { "{p.vp}" }
                                        td { "{p.turns}" }
                                    }
                                }
                            }
                        }
                    }
                }
                PlainBoard { view: view.clone() }
            }
            div { class: "side-col",
                section { class: "panel decision-panel",
                    if let Some(e) = error {
                        p { class: "error", "{e}" }
                    }
                    if let Some(d) = view.decision.clone() {
                        DecisionControls {
                            key: "{d.id}",
                            decision: d,
                            busy: answering,
                            on_answer: move |a| ctx.submit.call(a),
                        }
                    } else if view.game.running {
                        p { class: "waiting",
                            match deciding {
                                Some(name) => rsx! { "Waiting for {name} to decide…" },
                                None => rsx! { "Waiting…" },
                            }
                        }
                    } else {
                        p { class: "muted", "The game is over." }
                    }
                }
                LogPanel { entries: log }
            }
        }
    }
}

#[component]
fn TurnLine(view: AgentView) -> Element {
    let t = &view.turn;
    let mut parts = vec![];
    if let Some(n) = t.number {
        parts.push(format!("Turn {n}"));
    }
    if let Some(active) = &t.active_player {
        parts.push(if t.your_turn {
            format!("{active}'s turn (you)")
        } else {
            format!("{active}'s turn")
        });
    }
    if let Some(extra) = &t.extra {
        parts.push(format!("extra turn from {extra}"));
    }
    if let Some(phase) = &t.phase {
        parts.push(format!("{phase} phase"));
    }
    let mut stats = vec![];
    for (name, value) in [("Actions", t.actions), ("Buys", t.buys)] {
        if let Some(n) = value {
            stats.push(format!("{name} {n}"));
        }
    }
    if let Some(n) = t.coins {
        stats.push(format!("${n}"));
    }
    for (name, value) in [("Potions", t.potions), ("Debt", t.debt)] {
        if let Some(n) = value.filter(|&n| n > 0) {
            stats.push(format!("{name} {n}"));
        }
    }
    rsx! {
        div { class: if t.your_turn { "turn-line yours" } else { "turn-line" },
            span { class: "turn-what", "{parts.join(\" · \")}" }
            if !stats.is_empty() {
                span { class: "turn-stats", "{stats.join(\" · \")}" }
            }
            if !view.game.running {
                span { class: "over", "game over" }
            }
        }
    }
}

/// A card name showing the card on hover.
#[component]
pub fn CardName(name: String, count: Option<u32>) -> Element {
    let label = match count {
        Some(n) if n > 1 => format!("{n}× {name}"),
        _ => name.clone(),
    };
    rsx! {
        crate::card::CardHover { name: name.clone(),
            span { class: "card-name", "{label}" }
        }
    }
}

#[component]
fn Cards(known: CardCount, unknown: u32, count: u32) -> Element {
    let partly_known = unknown > 0 && !known.is_empty();
    rsx! {
        span { class: "cards",
            span { class: "count", "{count}" }
            for (name, n) in known.into_iter() {
                CardName { key: "{name}", name: name.clone(), count: n }
            }
            if partly_known {
                span { class: "unknown", "{unknown} unknown" }
            }
        }
    }
}

#[component]
fn ZoneRow(label: String, known: CardCount, unknown: u32, count: u32) -> Element {
    rsx! {
        tr {
            th { "{label}" }
            td { Cards { known, unknown, count } }
        }
    }
}

fn token_list(t: &Tokens) -> Vec<String> {
    let mut out: Vec<String> = [
        ("Coffers", t.coffers as i64),
        ("Villagers", t.villagers as i64),
        ("Favors", t.favors as i64),
        ("Debt", t.debt as i64),
        ("VP tokens", t.vp_tokens as i64),
        ("Pirate Ship", t.pirate_ship as i64),
    ]
    .into_iter()
    .filter(|(_, n)| *n != 0)
    .map(|(name, n)| format!("{name} {n}"))
    .collect();
    for (flag, name) in [
        (t.minus_card, "-1 Card token"),
        (t.minus_coin, "-$1 token"),
        (t.journey_face_up == Some(true), "Journey token face up"),
        (t.journey_face_up == Some(false), "Journey token face down"),
    ] {
        if flag {
            out.push(name.into());
        }
    }
    out
}

#[component]
fn PlayerBox(player: PlayerView, you: bool) -> Element {
    let p = &player;
    let mut extras = token_list(&p.tokens);
    for (token, pile) in &p.adventure_tokens {
        extras.push(match pile {
            Some(pile) => format!("{token} token on {pile}"),
            None => format!("{token} token"),
        });
    }
    for (name, n) in &p.counters {
        extras.push(format!("{name} {n}"));
    }
    if let Some(n) = p.turns_taken {
        extras.push(format!("{n} turns taken"));
    }
    let lists = [
        ("Projects", p.projects.clone()),
        ("States", p.states.clone()),
        ("Boons", p.boons.clone()),
        ("Artifacts", p.artifacts.clone()),
    ];
    rsx! {
        section { class: if you { "panel player you" } else { "panel player" },
            h3 {
                "{p.name}"
                if you { span { class: "muted", " (you)" } }
                if let Some(vp) = p.vp { span { class: "vp", "{vp} VP" } }
            }
            table { class: "zones",
                tbody {
                    ZoneRow { label: "Hand", known: p.hand.known.clone(), unknown: p.hand.unknown, count: p.hand.count }
                    if p.in_play.count > 0 {
                        ZoneRow { label: "In play", known: p.in_play.known.clone(), unknown: p.in_play.unknown, count: p.in_play.count }
                    }
                    tr {
                        th { "Deck" }
                        td {
                            Cards { known: p.deck.known.clone(), unknown: p.deck.unknown, count: p.deck.count }
                            if p.deck.order_known && p.deck.count > 0 { span { class: "muted", " (order known)" } }
                        }
                    }
                    ZoneRow { label: "Discard", known: p.discard.known.clone(), unknown: p.discard.unknown, count: p.discard.count }
                    for (name, z) in p.other_zones.iter() {
                        ZoneRow { key: "{name}", label: name.clone(), known: z.known.clone(), unknown: z.unknown, count: z.count }
                    }
                    for (name, list) in lists {
                        if !list.is_empty() {
                            tr { key: "{name}",
                                th { "{name}" }
                                td {
                                    for c in list {
                                        CardName { key: "{c}", name: c.clone() }
                                    }
                                }
                            }
                        }
                    }
                    if !p.unlocated.is_empty() {
                        tr {
                            th { "Unlocated" }
                            td { Cards { known: p.unlocated.clone(), unknown: 0, count: p.unlocated.values().sum::<u32>() } }
                        }
                    }
                    tr {
                        th { "Owns" }
                        td {
                            Cards {
                                known: p.owned.known.clone(),
                                unknown: p.owned.unknown,
                                count: p.owned.known.values().sum::<u32>() + p.owned.unknown,
                            }
                        }
                    }
                }
            }
            for u in p.uncertain.iter() {
                p { class: "uncertain",
                    "{u.zones.join(\" + \")} hold {u.count} of: "
                    span { class: "cards",
                        for (name, n) in u.candidates.iter() {
                            CardName { key: "{name}", name: name.clone(), count: *n }
                        }
                    }
                    if u.outside > 0 { " ({u.outside} of them elsewhere)" }
                }
            }
            if !extras.is_empty() {
                p { class: "extras", "{extras.join(\" · \")}" }
            }
        }
    }
}

fn pile_tokens(p: &PileView) -> Vec<String> {
    let t = &p.tokens;
    let mut out = vec![];
    if t.embargo > 0 {
        out.push(format!("{} Embargo", t.embargo));
    }
    if t.trade_route {
        out.push("Trade Route".into());
    }
    if t.vp != 0 {
        out.push(format!("{} VP", t.vp));
    }
    if t.debt != 0 {
        out.push(format!("{} Debt", t.debt));
    }
    for pt in &t.players {
        out.push(format!("P{}'s {}", pt.player + 1, pt.token));
    }
    out.extend(p.traits.iter().cloned());
    out
}

#[component]
fn PileTable(title: String, piles: Vec<PileView>) -> Element {
    rsx! {
        section { class: "panel piles",
            h3 { "{title}" }
            table {
                thead { tr { th { "Pile" } th { "Cost" } th { "Left" } th { "Notes" } } }
                tbody {
                    for p in piles.iter() {
                        tr { key: "{p.card}", class: if p.count == 0 { "empty" },
                            td {
                                CardName { name: p.card.clone() }
                                if let Some(top) = p.top.clone().filter(|t| *t != p.card) {
                                    span { class: "muted", " top: " }
                                    CardName { name: top }
                                }
                            }
                            td { {p.cost.as_ref().map(cost_text).unwrap_or_default()} }
                            td { "{p.count}" }
                            td { class: "muted",
                                {
                                    let mut notes = pile_tokens(p);
                                    if p.hidden {
                                        notes.insert(0, "face down".into());
                                    }
                                    notes.join(", ")
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Every part of a view except the decision and the log.
#[component]
pub fn PlainBoard(view: AgentView) -> Element {
    let v = &view;
    let name = |p: u8| player_name(v, p);
    rsx! {
        div { class: "players",
            if let Some(you) = v.you.clone() {
                PlayerBox { player: you, you: true }
            }
            for o in v.opponents.iter().cloned() {
                PlayerBox { key: "{o.index}", player: o, you: false }
            }
        }
        PileTable { title: "Supply", piles: v.supply.clone() }
        if !v.non_supply.is_empty() {
            PileTable { title: "Non-Supply", piles: v.non_supply.clone() }
        }
        if !v.landscapes.is_empty() {
            section { class: "panel",
                h3 { "Landscapes" }
                ul { class: "plain-list",
                    for l in v.landscapes.iter() {
                        li { key: "{l.card}",
                            CardName { name: l.card.clone() }
                            span { class: "muted", " {l.kind}" }
                            if let Some(c) = &l.cost { " {cost_text(c)}" }
                            if let Some(vp) = l.vp { " · {vp} VP" }
                            if let Some(s) = l.suns { " · {s} Sun" }
                            if l.active == Some(true) { " · active" }
                            if !l.owners.is_empty() {
                                " · owned by {l.owners.iter().map(|&p| name(p)).collect::<Vec<_>>().join(\", \")}"
                            }
                        }
                    }
                }
            }
        }
        section { class: "panel",
            h3 { "Trash" }
            Cards { known: v.trash.known.clone(), unknown: v.trash.unknown, count: v.trash.count }
        }
        if !v.uncertain.is_empty() {
            section { class: "panel",
                h3 { "Uncertain" }
                for (i, u) in v.uncertain.iter().enumerate() {
                    p { key: "{i}",
                        {
                            u.zones
                                .iter()
                                .map(|z| match (z.player, &z.pile) {
                                    (Some(p), _) => format!("{} {}", name(p), z.zone),
                                    (None, Some(pile)) => format!("{pile} pile"),
                                    _ => z.zone.clone(),
                                })
                                .collect::<Vec<_>>()
                                .join(" + ")
                        }
                        " hold {u.count} of: "
                        span { class: "cards",
                            for (c, n) in u.candidates.iter() {
                                CardName { key: "{c}", name: c.clone(), count: *n }
                            }
                        }
                    }
                }
            }
        }
        for (label, deck) in [("Boons", v.boons.clone()), ("Hexes", v.hexes.clone())] {
            if let Some(d) = deck {
                section { key: "{label}", class: "panel",
                    h3 { "{label}" }
                    p {
                        "{d.count} to draw"
                        if !d.discard.is_empty() { "; discarded: " }
                        for (i, c) in d.discard.iter().enumerate() {
                            CardName { key: "{i}", name: c.clone() }
                        }
                    }
                }
            }
        }
        if !v.artifacts.is_empty() {
            section { class: "panel",
                h3 { "Artifacts" }
                ul { class: "plain-list",
                    for a in v.artifacts.iter() {
                        li { key: "{a.card}",
                            CardName { name: a.card.clone() }
                            match a.holder {
                                Some(p) => rsx! { " — {name(p)}" },
                                None => rsx! { " — unclaimed" },
                            }
                        }
                    }
                }
            }
        }
        if !v.lasting.is_empty() {
            section { class: "panel",
                h3 { "Lasting effects" }
                ul { class: "plain-list",
                    for (i, l) in v.lasting.iter().enumerate() {
                        li { key: "{i}",
                            CardName { name: l.card.clone() }
                            " ({name(l.owner)}) "
                            {
                                match (l.until.as_str(), l.until_player) {
                                    ("start-of-turn", Some(p)) => format!("until {}'s next turn", name(p)),
                                    ("end-of-turn", _) => "this turn".into(),
                                    _ => "until removed".into(),
                                }
                            }
                            if !l.cards.is_empty() { ": {l.cards.join(\", \")}" }
                            if let Some(n) = l.count { " [{n}]" }
                        }
                    }
                }
            }
        }
        if !v.extra_turns.is_empty() {
            section { class: "panel",
                h3 { "Extra turns" }
                p {
                    {v.extra_turns.iter().map(|e| format!("{} ({})", name(e.player), e.source)).collect::<Vec<_>>().join(", ")}
                }
            }
        }
        if !v.notes.is_empty() {
            section { class: "panel",
                h3 { "Notes" }
                ul { class: "plain-list",
                    for (i, n) in v.notes.iter().enumerate() {
                        li { key: "{i}",
                            CardName { name: n.source.clone() }
                            {
                                let mut parts = vec![];
                                if let Some(p) = n.player {
                                    parts.push(name(p));
                                }
                                if let Some(pile) = &n.pile {
                                    parts.push(format!("{pile} pile"));
                                }
                                if let Some(card) = &n.card {
                                    parts.push(card.clone());
                                }
                                if n.count != 0 {
                                    parts.push(n.count.to_string());
                                }
                                if parts.is_empty() { String::new() } else { format!(": {}", parts.join(" ")) }
                            }
                        }
                    }
                }
            }
        }
        if !v.counters.is_empty() {
            section { class: "panel",
                h3 { "Counters" }
                p { {v.counters.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(" · ")} }
            }
        }
    }
}

/// The log, newest at the bottom, kept scrolled to the end.
#[component]
pub fn LogPanel(entries: Vec<LogEntry>) -> Element {
    let len = entries.len();
    use_effect(use_reactive!(|len| {
        let _ = len;
        document::eval(
            "const e = document.getElementById('game-log'); if (e) e.scrollTop = e.scrollHeight;",
        );
    }));
    rsx! {
        section { class: "panel log-panel",
            h3 { "Log" }
            ol { id: "game-log", class: "log",
                for e in entries.iter() {
                    li { key: "{e.seq}", class: "log-{e.kind}", "data-seq": "{e.seq}", "{e.text}" }
                }
            }
        }
    }
}

fn option_label(o: &AgentOption) -> String {
    match o.zone.as_deref() {
        Some(z) if o.action == "select" && !matches!(z, "hand" | "looking") => {
            format!("{} (from {z})", o.label)
        }
        _ => o.label.clone(),
    }
}

fn bounds_text(min: u32, max: u32) -> String {
    if min == max {
        format!("choose {min}")
    } else {
        format!("choose {min}–{max}")
    }
}

/// An open decision as generic controls: its prompt, a button per option, and for
/// decisions of more than one pick a Confirm enabled only when the selection is legal.
/// Remount it (key it by decision id) for each new decision.
#[component]
pub fn DecisionControls(
    decision: AgentDecision,
    busy: bool,
    on_answer: EventHandler<Answer>,
) -> Element {
    let cards = use_cards();
    let mut selection = use_signal(Selection::default);
    let mut name = use_signal(String::new);
    let d = decision.clone();
    let click_answers = answers_on_click(&d);
    let sel = selection();
    let legal = sel.answer(&d);
    let many_groups = d.groups.len() > 1;
    let named = name_answer(&d, &cards, &name());
    let label_of = |i: u32| {
        d.options
            .iter()
            .find(|o| o.index == i)
            .map(option_label)
            .unwrap_or_default()
    };
    rsx! {
        div { class: "decision",
            h3 { class: "prompt", "{wording::own_prompt(&d)}" }
            for g in d.groups.iter().cloned() {
                div { key: "{g.group}", class: "group",
                    if many_groups {
                        h4 { "{wording::without_way_hint(&g.prompt)} ({bounds_text(g.min, g.max)})" }
                    } else if !click_answers {
                        p { class: "muted", "{bounds_text(g.min, g.max)}" if d.ordered { " — order matters, first picked first" } }
                    }
                    div { class: "options",
                        for o in d.options.iter().filter(|o| o.group == g.group).cloned() {
                            {
                                let d = d.clone();
                                let pos = sel.position(o.index);
                                let title = o.card.as_deref().map(|c| cards.tooltip(c)).unwrap_or_default();
                                rsx! {
                                    button {
                                        key: "{o.index}",
                                        class: if pos.is_some() { "option {o.action} picked" } else { "option {o.action}" },
                                        title,
                                        disabled: busy,
                                        "data-index": "{o.index}",
                                        onclick: move |_| {
                                            if click_answers {
                                                if let Some(a) = Selection::new(vec![o.index]).answer(&d) {
                                                    on_answer.call(a);
                                                }
                                            } else {
                                                selection.write().toggle(&d, o.index);
                                            }
                                        },
                                        if let (Some(p), true) = (pos, d.ordered) {
                                            span { class: "order", "{p + 1}" }
                                        }
                                        "{option_label(&o)}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if d.ordered && !sel.picks().is_empty() {
                ol { class: "order-list",
                    for (i, &p) in sel.picks().iter().enumerate() {
                        li { key: "{p}",
                            span { "{label_of(p)}" }
                            button {
                                class: "small",
                                disabled: i == 0,
                                onclick: move |_| selection.write().reorder(i, i - 1),
                                "↑"
                            }
                            button {
                                class: "small",
                                disabled: i + 1 == sel.picks().len(),
                                onclick: move |_| selection.write().reorder(i, i + 1),
                                "↓"
                            }
                        }
                    }
                }
            }
            if !click_answers {
                div { class: "actions",
                    button {
                        class: "primary confirm",
                        disabled: busy || legal.is_none(),
                        onclick: move |_| {
                            if let Some(a) = legal.clone() {
                                on_answer.call(a);
                            }
                        },
                        "Confirm"
                        if !sel.picks().is_empty() { " ({sel.picks().len()})" }
                    }
                    if !sel.picks().is_empty() {
                        button { onclick: move |_| selection.write().clear(), "Clear" }
                    }
                }
            }
            if d.name_any {
                div { class: "name-any",
                    label { "Or name any card:" }
                    div { class: "row",
                        input {
                            list: "nameable-cards",
                            value: "{name}",
                            placeholder: "card name",
                            oninput: move |e| name.set(e.value()),
                        }
                        button {
                            class: "primary",
                            disabled: busy || named.is_none(),
                            onclick: move |_| {
                                if let Some(a) = named.clone() {
                                    on_answer.call(a);
                                }
                            },
                            "Name it"
                        }
                    }
                    datalist { id: "nameable-cards",
                        for c in cards.nameable() {
                            option { key: "{c}", value: "{c}" }
                        }
                    }
                }
            }
        }
    }
}

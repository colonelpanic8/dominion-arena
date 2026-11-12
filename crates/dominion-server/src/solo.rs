//! Solo games: one player at a table whose other seats sit idle, kept in a file and
//! advanced by named steps. For playing a deck's build out to see how fast it is.

use crate::hub::{self, NewGame};
use dominion_agent::{AgentDecision, AgentOption, AgentView, Answer, Table, text};
use dominion_engine::PlayerId;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// The seat the steps play.
const SEAT: PlayerId = PlayerId(0);

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    table: Table,
    log: usize,
}

/// A solo game as saved: the table, everything the player has been shown of its log, and
/// the state before the last batch of steps.
#[derive(Clone, Serialize, Deserialize)]
pub struct Solo {
    table: Table,
    seed: u64,
    log: Vec<String>,
    undo: Option<Box<Snapshot>>,
}

/// What a batch of steps did.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Log lines the batch added.
    pub log: Vec<String>,
    /// The step that couldn't be done, and why. The steps before it stand.
    pub stopped: Option<(String, String)>,
}

fn key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A name with an optional count: `Copper x3`.
fn counted(item: &str) -> (&str, usize) {
    if let Some((name, n)) = item.rsplit_once(" x")
        && let Ok(n) = n.trim().parse::<usize>()
    {
        return (name.trim(), n);
    }
    (item, 1)
}

fn items(rest: &str) -> Vec<String> {
    rest.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .flat_map(|item| {
            // `Lab x2 as Way of the Ox`: the count belongs to the card.
            let (card, way) = match item.split_once(" as ") {
                Some((card, way)) => (card.trim(), format!(" as {}", way.trim())),
                None => (item, String::new()),
            };
            let (name, n) = counted(card);
            std::iter::repeat_n(format!("{name}{way}"), n)
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// A card, and the Way to play it as.
    Play(String, Option<String>),
    Buy(String),
    Treasures,
    /// Pay off all the Debt the turn's $ covers.
    Pay,
    /// A token move (`spend-coffers`, `spend-villager`), this many times.
    Spend(&'static str, usize),
    End,
    Next,
    Pick(Vec<String>),
    Name(String),
}

fn parse(steps: &str) -> Result<Vec<(String, Step)>, String> {
    let mut out = vec![];
    for raw in steps.split([';', '\n']) {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (verb, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
        let rest = rest.trim();
        let each = |make: fn(String) -> Step| -> Result<Vec<Step>, String> {
            let names = items(rest);
            if names.is_empty() {
                return Err(format!("{raw:?} names no card"));
            }
            Ok(names.into_iter().map(make).collect())
        };
        let parsed = match verb.to_lowercase().as_str() {
            "play" => each(|item| match item.split_once(" as ") {
                Some((card, way)) => Step::Play(card.trim().into(), Some(way.trim().into())),
                None => Step::Play(item, None),
            })?,
            "buy" => each(Step::Buy)?,
            "treasures" if rest.is_empty() => vec![Step::Treasures],
            "pay" if rest.is_empty() => vec![Step::Pay],
            "coffers" | "villagers" => {
                let n = match rest {
                    "" => 1,
                    n => n
                        .parse()
                        .map_err(|_| format!("{raw:?}: {n:?} is not a number"))?,
                };
                let action = match verb.to_lowercase().as_str() {
                    "coffers" => "spend-coffers",
                    _ => "spend-villager",
                };
                vec![Step::Spend(action, n)]
            }
            "end" if rest.is_empty() => vec![Step::End],
            "next" if rest.is_empty() => vec![Step::Next],
            "pick" => vec![Step::Pick(match key(rest).as_str() {
                "" | "none" | "nothing" => vec![],
                _ => items(rest),
            })],
            "name" if !rest.is_empty() => vec![Step::Name(rest.to_string())],
            _ => {
                return Err(format!(
                    "{raw:?} is not a step; the steps are play, buy, treasures, pay, coffers, \
                     villagers, end, next, pick and name"
                ));
            }
        };
        out.extend(parsed.into_iter().map(|s| (raw.to_string(), s)));
    }
    if out.is_empty() {
        return Err("no steps given".into());
    }
    Ok(out)
}

fn names_card(o: &AgentOption, name: &str) -> bool {
    o.card.as_deref().is_some_and(|c| key(c) == key(name))
}

/// The option `name` means among those `allowed`, skipping the ones in `taken`: by index,
/// by card, by its whole label, then by the one label that starts with or contains it.
fn option(
    d: &AgentDecision,
    name: &str,
    taken: &[u32],
    allowed: impl Fn(&AgentOption) -> bool,
) -> Result<u32, String> {
    let free: Vec<&AgentOption> = d
        .options
        .iter()
        .filter(|o| allowed(o) && !taken.contains(&o.index))
        .collect();
    if let Ok(i) = name.parse::<u32>() {
        return free
            .iter()
            .find(|o| o.index == i)
            .map(|o| o.index)
            .ok_or_else(|| format!("option {i} can't be chosen"));
    }
    let wanted = key(name);
    let exact = free
        .iter()
        .find(|o| names_card(o, name) || key(&o.label) == wanted);
    if let Some(o) = exact {
        return Ok(o.index);
    }
    for partial in [
        |label: &str, wanted: &str| label.starts_with(wanted),
        |label: &str, wanted: &str| label.contains(wanted),
    ] {
        let hits: Vec<&&AgentOption> = free
            .iter()
            .filter(|o| partial(&key(&o.label), &wanted))
            .collect();
        match hits.as_slice() {
            [] => {}
            [o] => return Ok(o.index),
            _ => {
                let labels: Vec<&str> = hits.iter().map(|o| o.label.as_str()).collect();
                return Err(format!(
                    "{name:?} fits several options: {}",
                    labels.join("; ")
                ));
            }
        }
    }
    Err(format!("no option is {name:?}"))
}

/// Whether `d` asks if a card just played is played as itself or as a Way.
fn asks_way(d: &AgentDecision) -> bool {
    let ways = |o: &AgentOption| {
        o.action == "select" && o.card.as_deref().is_some_and(|c| c.starts_with("Way of "))
    };
    d.purpose == "mode"
        && d.options.first().is_some_and(|o| o.action == "no")
        && d.options.len() > 1
        && d.options[1..].iter().all(ways)
}

fn end_phase(d: &AgentDecision) -> Option<u32> {
    d.options
        .iter()
        .find(|o| o.action == "end-phase")
        .map(|o| o.index)
}

/// The least an idle seat can do: end its phases, decline what it may, and otherwise take
/// the first options offered.
fn idle_answer(d: &AgentDecision) -> Answer {
    if let Some(i) = end_phase(d) {
        return Answer::indices(&d.id, vec![i]);
    }
    let min = d.min.unwrap_or(1);
    if min == 0 {
        return Answer::indices(&d.id, vec![]);
    }
    let refuse = d
        .options
        .iter()
        .find(|o| matches!(o.action.as_str(), "decline" | "no"));
    match refuse {
        Some(o) if min == 1 => Answer::indices(&d.id, vec![o.index]),
        _ => Answer::indices(
            &d.id,
            d.options
                .iter()
                .take(min as usize)
                .map(|o| o.index)
                .collect(),
        ),
    }
}

impl Solo {
    /// Starts a game from `req`, whose first seat plays and whose other seats sit idle.
    pub fn new(name: &str, req: &NewGame) -> Result<Solo, String> {
        let config = hub::config(req).map_err(|e| e.to_string())?;
        let seed = config.seed;
        let names = (0..req.players)
            .map(|i| match i {
                0 => "Solo".to_string(),
                1 => "Idle".to_string(),
                i => format!("Idle {i}"),
            })
            .collect();
        let table = Table::new(name, config, names).map_err(|e| e.to_string())?;
        let mut solo = Solo {
            table,
            seed,
            log: vec![],
            undo: None,
        };
        solo.settle()?;
        solo.collect();
        Ok(solo)
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    /// Every log line shown so far.
    pub fn log(&self) -> &[String] {
        &self.log
    }

    fn view(&self) -> AgentView {
        self.table.view(SEAT)
    }

    fn decision(&self) -> Result<AgentDecision, String> {
        self.view()
            .decision
            .ok_or_else(|| "the game is over".to_string())
    }

    /// Answers for the idle seats until the player must decide or the game ends.
    fn settle(&mut self) -> Result<(), String> {
        while let Some(seat) = self.table.deciding().filter(|&s| s != SEAT) {
            let d = self
                .table
                .view(seat)
                .decision
                .expect("the deciding seat's decision");
            self.table.answer(seat, &idle_answer(&d)).map_err(|e| {
                format!(
                    "the idle seat {} couldn't answer {:?} on its own: {e}",
                    self.table.names()[seat.index()],
                    d.prompt
                )
            })?;
        }
        Ok(())
    }

    /// Moves what the player hasn't been shown of the log into `self.log`.
    fn collect(&mut self) {
        let view = self.view();
        self.log
            .extend(view.since_last_decision.into_iter().map(|e| e.text));
    }

    fn answer(&mut self, indices: Vec<u32>) -> Result<(), String> {
        let d = self.decision()?;
        self.respond(&Answer::indices(d.id, indices))
    }

    fn respond(&mut self, answer: &Answer) -> Result<(), String> {
        self.table.answer(SEAT, answer).map_err(|e| e.to_string())?;
        self.settle()?;
        self.collect();
        Ok(())
    }

    /// The player's turn decision, if that is what is open.
    fn turn(&self) -> Option<AgentDecision> {
        let view = self.view();
        view.decision
            .filter(|d| d.kind == "turn" && view.turn.your_turn)
    }

    /// Makes the turn move `wanted` picks out, ending the Action phase first when the
    /// move only exists in a later one. Ending a phase is kept only if the move is there.
    fn make_move(
        &mut self,
        what: &str,
        wanted: impl Fn(&AgentDecision) -> Option<u32>,
    ) -> Result<(), String> {
        let Some(d) = self.turn() else {
            let open = self.decision()?;
            return Err(format!(
                "{what} needs your turn, but this is open: {}",
                open.prompt
            ));
        };
        if let Some(i) = wanted(&d) {
            return self.answer(vec![i]);
        }
        let mut ahead = self.clone();
        let number = self.view().turn.number;
        while let Some(d) = ahead.turn().filter(|_| ahead.view().turn.number == number) {
            if let Some(i) = wanted(&d) {
                *self = ahead;
                return self.answer(vec![i]);
            }
            let Some(end) = end_phase(&d) else { break };
            ahead.answer(vec![end])?;
        }
        let turn = self.view().turn;
        Err(format!(
            "{what} is not possible now ({} phase, ${}, {} Buys, {} Actions)",
            turn.phase.unwrap_or_default(),
            turn.coins.unwrap_or(0),
            turn.buys.unwrap_or(0),
            turn.actions.unwrap_or(0)
        ))
    }

    /// `turn` is the turn the batch last saw the player in. A turn ends by itself when
    /// nothing is left to do in it, and `next` must not then end the one after.
    fn step(&mut self, step: &Step, turn: &mut Option<u32>) -> Result<(), String> {
        let now = self.view().turn;
        if matches!(step, Step::Next) && (now.number != *turn || !now.your_turn) {
            *turn = now.number;
            return Ok(());
        }
        let done = self.act(step);
        if matches!(step, Step::Next) {
            *turn = self.view().turn.number;
        }
        done
    }

    fn act(&mut self, step: &Step) -> Result<(), String> {
        match step {
            Step::Play(name, way) => {
                self.make_move(&format!("playing {name}"), |d| {
                    option(d, name, &[], |o| o.action == "play").ok()
                })?;
                let asked = self.view().decision.filter(asks_way);
                match (asked, way) {
                    (Some(d), None) => self.answer(vec![d.options[0].index]),
                    (Some(d), Some(way)) => {
                        let i = option(&d, way, &[], |o| o.action == "select")?;
                        self.answer(vec![i])
                    }
                    (None, Some(way)) => Err(format!("{name} is played, but not as {way}")),
                    (None, None) => Ok(()),
                }
            }
            Step::Buy(name) => {
                let buy = |d: &AgentDecision| {
                    option(d, name, &[], |o| {
                        matches!(o.action.as_str(), "buy" | "buy-landscape")
                    })
                    .ok()
                };
                let what = format!("buying {name}");
                let refused = match self.make_move(&what, buy) {
                    Ok(()) => return Ok(()),
                    Err(e) => e,
                };
                // Nothing can be bought with Debt unpaid.
                let mut paid = self.clone();
                if paid.act(&Step::Pay).is_ok() && paid.turn().is_some_and(|d| buy(&d).is_some()) {
                    *self = paid;
                    return self.make_move(&what, buy);
                }
                Err(refused)
            }
            Step::Pay => {
                let pay = |d: &AgentDecision| {
                    let o = d.options.iter().find(|o| o.action == "pay-debt")?;
                    Some(o.index)
                };
                self.make_move("paying off Debt", pay)?;
                while let Some(i) = self.turn().and_then(|d| pay(&d)) {
                    self.answer(vec![i])?;
                }
                Ok(())
            }
            Step::Spend(action, n) => {
                let what = match *action {
                    "spend-coffers" => "spending Coffers",
                    _ => "spending a Villager",
                };
                for _ in 0..*n {
                    self.make_move(what, |d| {
                        let o = d.options.iter().find(|o| o.action == *action)?;
                        Some(o.index)
                    })?;
                }
                Ok(())
            }
            Step::Treasures => {
                let phase = |d: &AgentDecision, p| d.phase.as_deref() == Some(p);
                let d = self.turn().ok_or("playing Treasures needs your turn")?;
                if phase(&d, "action") {
                    let end = end_phase(&d).ok_or("the Action phase can't be ended yet")?;
                    self.answer(vec![end])?;
                }
                while let Some(d) = self.turn().filter(|d| phase(d, "buy")) {
                    let Some(play) = d.options.iter().find(|o| o.action == "play") else {
                        break;
                    };
                    self.answer(vec![play.index])?;
                }
                Ok(())
            }
            Step::End => {
                let d = self.turn().ok_or("there is no phase of yours to end")?;
                let end = end_phase(&d).ok_or("this phase can't be ended yet")?;
                self.answer(vec![end])
            }
            Step::Next => {
                let d = self.turn().ok_or("there is no turn of yours to end")?;
                let turn = self.view().turn.number;
                let mut open = Some(d);
                while let Some(d) = open.filter(|_| self.view().turn.number == turn) {
                    let end = end_phase(&d).ok_or("this phase can't be ended yet")?;
                    self.answer(vec![end])?;
                    open = self.turn();
                }
                Ok(())
            }
            Step::Pick(names) => {
                let d = self.decision()?;
                let mut taken = vec![];
                for name in names {
                    taken.push(option(&d, name, &taken, |_| true)?);
                }
                self.answer(taken)
            }
            Step::Name(name) => {
                let d = self.decision()?;
                if !d.name_any {
                    return Err("this decision doesn't ask for a card's name".into());
                }
                self.respond(&Answer::name(d.id, name.clone()))
            }
        }
    }

    /// Does `steps` in order and stops at the first that can't be done.
    pub fn run(&mut self, steps: &str) -> Result<Outcome, String> {
        let steps = parse(steps)?;
        let before = Snapshot {
            table: self.table.clone(),
            log: self.log.len(),
        };
        let from = self.log.len();
        let mut stopped = None;
        let mut turn = self.view().turn.number;
        for (raw, step) in &steps {
            if let Err(e) = self.step(step, &mut turn) {
                let open = self.view().decision.map(|d| {
                    let options: Vec<String> = d
                        .options
                        .iter()
                        .map(|o| format!("{}: {}", o.index, o.label))
                        .collect();
                    format!(". Open: {} [{}]", d.prompt, options.join("; "))
                });
                stopped = Some((raw.clone(), e + &open.unwrap_or_default()));
                break;
            }
        }
        if self.table.decision_id() != before.table.decision_id() {
            self.undo = Some(Box::new(before));
        }
        Ok(Outcome {
            log: self.log[from..].to_vec(),
            stopped,
        })
    }

    /// Takes back the last batch of steps.
    pub fn undo(&mut self) -> Result<(), String> {
        let before = self.undo.take().ok_or("nothing to take back")?;
        self.table = before.table;
        self.log.truncate(before.log);
        Ok(())
    }

    /// The player's view as text, without the log lines already shown. `brief` leaves out
    /// what seldom changes between steps: the idle seats, the Supply and the landscapes.
    pub fn text(&self, brief: bool) -> String {
        let mut view = self.view();
        view.since_last_decision.clear();
        if !brief {
            return text::render(&view);
        }
        view.opponents.clear();
        let skipped = |l: &&str| ["SUPPLY ", "LANDSCAPES "].iter().any(|s| l.starts_with(s));
        let full = text::render(&view);
        let lines: Vec<&str> = full.lines().filter(|l| !skipped(l)).collect();
        lines.join("\n")
    }
}

/// The file a solo game named `name` is kept in.
pub fn path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if !plain {
        return Err(format!(
            "{name:?} can't name a solo game: use letters, digits, - and _"
        ));
    }
    Ok(dir.join(format!("{name}.json")))
}

pub fn load(path: &Path) -> Result<Solo, String> {
    let bytes = fs::read(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => format!(
            "no solo game at {}: start one with `dominion solo new`",
            path.display()
        ),
        _ => format!("{}: {e}", path.display()),
    })?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn save(path: &Path, solo: &Solo) -> Result<(), String> {
    let write = || -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let file = fs::File::create(&tmp)?;
        let mut out = BufWriter::new(&file);
        serde_json::to_writer(&mut out, solo)?;
        out.flush()?;
        drop(out);
        fs::rename(&tmp, path)
    };
    write().map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(kingdom: &[&str], seed: u64) -> Solo {
        Solo::new(
            "t",
            &NewGame {
                players: 2,
                kingdom: kingdom.iter().map(|s| s.to_string()).collect(),
                seed: Some(seed),
                colonies: Some(false),
                shelters: Some(false),
                ..NewGame::default()
            },
        )
        .unwrap()
    }

    fn owned(solo: &Solo, card: &str) -> u32 {
        let you = solo.view().you.unwrap();
        you.owned.known.get(card).copied().unwrap_or(0)
    }

    #[test]
    fn steps_are_parsed_with_counts_and_lists() {
        let steps: Vec<Step> = parse("play Village, Smithy x2; treasures\nbuy Gold; pick; next")
            .unwrap()
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        assert_eq!(
            steps,
            [
                Step::Play("Village".into(), None),
                Step::Play("Smithy".into(), None),
                Step::Play("Smithy".into(), None),
                Step::Treasures,
                Step::Buy("Gold".into()),
                Step::Pick(vec![]),
                Step::Next,
            ]
        );
        assert!(parse("dance").is_err());
        assert!(parse("buy").is_err());
        assert!(parse(" ; ").is_err());
    }

    #[test]
    fn a_money_turn_is_one_batch_and_the_idle_seat_passes() {
        let mut solo = game(&["Village", "Smithy"], 7);
        let out = solo.run("treasures; buy Silver; next").unwrap();
        assert_eq!(out.stopped, None);
        assert_eq!(owned(&solo, "Silver"), 1);
        let view = solo.view();
        assert_eq!(view.turn.number, Some(2));
        assert!(view.turn.your_turn);
        assert!(!view.opponents[0].owned.known.contains_key("Silver"));
        assert!(out.log.iter().any(|l| l.contains("buys Silver")));
        assert!(out.log.iter().any(|l| l.contains("Idle")));
    }

    #[test]
    fn next_does_not_end_a_turn_the_batch_never_played() {
        let mut solo = game(&["Village", "Smithy"], 7);
        // The only Buy is spent, so the turn ends by itself.
        solo.run("treasures; buy Silver").unwrap();
        assert_eq!(solo.view().turn.number, Some(2));
        solo.run("treasures; buy Silver; next; treasures; buy Silver; next")
            .unwrap();
        assert_eq!(owned(&solo, "Silver"), 3);
        assert_eq!(solo.view().turn.number, Some(4));
        solo.run("next").unwrap();
        assert_eq!(solo.view().turn.number, Some(5));
        assert_eq!(owned(&solo, "Silver"), 3);
    }

    #[test]
    fn a_step_that_cannot_be_done_stops_the_batch_and_keeps_the_turn() {
        let mut solo = game(&["Village", "Smithy"], 7);
        let out = solo.run("treasures; buy Province; next").unwrap();
        let (step, _) = out.stopped.unwrap();
        assert_eq!(step, "buy Province");
        let view = solo.view();
        assert_eq!(view.turn.number, Some(1));
        assert_eq!(view.turn.phase.as_deref(), Some("buy"));
    }

    #[test]
    fn a_move_that_is_not_there_does_not_end_the_phase() {
        let mut solo = game(&["Village", "Smithy"], 7);
        let open = solo.table().decision_id();
        let out = solo.run("play Smithy").unwrap();
        assert!(out.stopped.is_some());
        assert_eq!(solo.table().decision_id(), open);
        assert_eq!(solo.view().turn.number, Some(1));
    }

    #[test]
    fn pick_answers_a_card_choice_by_name() {
        let mut solo = game(&["Chapel", "Village"], 3);
        solo.run("treasures; buy Chapel; next").unwrap();
        solo.run("treasures; buy Silver; next").unwrap();
        while !solo.view().you.unwrap().hand.known.contains_key("Chapel") {
            solo.run("next").unwrap();
        }
        let estates = owned(&solo, "Estate");
        let coppers = owned(&solo, "Copper");
        let hand = solo.view().you.unwrap().hand.known;
        let in_hand = hand.get("Copper").copied().unwrap_or(0).min(2);
        let out = solo
            .run(&format!("play Chapel; pick Copper x{in_hand}"))
            .unwrap();
        assert_eq!(out.stopped, None);
        assert_eq!(owned(&solo, "Copper"), coppers - in_hand);
        assert_eq!(owned(&solo, "Estate"), estates);
    }

    #[test]
    fn a_card_is_played_as_itself_unless_a_way_is_named() {
        let way = |steps: &str| {
            let mut solo = Solo::new(
                "t",
                &NewGame {
                    players: 2,
                    kingdom: vec!["Village".into()],
                    landscapes: vec!["Way of the Sheep".into()],
                    seed: Some(5),
                    ..NewGame::default()
                },
            )
            .unwrap();
            solo.run("treasures; buy Village; next").unwrap();
            while !solo.view().you.unwrap().hand.known.contains_key("Village") {
                solo.run("next").unwrap();
            }
            assert_eq!(solo.run(steps).unwrap().stopped, None);
            solo.view().turn
        };
        let plain = way("play Village");
        assert_eq!((plain.actions, plain.coins), (Some(2), Some(0)));
        let sheep = way("play Village as Way of the Sheep");
        assert_eq!((sheep.actions, sheep.coins), (Some(0), Some(2)));
        assert_eq!(
            parse("play Village x2 as sheep").unwrap()[1].1,
            Step::Play("Village".into(), Some("sheep".into()))
        );
    }

    #[test]
    fn debt_is_paid_by_name_and_before_a_buy() {
        let mut solo = game(&["Engineer"], 7);
        assert_eq!(
            solo.run("treasures; buy Engineer; pay; next")
                .unwrap()
                .stopped,
            None
        );
        assert_eq!(owned(&solo, "Engineer"), 1);
        // The two opening hands hold $7: what the first left unpaid, the second pays.
        let out = solo.run("treasures; buy Silver; next").unwrap();
        assert_eq!(out.stopped, None);
        assert_eq!(owned(&solo, "Silver"), 1);
        assert_eq!(solo.view().turn.debt.unwrap_or(0), 0);
        let (_, why) = solo.run("pay").unwrap().stopped.unwrap();
        assert!(
            why.contains("paying off Debt") && why.contains("Open: "),
            "{why}"
        );
    }

    #[test]
    fn pick_can_make_a_turn_move_by_its_text() {
        let mut solo = game(&["Village", "Smithy"], 7);
        solo.run("treasures").unwrap();
        assert_eq!(solo.run("pick Buy Silver").unwrap().stopped, None);
        assert_eq!(owned(&solo, "Silver"), 1);
    }

    #[test]
    fn undo_takes_back_one_batch() {
        let mut solo = game(&["Village", "Smithy"], 7);
        solo.run("treasures; buy Silver; next").unwrap();
        let lines = solo.log().len();
        solo.run("treasures; buy Silver; next").unwrap();
        solo.undo().unwrap();
        assert_eq!(owned(&solo, "Silver"), 1);
        assert_eq!(solo.view().turn.number, Some(2));
        assert_eq!(solo.log().len(), lines);
        assert!(solo.undo().is_err());
    }

    #[test]
    fn a_saved_game_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path(), "plan-a").unwrap();
        let mut solo = game(&["Village", "Smithy"], 7);
        solo.run("treasures; buy Silver; next").unwrap();
        save(&file, &solo).unwrap();
        let mut again = load(&file).unwrap();
        assert_eq!(again.text(false), solo.text(false));
        assert!(solo.text(false).contains("SUPPLY ") && solo.text(false).contains("OPPONENT"));
        assert!(!solo.text(true).contains("SUPPLY ") && !solo.text(true).contains("OPPONENT"));
        assert_eq!(again.run("treasures; buy Silver").unwrap().stopped, None);
        assert!(path(dir.path(), "../x").is_err());
    }
}

//! Turns played by named steps (`play Village, Smithy x2; treasures; buy Gold; next`)
//! instead of one decision at a time. Solo games and hosted games both use them.

use dominion_agent::{AgentDecision, AgentOption, AgentView, Answer, Table};
use dominion_engine::PlayerId;

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
pub(crate) enum Step {
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

pub(crate) fn parse(steps: &str) -> Result<Vec<(String, Step)>, String> {
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

/// One seat playing steps on a copy of a table. With `idle_others`, every other seat is
/// answered with the least it can do whenever it must decide.
#[derive(Clone)]
pub struct Runner {
    pub table: Table,
    seat: PlayerId,
    idle_others: bool,
    /// Log lines the runner has seen, and the highest log number among them.
    pub log: Vec<String>,
    seen: Option<usize>,
}

impl Runner {
    /// A runner that treats the log lines with numbers up to `seen` as already shown.
    pub fn new(table: Table, seat: PlayerId, idle_others: bool, seen: Option<usize>) -> Runner {
        Runner {
            table,
            seat,
            idle_others,
            log: vec![],
            seen,
        }
    }

    /// The highest log number shown so far.
    pub fn seen(&self) -> Option<usize> {
        self.seen
    }

    pub fn view(&self) -> AgentView {
        self.table.view(self.seat)
    }

    fn decision(&self) -> Result<AgentDecision, String> {
        if let Some(d) = self.view().decision {
            return Ok(d);
        }
        Err(match self.table.deciding() {
            None => "the game is over".to_string(),
            Some(p) => format!(
                "it is {}'s decision now; wait for yours",
                self.table.names()[p.index()]
            ),
        })
    }

    /// Answers for the idle seats until the player must decide or the game ends.
    pub(crate) fn settle(&mut self) -> Result<(), String> {
        if !self.idle_others {
            return Ok(());
        }
        while let Some(seat) = self.table.deciding().filter(|&s| s != self.seat) {
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
    pub fn collect(&mut self) {
        for e in self.view().since_last_decision {
            if self.seen.is_none_or(|s| e.seq > s) {
                self.seen = Some(e.seq);
                self.log.push(e.text);
            }
        }
    }

    fn answer(&mut self, indices: Vec<u32>) -> Result<(), String> {
        let d = self.decision()?;
        self.respond(&Answer::indices(d.id, indices))
    }

    fn respond(&mut self, answer: &Answer) -> Result<(), String> {
        self.table
            .answer(self.seat, answer)
            .map_err(|e| e.to_string())?;
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
            // Only the Action phase: playing the last Action can end it already, and an
            // `end` written for it must not then end the Buy phase.
            Step::End => {
                let d = self.turn().ok_or("there is no phase of yours to end")?;
                if d.phase.as_deref() != Some("action") {
                    return Ok(());
                }
                let end = end_phase(&d).ok_or("the Action phase can't be ended yet")?;
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

    /// Does `steps` in order and stops at the first that can't be done: returns that step
    /// and why. The steps before it stand.
    pub fn run(&mut self, steps: &str) -> Result<Option<(String, String)>, String> {
        let steps = parse(steps)?;
        let mut turn = self.view().turn.number;
        for (raw, step) in &steps {
            if let Err(e) = self.step(step, &mut turn) {
                if let Some(p) = self.table.deciding().filter(|&p| p != self.seat) {
                    let who = &self.table.names()[p.index()];
                    let why = format!("it is {who}'s decision now; wait for yours");
                    return Ok(Some((raw.clone(), why)));
                }
                let open = self.view().decision.map(|d| {
                    let options: Vec<String> = d
                        .options
                        .iter()
                        .map(|o| format!("{}: {}", o.index, o.label))
                        .collect();
                    format!(". Open: {} [{}]", d.prompt, options.join("; "))
                });
                return Ok(Some((raw.clone(), e + &open.unwrap_or_default())));
            }
        }
        Ok(None)
    }
}

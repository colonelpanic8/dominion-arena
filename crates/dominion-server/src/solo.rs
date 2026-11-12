//! Solo games: one player at a table whose other seats sit idle, kept in a file and
//! advanced by named steps. For playing a deck's build out to see how fast it is.

use crate::hub::{self, NewGame};
use crate::steps::Runner;
use dominion_agent::{AgentView, Table, text};
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
    /// The highest log number in `log`.
    #[serde(default)]
    seen: Option<usize>,
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
        let mut runner = Runner::new(table, SEAT, true, None);
        runner.settle()?;
        runner.collect();
        let mut solo = Solo {
            table: runner.table.clone(),
            seed,
            log: vec![],
            seen: None,
            undo: None,
        };
        solo.adopt(runner);
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

    fn runner(&self) -> Runner {
        Runner::new(self.table.clone(), SEAT, true, self.seen)
    }

    fn adopt(&mut self, runner: Runner) {
        self.seen = runner.seen();
        self.log.extend(runner.log);
        self.table = runner.table;
    }

    /// Does `steps` in order and stops at the first that can't be done.
    pub fn run(&mut self, steps: &str) -> Result<Outcome, String> {
        let before = Snapshot {
            table: self.table.clone(),
            log: self.log.len(),
        };
        let mut runner = self.runner();
        let stopped = runner.run(steps)?;
        let from = self.log.len();
        self.adopt(runner);
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
        self.seen = None;
        let mut runner = self.runner();
        runner.collect();
        self.seen = runner.seen();
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
    use crate::steps::{Step, parse};

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
    fn end_only_ends_the_action_phase() {
        let mut solo = game(&["Smithy"], 7);
        solo.run("treasures; buy Smithy; next").unwrap();
        while !solo.view().you.unwrap().hand.known.contains_key("Smithy") {
            solo.run("treasures; next").unwrap();
        }
        let turn = solo.view().turn.number;
        // Playing the last Action ends the Action phase, in this call or an earlier one.
        solo.run("play Smithy").unwrap();
        let out = solo.run("end; treasures; buy Copper; end").unwrap();
        assert_eq!(out.stopped, None);
        assert!(out.log.iter().any(|l| l.contains("buys Copper")));
        assert_eq!(solo.view().turn.number, turn.map(|t| t + 1));
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

//! A seat a person and a model share: which of them holds it, what the person tells the
//! model, and what the model writes for the person holding it. A model's own seat taken
//! over and a person's seat with a model attached are the same thing from either side.
use crate::hub::HubError;
use dominion_protocol::api::{AdviseMode, ModelAction, ModelNote, ModelSeat};
use dominion_protocol::lobby::LaunchOptions;
use dominion_protocol::view::AgentView;
use serde::{Deserialize, Serialize};

/// The most notes kept for a seat.
const MAX_NOTES: usize = 40;
const MAX_NOTE_CHARS: usize = 4000;
const MAX_FROM_CHARS: usize = 60;

/// Who directs a seat's model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Director {
    Admin,
    /// The game's host: may prompt, never play.
    Host,
    /// The person whose own seat the model is attached to.
    Player,
}

/// A model attached to a person's seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attached {
    /// The agent's account id.
    pub user: String,
    pub name: String,
    /// What the model reaches the seat with: the seat's own token is its player's.
    pub token: String,
    #[serde(default)]
    pub options: LaunchOptions,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelControl {
    /// The account holding the seat; none: its model does.
    pub controller: Option<String>,
    pub prompts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached: Option<Attached>,
    /// The model the seat's player had attached at some point of the game, for its
    /// record: kept once the model is sent away.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aided: Option<String>,
    #[serde(default)]
    pub advise: AdviseMode,
    /// The holder asked for a note on the seat's next decision.
    #[serde(default)]
    pub requested: bool,
    /// The holder said something the model hasn't written back about.
    #[serde(default)]
    pub asked: bool,
    /// The decision the model last wrote about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advised: Option<String>,
    /// The turn of the last note [`AdviseMode::Buys`] asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advised_turn: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<ModelNote>,
}

/// Says in the server's log that who holds a seat changed: nothing else keeps when.
pub fn log_change(game: &str, seat: u8, who: &str, change: &str) {
    eprintln!("dominion: {game} seat {seat}: {who} {change}");
}

/// What `action` changes about who holds a seat, if it does.
pub fn holding_change(action: &ModelAction) -> Option<&'static str> {
    match action {
        ModelAction::Take => Some("took the seat from its model"),
        ModelAction::Release => Some("handed the seat to its model"),
        _ => None,
    }
}

/// What of a seat's view decides whether its model owes a note.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Moment {
    /// The seat's open decision.
    pub decision: Option<String>,
    /// That decision offers a buy.
    pub buying: bool,
    pub turn: Option<u32>,
}

impl Moment {
    pub fn of(view: &AgentView) -> Moment {
        Moment {
            decision: view.decision.as_ref().map(|d| d.id.clone()),
            buying: (view.decision.iter().flat_map(|d| &d.options)).any(|o| o.action == "buy"),
            turn: view.turn.number,
        }
    }
}

impl ModelControl {
    pub fn update(
        &mut self,
        who: &str,
        role: Director,
        action: &ModelAction,
    ) -> Result<(), HubError> {
        let holding = self.controller.as_deref() == Some(who);
        let changed = || HubError::Conflict("Control of this seat has changed".into());
        match action {
            ModelAction::Prompt { text } => {
                let text = text.trim();
                if text.is_empty() || text.chars().count() > 2000 {
                    return Err(HubError::BadRequest(
                        "Enter a prompt of 1–2000 characters".into(),
                    ));
                }
                self.prompts.push(text.into());
                self.asked = self.controller.is_some();
            }
            _ if role == Director::Host => {
                return Err(HubError::Forbidden(
                    "Only an admin can play a model seat".into(),
                ));
            }
            ModelAction::Take if self.controller.is_none() => self.controller = Some(who.into()),
            ModelAction::Release if holding => {
                self.controller = None;
                (self.requested, self.asked) = (false, false);
            }
            ModelAction::Answer { .. } if holding => {}
            ModelAction::Advise { mode } if holding => self.advise = *mode,
            ModelAction::Request if holding => self.requested = true,
            ModelAction::Attach { .. } | ModelAction::Detach => {
                return Err(HubError::BadRequest(
                    "a model is attached and sent away through the arena".into(),
                ));
            }
            _ => return Err(changed()),
        }
        Ok(())
    }

    /// The seat as `who` may see it. `view` is the seat's own view, read only for whoever
    /// holds it.
    pub fn seat(
        &self,
        who: &str,
        seat: u8,
        name: String,
        version: u64,
        view: impl FnOnce() -> AgentView,
    ) -> ModelSeat {
        let holding = self.controller.as_deref() == Some(who);
        ModelSeat {
            seat,
            name,
            controlled: self.controller.is_some(),
            playing: (holding && self.attached.is_none()).then(|| {
                let view = view();
                dominion_protocol::api::ModelHand {
                    hand: view.you.map(|p| p.hand).unwrap_or_default(),
                    decision: view.decision,
                    version,
                }
            }),
            prompts: self.prompts.clone(),
            agent: self.attached.as_ref().map(|a| a.name.clone()),
            own: self.attached.is_some(),
            holding,
            advise: self.advise,
            requested: holding && self.requested,
            notes: if holding { self.notes.clone() } else { vec![] },
        }
    }

    /// Whether the seat's model may answer for it.
    pub fn model_turn(&self) -> Result<(), HubError> {
        if self.controller.is_some() {
            return Err(HubError::Conflict(
                "A person is playing this seat and you are advising them: answer nothing; \
                 call wait until you are asked for advice or the seat is handed back"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Whether the person a model is attached to may answer for their seat.
    pub fn player_turn(&self) -> Result<(), HubError> {
        match (&self.attached, &self.controller) {
            (Some(model), None) => Err(HubError::Conflict(format!(
                "{} is playing your seat: take it back first",
                model.name
            ))),
            _ => Ok(()),
        }
    }

    /// Whether the seat's model owes its holder a note at `now`.
    pub fn wants_advice(&self, now: &Moment) -> bool {
        if self.controller.is_none() {
            return false;
        }
        if self.asked {
            return true;
        }
        if now.decision.is_none() || self.advised == now.decision {
            return false;
        }
        self.requested
            || match self.advise {
                AdviseMode::Off => false,
                AdviseMode::Always => true,
                AdviseMode::Buys => now.buying && self.advised_turn != now.turn,
            }
    }

    /// Whether anything could make the model owe a note without the holder asking again:
    /// what [`ModelControl::wants_advice`] needs the seat's view to settle.
    pub fn may_want_advice(&self) -> bool {
        self.controller.is_some()
            && (self.asked || self.requested || self.advise != AdviseMode::Off)
    }

    /// Keeps what the model wrote, as about the decision open at `now`.
    pub fn note(&mut self, now: &Moment, text: &str, at: u64) -> Result<(), HubError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(HubError::BadRequest("advise needs text".into()));
        }
        if self.controller.is_none() {
            return Err(HubError::Conflict(
                "You hold this seat: nobody is reading advice. Play it".into(),
            ));
        }
        if now.buying {
            self.advised_turn = now.turn;
        }
        self.advised = now.decision.clone();
        (self.requested, self.asked) = (false, false);
        self.keep(ModelNote {
            from: None,
            decision: now.decision.clone(),
            turn: now.turn,
            text: text.into(),
            at,
        });
        Ok(())
    }

    /// Keeps what `from`, an analyst working beside the seat's model, wrote: whoever
    /// holds the seat, and settling nothing its holder asked of the model.
    pub fn helper_note(
        &mut self,
        now: &Moment,
        from: &str,
        text: &str,
        at: u64,
    ) -> Result<(), HubError> {
        let (from, text) = (from.trim(), text.trim());
        if text.is_empty() || from.is_empty() || from.chars().count() > MAX_FROM_CHARS {
            return Err(HubError::BadRequest(
                "note needs text and who it is from, in at most 60 characters".into(),
            ));
        }
        self.keep(ModelNote {
            from: Some(from.into()),
            decision: None,
            turn: now.turn,
            text: text.into(),
            at,
        });
        Ok(())
    }

    fn keep(&mut self, mut note: ModelNote) {
        note.text = note.text.chars().take(MAX_NOTE_CHARS).collect();
        self.notes.push(note);
        let extra = self.notes.len().saturating_sub(MAX_NOTES);
        self.notes.drain(..extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(decision: Option<(&str, &str)>, turn: u32) -> Moment {
        Moment {
            decision: decision.map(|(id, _)| id.into()),
            buying: decision.is_some_and(|(_, action)| action == "buy"),
            turn: Some(turn),
        }
    }

    fn held() -> ModelControl {
        ModelControl {
            controller: Some("ann".into()),
            ..ModelControl::default()
        }
    }

    #[test]
    fn a_model_holding_the_seat_owes_no_advice() {
        let mut control = ModelControl {
            advise: AdviseMode::Always,
            ..ModelControl::default()
        };
        assert!(!control.wants_advice(&view(Some(("d1", "buy")), 1)));
        assert!(control.note(&view(None, 1), "Buy Gold", 0).is_err());
        assert!(control.model_turn().is_ok());
    }

    #[test]
    fn a_request_is_for_the_next_decision_and_one_note_settles_it() {
        let mut control = held();
        let d1 = view(Some(("d1", "play")), 3);
        assert!(!control.wants_advice(&d1));
        (control.update("ann", Director::Player, &ModelAction::Request)).unwrap();
        assert!(!control.wants_advice(&view(None, 3)));
        assert!(control.wants_advice(&d1));
        control.note(&d1, " Play Village first. ", 7).unwrap();
        assert!(!control.wants_advice(&d1));
        assert!(!control.wants_advice(&view(Some(("d2", "play")), 3)));
        assert_eq!(control.notes[0].text, "Play Village first.");
        assert_eq!(control.notes[0].decision.as_deref(), Some("d1"));
    }

    #[test]
    fn an_analysts_note_is_kept_whoever_holds_the_seat_and_settles_nothing() {
        let mut models = ModelControl::default();
        let d1 = view(Some(("d1", "buy")), 2);
        (models.helper_note(&d1, "Opening analyst", "Open Militia.", 5)).unwrap();
        assert!(models.helper_note(&d1, " ", "Open Militia.", 5).is_err());
        let mut control = held();
        (control.update("ann", Director::Player, &ModelAction::Request)).unwrap();
        (control.helper_note(&d1, "Strategy analyst", "Green now.", 5)).unwrap();
        assert!(control.wants_advice(&d1));
        let note = &control.notes[0];
        assert_eq!(note.from.as_deref(), Some("Strategy analyst"));
        assert_eq!((note.decision.as_deref(), note.turn), (None, Some(2)));
    }

    #[test]
    fn buys_are_written_about_once_a_turn() {
        let mut control = held();
        let set = ModelAction::Advise {
            mode: AdviseMode::Buys,
        };
        control.update("ann", Director::Player, &set).unwrap();
        assert!(!control.wants_advice(&view(Some(("d1", "play")), 2)));
        let buying = view(Some(("d2", "buy")), 2);
        assert!(control.wants_advice(&buying));
        control.note(&buying, "Gold", 0).unwrap();
        assert!(!control.wants_advice(&view(Some(("d3", "buy")), 2)));
        assert!(control.wants_advice(&view(Some(("d9", "buy")), 3)));
    }

    #[test]
    fn a_question_is_owed_an_answer_with_no_decision_open() {
        let mut control = held();
        let ask = ModelAction::Prompt {
            text: "Can they end it next turn?".into(),
        };
        control.update("ann", Director::Player, &ask).unwrap();
        assert!(control.wants_advice(&view(None, 4)));
        control
            .note(&view(None, 4), "Not before turn 12.", 0)
            .unwrap();
        assert!(!control.wants_advice(&view(None, 4)));
    }

    #[test]
    fn only_its_holder_changes_how_a_seat_is_advised_and_a_host_never_plays() {
        let mut control = held();
        let request = ModelAction::Request;
        assert!(control.update("bob", Director::Admin, &request).is_err());
        assert!(
            control
                .update("bob", Director::Admin, &ModelAction::Take)
                .is_err()
        );
        let mut models = ModelControl::default();
        assert!(
            models
                .update("hal", Director::Host, &ModelAction::Take)
                .is_err()
        );
        (models.update("bob", Director::Admin, &ModelAction::Take)).unwrap();
        assert!(models.model_turn().is_err());
    }

    #[test]
    fn a_person_whose_model_holds_their_seat_takes_it_back_to_play() {
        let mut control = ModelControl {
            attached: Some(Attached {
                user: "u1".into(),
                name: "Astra".into(),
                token: "t".into(),
                options: LaunchOptions::default(),
            }),
            ..held()
        };
        assert!(control.player_turn().is_ok() && control.model_turn().is_err());
        (control.update("ann", Director::Player, &ModelAction::Release)).unwrap();
        assert!(control.player_turn().is_err() && control.model_turn().is_ok());
        let seat = control.seat("ann", 0, "Ann".into(), 0, || unreachable!());
        assert!(seat.own && !seat.holding && seat.notes.is_empty());
    }
}

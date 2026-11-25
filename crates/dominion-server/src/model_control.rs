//! Instructions and manual control of a single model seat.
use crate::hub::HubError;
use dominion_protocol::api::{ModelAction, ModelSeat};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelControl {
    pub controller: Option<String>,
    pub prompts: Vec<String>,
}

impl ModelControl {
    pub fn update(&mut self, who: &str, admin: bool, action: &ModelAction) -> Result<(), HubError> {
        match action {
            ModelAction::Prompt { text } => {
                let text = text.trim();
                if self.controller.is_some() {
                    return Err(HubError::Conflict(
                        "Return this seat to the model first".into(),
                    ));
                }
                if text.is_empty() || text.chars().count() > 2000 {
                    return Err(HubError::BadRequest(
                        "Enter a prompt of 1–2000 characters".into(),
                    ));
                }
                self.prompts.push(text.into());
            }
            _ if !admin => {
                return Err(HubError::Forbidden(
                    "Only an admin can play a model seat".into(),
                ));
            }
            ModelAction::Take if self.controller.is_none() => self.controller = Some(who.into()),
            ModelAction::Release if self.controller.as_deref() == Some(who) => {
                self.controller = None
            }
            ModelAction::Answer { .. } if self.controller.as_deref() == Some(who) => {}
            _ => {
                return Err(HubError::Conflict(
                    "Control of this seat has changed".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn seat(
        &self,
        admin: Option<&str>,
        seat: u8,
        name: String,
        view: impl FnOnce() -> dominion_protocol::AgentView,
    ) -> ModelSeat {
        let yours = admin.is_some() && self.controller.as_deref() == admin;
        ModelSeat {
            seat,
            name,
            controlled: self.controller.is_some(),
            playing: yours.then(|| {
                let view = view();
                dominion_protocol::api::ModelHand {
                    hand: view.you.map(|p| p.hand).unwrap_or_default(),
                    decision: view.decision,
                }
            }),
            prompts: self.prompts.clone(),
        }
    }

    pub fn model_turn(&self) -> Result<(), HubError> {
        if self.controller.is_some() {
            return Err(HubError::Conflict(
                "An admin is playing this seat; call wait until it is handed back".into(),
            ));
        }
        Ok(())
    }
}

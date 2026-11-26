use super::*;

#[derive(Default)]
pub(super) struct OfficialModel {
    game: String,
    agent: String,
    control: crate::model_control::ModelControl,
    generation: u64,
}

impl Hosted {
    pub(super) fn check_assignment(&self, game: &str) -> Result<(), String> {
        let model = crate::hub::lock(&self.model);
        if model.game == game {
            model.control.model_turn().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub(in crate::online) fn model_generation(&self) -> u64 {
        crate::hub::lock(&self.model).generation
    }

    pub(in crate::online) fn prompts(&self, game: &str) -> Vec<String> {
        let model = crate::hub::lock(&self.model);
        if model.game == game {
            model.control.prompts.clone()
        } else {
            vec![]
        }
    }

    /// What the account's model is told while a person holds its seat in the game `view`
    /// shows.
    pub(in crate::online) fn advising(
        &self,
        view: &dominion_protocol::AgentView,
    ) -> Option<crate::hub::Advising> {
        let model = crate::hub::lock(&self.model);
        let held = view.game.running
            && view.game.id.as_ref() == Some(&model.game)
            && model.control.controller.is_some();
        held.then(|| crate::hub::Advising {
            wanted: (model.control).wants_advice(&crate::model_control::Moment::of(view)),
        })
    }

    /// Keeps what the account's model wrote for the person holding its seat.
    pub(in crate::online) fn advise(
        &self,
        view: &dominion_protocol::AgentView,
        text: &str,
    ) -> Result<(), String> {
        let mut model = crate::hub::lock(&self.model);
        if view.game.id.as_ref() != Some(&model.game) {
            return Err("REFUSED: nobody holds this seat for you to advise".into());
        }
        let now = crate::model_control::Moment::of(view);
        (model.control.note(&now, text, crate::hub::now_ms())).map_err(|e| format!("REFUSED: {e}"))
    }

    /// Keeps what `from`, an analyst working beside the account's model, wrote in the
    /// running game `view` shows.
    pub(in crate::online) fn note(
        &self,
        view: &dominion_protocol::AgentView,
        from: &str,
        text: &str,
    ) -> Result<(), String> {
        let Some(game) = view.game.id.as_ref().filter(|_| view.game.running) else {
            return Err("REFUSED: no game is running".into());
        };
        let mut model = crate::hub::lock(&self.model);
        if &model.game != game {
            model.game = game.clone();
            model.control = Default::default();
        }
        let now = crate::model_control::Moment::of(view);
        (model
            .control
            .helper_note(&now, from, text, crate::hub::now_ms()))
        .map_err(|e| format!("REFUSED: {e}"))
    }

    pub(in crate::online) fn check_model(&self, generation: u64) -> Result<(), String> {
        let model = crate::hub::lock(&self.model);
        if generation != model.generation {
            return Err("Control of this seat has changed".into());
        }
        if self
            .client
            .current()
            .view()?
            .and_then(|v| v.game.id)
            .as_ref()
            == Some(&model.game)
        {
            model.control.model_turn().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn model_user(web: &Web, headers: &HeaderMap) -> Result<(String, bool), crate::http::Error> {
    let token = crate::http::bearer(headers).or_else(|| crate::http::cookie(headers));
    let ident = token
        .and_then(|t| web.access.arena.identify(t))
        .ok_or_else(|| crate::http::Error::new(StatusCode::UNAUTHORIZED, "Sign in first"))?;
    let (name, admin) = web.access.arena.model_director(&ident)?;
    if !admin && (ident.agent || name != web.hosted.owner) {
        return Err(crate::http::Error::new(
            StatusCode::FORBIDDEN,
            "Only the official account owner and admins can direct this model",
        ));
    }
    Ok((ident.user, admin))
}

fn model_status(web: &Web, who: &str) -> Vec<dominion_protocol::api::ModelSeat> {
    let update = web.hosted.client.current();
    let version = update.reducer.revision;
    let Some(view) = update.view().ok().flatten().filter(|v| v.game.running) else {
        return vec![];
    };
    let (Some(game), Some(seat)) = (view.game.id.as_ref(), view.seat) else {
        return vec![];
    };
    let model = crate::hub::lock(&web.hosted.model);
    let job = web.job().filter(|j| &j.game_id == game);
    let current = &model.game == game;
    if job.is_none() && !(current && model.control.controller.is_some()) {
        return vec![];
    }
    let control = if current {
        model.control.clone()
    } else {
        Default::default()
    };
    vec![control.seat(
        who,
        seat,
        job.map(|j| j.agent).unwrap_or_else(|| model.agent.clone()),
        version,
        || view,
    )]
}

pub(super) async fn model_seats(
    State(web): State<Web>,
    headers: HeaderMap,
) -> Result<axum::Json<Vec<dominion_protocol::api::ModelSeat>>, crate::http::Error> {
    let (who, admin) = model_user(&web, &headers)?;
    let _guard = web.hosted.control.lock().await;
    let _ = admin;
    Ok(axum::Json(model_status(&web, &who)))
}

pub(super) async fn model_action(
    State(web): State<Web>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<dominion_protocol::api::ModelRequest>,
) -> Response {
    use dominion_protocol::api::ModelAction;
    let (who, admin) = match model_user(&web, &headers) {
        Ok(user) => user,
        Err(e) => return e.into_response(),
    };
    let _guard = web.hosted.control.lock().await;
    let Some(seat) = model_status(&web, &who).pop() else {
        return refused("This model is no longer playing");
    };
    let current = web
        .hosted
        .client
        .current()
        .view()
        .ok()
        .flatten()
        .and_then(|v| v.game.id);
    if current.as_ref() != Some(&req.game) || seat.seat != req.seat {
        return refused("The game has changed");
    }
    let mut control = {
        let model = crate::hub::lock(&web.hosted.model);
        if model.game == req.game {
            model.control.clone()
        } else {
            Default::default()
        }
    };
    // The account's owner is its player: they hold its seat as an admin may.
    let role = match admin {
        true => crate::model_control::Director::Admin,
        false => crate::model_control::Director::Player,
    };
    if let Err(e) = control.update(&who, role, &req.action) {
        return crate::http::Error::from(e).into_response();
    }
    match &req.action {
        ModelAction::Answer { answer } => {
            if let Err(e) = web.hosted.client.answer(answer.clone(), None).await {
                return refused(e);
            }
        }
        // The model went on reading along while its seat was held: it has it back.
        ModelAction::Release if web.job().is_some_and(|j| j.game_id == req.game) => {}
        ModelAction::Release => {
            let strategy = web
                .access
                .arena
                .official_game_options(&web.hosted.id, &req.game)
                .unwrap_or_default();
            let reservation = match web.access.arena.reserve_official(
                &web.hosted.owner,
                &web.hosted.id,
                &seat.name,
                strategy,
            ) {
                Ok(r) => r,
                Err(e) => return refused(e),
            };
            {
                let mut model = crate::hub::lock(&web.hosted.model);
                model.generation += 1;
                model.control.controller = None;
            }
            if let Err(e) = assign(&web, seat.name.clone(), strategy) {
                crate::hub::lock(&web.hosted.model).control.controller = Some(who);
                return refused(e);
            }
            drop(reservation);
        }
        _ => {}
    }
    if let Some(change) = crate::model_control::holding_change(&req.action) {
        crate::model_control::log_change(&req.game, req.seat, &who, change);
    }
    {
        let mut model = crate::hub::lock(&web.hosted.model);
        model.game = req.game;
        model.control = control;
        model.agent = seat.name;
    }
    axum::Json(model_status(&web, &who)).into_response()
}

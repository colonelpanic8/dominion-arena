use super::*;
use crate::model_control::{Director, ModelControl};

impl Hub {
    pub(crate) fn restore_model_host(
        &self,
        id: &str,
        agents: Vec<bool>,
        host: String,
    ) -> Result<(), HubError> {
        let game = self.game(id)?;
        let mut table = lock(&game.table);
        self.commit(&game, &mut table, None, |meta, _| {
            meta.agents = agents;
            meta.host = Some(host);
        })
    }

    /// Notes that an agent the server doesn't run plays `seat`, so it can be directed and
    /// taken over like the server's own.
    pub(crate) fn mark_remote_agent(&self, seat: &Seat) {
        let Ok(game) = self.game(&seat.game) else {
            return;
        };
        let i = seat.seat.index();
        if model_seat(&lock(&game.meta), i) {
            return;
        }
        let mut table = lock(&game.table);
        if table.is_over() || i >= table.names().len() {
            return;
        }
        let marked = self.commit(&game, &mut table, None, |meta, table| {
            meta.remote_agents.resize(table.names().len(), false);
            meta.remote_agents[i] = true;
        });
        if marked.is_ok() {
            game.version.send_modify(|v| *v += 1);
        }
    }

    pub(crate) fn model_prompts(&self, seat: &Seat) -> Vec<String> {
        self.read(&seat.game)
            .ok()
            .and_then(|g| {
                lock(&g.meta)
                    .model_controls
                    .get(seat.seat.index())
                    .map(|c| c.prompts.clone())
            })
            .unwrap_or_default()
    }

    /// The seats of game `id` with a model at them, as `who` may see them: the models'
    /// own seats for someone with `standing` toward them (the host, an admin), and the
    /// seat of `who`'s own a model is attached to.
    pub(crate) fn model_seats(
        &self,
        id: &str,
        who: &str,
        standing: Option<Director>,
    ) -> Result<Vec<dominion_protocol::api::ModelSeat>, HubError> {
        let game = self.game(id)?;
        let table = lock(&game.table);
        if table.is_over() {
            return Ok(vec![]);
        }
        let meta = lock(&game.meta);
        Ok((0..table.names().len())
            .filter(|&i| match attached(&meta, i) {
                true => meta.owners.get(i).and_then(Option::as_deref) == Some(who),
                false => standing.is_some() && model_seat(&meta, i),
            })
            .map(|i| {
                let seat = i as u8;
                let control = meta.model_controls.get(i).cloned().unwrap_or_default();
                let holding = control.controller.as_deref() == Some(who);
                if holding && control.attached.is_none() {
                    game.touch(&Seat {
                        game: id.into(),
                        seat: PlayerId(seat),
                        model: false,
                    });
                }
                control.seat(who, seat, table.names()[i].clone(), || {
                    table.view(PlayerId(seat))
                })
            })
            .collect())
    }

    /// `who` directs the model at a seat of game `id`. `standing` is theirs toward the
    /// models' own seats; a seat a model is attached to is directed by its player alone.
    pub(crate) fn model_action(
        &self,
        id: &str,
        who: &str,
        standing: Option<Director>,
        req: &dominion_protocol::api::ModelRequest,
    ) -> Result<(), HubError> {
        use dominion_protocol::api::ModelAction;
        let game = self.game(id)?;
        self.in_time(&game, id)?;
        let mut table = lock(&game.table);
        let seat = PlayerId(req.seat);
        let (role, mut control) = {
            let meta = lock(&game.meta);
            if req.game != id || table.is_over() || !model_seat(&meta, seat.index()) {
                return Err(HubError::Conflict(
                    "This model seat is no longer playing".into(),
                ));
            }
            let own = meta.owners.get(seat.index()).and_then(Option::as_deref) == Some(who);
            let role = match attached(&meta, seat.index()) {
                true => own.then_some(Director::Player),
                false => standing,
            };
            let control = meta.model_controls.get(seat.index()).cloned();
            (role, control.unwrap_or_default())
        };
        let role = role.ok_or_else(|| {
            HubError::Forbidden("Only the game host and admins can direct its models".into())
        })?;
        control.update(who, role, &req.action)?;
        if let ModelAction::Answer { answer } = &req.action {
            let (next, _) = self.attempt(&table, |next| next.answer_with(seat, answer, None))?;
            let ended = self.changed(&game, &mut table, next, |_| {})?;
            self.answered(&game, &table, seat);
            drop(table);
            self.ended(ended);
        } else {
            self.commit(&game, &mut table, None, |meta, table| {
                meta.model_controls
                    .resize_with(table.names().len(), Default::default);
                meta.model_controls[seat.index()] = control;
            })?;
            game.version.send_modify(|v| *v += 1);
        }
        Ok(())
    }

    /// Attaches the agent `user` (named `name`) to seat `seat` of game `id`, which is
    /// `who`'s own: they go on holding it, and the model reads it with a token of its own.
    pub(crate) fn attach_model(
        &self,
        id: &str,
        seat: u8,
        who: &str,
        (user, name): (String, String),
        options: dominion_protocol::lobby::LaunchOptions,
    ) -> Result<(), HubError> {
        let game = self.game(id)?;
        let mut table = lock(&game.table);
        let i = usize::from(seat);
        {
            let meta = lock(&game.meta);
            if table.is_over() {
                return Err(HubError::Conflict("The game is over".into()));
            }
            if meta.owners.get(i).and_then(Option::as_deref) != Some(who) || model_seat(&meta, i) {
                return Err(HubError::Forbidden(
                    "A model is attached to a seat of your own that you play yourself".into(),
                ));
            }
            if meta.rated {
                return Err(HubError::Forbidden(
                    "A rated game is played without a model's help".into(),
                ));
            }
        }
        let token = token();
        let control = ModelControl {
            controller: Some(who.into()),
            attached: Some(crate::model_control::Attached {
                user,
                name,
                token: token.clone(),
                options,
            }),
            ..ModelControl::default()
        };
        self.commit(&game, &mut table, None, |meta, table| {
            meta.model_controls
                .resize_with(table.names().len(), Default::default);
            meta.model_controls[i] = control;
        })?;
        let model = Seat {
            game: id.into(),
            seat: PlayerId(seat),
            model: true,
        };
        lock(&self.index).seats.insert(token, model);
        game.version.send_modify(|v| *v += 1);
        Ok(())
    }

    /// Sends away the model attached to `who`'s seat `seat` of game `id`: the seat is
    /// theirs alone again.
    pub(crate) fn detach_model(&self, id: &str, seat: u8, who: &str) -> Result<(), HubError> {
        let game = self.game(id)?;
        let mut table = lock(&game.table);
        let i = usize::from(seat);
        let token = {
            let meta = lock(&game.meta);
            let model = meta.model_controls.get(i).and_then(|c| c.attached.as_ref());
            match model {
                Some(model) if meta.owners[i].as_deref() == Some(who) => model.token.clone(),
                _ => {
                    return Err(HubError::Conflict(
                        "No model is attached to that seat".into(),
                    ));
                }
            }
        };
        self.commit(&game, &mut table, None, |meta, _| {
            meta.model_controls[i] = ModelControl::default();
        })?;
        lock(&self.index).seats.remove(&token);
        game.version.send_modify(|v| *v += 1);
        Ok(())
    }

    /// Every seat of a running game a model is attached to, with the model.
    pub(crate) fn attached(&self) -> Vec<(MyGame, crate::model_control::Attached)> {
        let games: Vec<(String, Arc<Hosted>)> = {
            let index = lock(&self.index);
            (index.games.iter())
                .map(|(id, game)| (id.clone(), game.clone()))
                .collect()
        };
        let mut found = vec![];
        for (id, game) in games {
            let models: Vec<_> = (lock(&game.meta).model_controls.iter().enumerate())
                .filter_map(|(i, c)| Some((i, c.attached.clone()?)))
                .collect();
            if models.is_empty() {
                continue;
            }
            let table = lock(&game.table);
            if table.is_over() {
                continue;
            }
            for (i, model) in models {
                let seated = MyGame {
                    game_id: id.clone(),
                    seat: i as u8,
                    name: table.names()[i].clone(),
                    token: model.token.clone(),
                    running: true,
                    opponents: (table.names().iter().enumerate())
                        .filter(|&(other, _)| other != i)
                        .map(|(_, n)| n.clone())
                        .collect(),
                };
                found.push((seated, model));
            }
        }
        found
    }

    /// Keeps what `seat`'s model wrote for the person holding the seat.
    pub(crate) fn advise(&self, seat: &Seat, text: &str) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let mut table = lock(&game.table);
        let i = seat.seat.index();
        let mut control = {
            let meta = lock(&game.meta);
            let control = meta.model_controls.get(i).cloned().unwrap_or_default();
            if table.is_over() || !(seat.model || control.attached.is_none()) {
                return Err(HubError::Conflict(
                    "Only the model of a seat a person holds writes advice".into(),
                ));
            }
            control
        };
        let now = crate::model_control::Moment::of(&table.view(seat.seat));
        control.note(&now, text, now_ms())?;
        self.commit(&game, &mut table, None, |meta, table| {
            meta.model_controls
                .resize_with(table.names().len(), Default::default);
            meta.model_controls[i] = control;
        })?;
        game.version.send_modify(|v| *v += 1);
        Ok(())
    }

    /// What `seat`'s caller is told about who holds the seat, when it is the seat's model
    /// and a person holds it.
    pub(super) fn advising(&self, game: &Hosted, table: &Table, seat: &Seat) -> Option<Advising> {
        let control = {
            let meta = lock(&game.meta);
            (meta.model_controls.get(seat.seat.index()))
                .filter(|c| seat.model || c.attached.is_none())
                .filter(|c| c.controller.is_some())
                .cloned()?
        };
        Some(Advising {
            wanted: control.wants_advice(&crate::model_control::Moment::of(&table.view(seat.seat))),
        })
    }
}

/// Whether a model is attached to seat `i`, a person's own.
fn attached(meta: &Meta, i: usize) -> bool {
    (meta.model_controls.get(i)).is_some_and(|c| c.attached.is_some())
}

/// Whether a model is at seat `i`: an agent plays it (one of the server's, or one from
/// elsewhere), an admin has taken it over, or one is attached to its player.
fn model_seat(meta: &Meta, i: usize) -> bool {
    meta.agents.get(i) == Some(&true)
        || meta.remote_agents.get(i) == Some(&true)
        || (meta.model_controls.get(i))
            .is_some_and(|c| c.controller.is_some() || c.attached.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_protocol::api::{ModelAction, ModelRequest};

    #[test]
    fn a_reopened_game_keeps_the_prompt_and_the_manual_seat() {
        let dir = tempfile::tempdir().unwrap();
        let open = || {
            let (hub, loaded) = Hub::open(dir.path(), Duration::from_secs(3600)).unwrap();
            assert!(loaded.unreadable.is_empty(), "{loaded:?}");
            hub
        };
        let hub = open();
        let config = config(&NewGame {
            players: 2,
            kingdom: vec!["Village".into()],
            seed: Some(1),
            ..Default::default()
        })
        .unwrap();
        let created = hub
            .host(
                config,
                vec!["Model".into(), "Opponent".into()],
                Meta {
                    agents: vec![true, false],
                    ..Default::default()
                },
            )
            .unwrap();
        let id = created.game_id;
        let request = |action| ModelRequest {
            game: id.clone(),
            seat: 0,
            action,
        };
        hub.model_action(
            &id,
            "host",
            Some(Director::Host),
            &request(ModelAction::Prompt {
                text: "Buy Village".into(),
            }),
        )
        .unwrap();
        hub.model_action(
            &id,
            "admin",
            Some(Director::Admin),
            &request(ModelAction::Take),
        )
        .unwrap();
        drop(hub);

        let hub = open();
        let admin = Some(Director::Admin);
        let seat = hub.model_seats(&id, "admin", admin).unwrap().remove(0);
        assert_eq!(seat.prompts, ["Buy Village"]);
        assert!(seat.controlled && seat.playing.unwrap().decision.is_some());
        let others = hub.model_seats(&id, "host", Some(Director::Host)).unwrap();
        assert!(others[0].playing.is_none());
        hub.model_action(
            &id,
            "admin",
            Some(Director::Admin),
            &request(ModelAction::Release),
        )
        .unwrap();
        assert!(!hub.model_seats(&id, "admin", admin).unwrap()[0].controlled);
    }

    #[tokio::test]
    async fn an_admin_takes_over_a_seat_an_outside_agent_plays() {
        let hub = Hub::default();
        let config = config(&NewGame {
            players: 2,
            kingdom: vec!["Village".into()],
            seed: Some(1),
            ..Default::default()
        })
        .unwrap();
        let created = hub
            .host(config, vec!["A".into(), "B".into()], Meta::default())
            .unwrap();
        let id = created.game_id;
        let seats: Vec<Seat> = (created.seats.iter())
            .map(|s| hub.find(&s.token).unwrap())
            .collect();
        let seat = (seats.into_iter())
            .find(|s| hub.view(s).unwrap().0.decision.is_some())
            .unwrap();
        let host = Some(Director::Host);
        assert!(hub.model_seats(&id, "host", host).unwrap().is_empty());

        hub.mark_remote_agent(&seat);
        let listed = hub.model_seats(&id, "host", host).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].seat, seat.seat.0);

        let request = |action| ModelRequest {
            game: id.clone(),
            seat: seat.seat.0,
            action,
        };
        hub.model_action(
            &id,
            "admin",
            Some(Director::Admin),
            &request(ModelAction::Take),
        )
        .unwrap();
        let decision = hub.view(&seat).unwrap().0.decision.unwrap();
        let answer = Answer {
            decision: decision.id.clone(),
            indices: Some(vec![decision.options[0].index]),
            name: None,
        };
        assert!(hub.answer_with(&seat, &answer, None).is_err());
        assert!(!hub.wait(&seat, Duration::from_millis(20)).await.unwrap());

        hub.model_action(
            &id,
            "admin",
            Some(Director::Admin),
            &request(ModelAction::Release),
        )
        .unwrap();
        assert!(hub.wait(&seat, Duration::from_millis(20)).await.unwrap());
        hub.answer_with(&seat, &answer, None).unwrap();
    }

    #[tokio::test]
    async fn a_person_and_the_model_attached_to_their_seat_share_it() {
        let dir = tempfile::tempdir().unwrap();
        let open = || Hub::open(dir.path(), Duration::from_secs(3600)).unwrap().0;
        let hub = open();
        let config = config(&NewGame {
            players: 2,
            kingdom: vec!["Village".into()],
            seed: Some(1),
            ..Default::default()
        })
        .unwrap();
        let meta = Meta {
            owners: vec![Some("ann".into()), Some("bo".into())],
            ..Default::default()
        };
        let created = hub.host(config, vec!["Ann".into(), "Bo".into()], meta);
        let created = created.unwrap();
        let id = created.game_id;
        let person = (created.seats.iter())
            .map(|s| hub.find(&s.token).unwrap())
            .find(|s| hub.view(s).unwrap().0.decision.is_some())
            .unwrap();
        let (who, other) = match person.seat.0 {
            0 => ("ann", "bo"),
            _ => ("bo", "ann"),
        };
        let request = |action| ModelRequest {
            game: id.clone(),
            seat: person.seat.0,
            action,
        };
        let astra = || ("u-astra".to_string(), "Astra".to_string());
        let attach = |by| hub.attach_model(&id, person.seat.0, by, astra(), Default::default());
        assert!(attach(other).is_err());
        attach(who).unwrap();
        assert!(attach(who).is_err());
        let (job, attached) = hub.attached().remove(0);
        assert_eq!(attached.user, "u-astra");
        drop(hub);

        let hub = open();
        let model = hub.find(&job.token).unwrap();
        assert!(
            model.model
                && model.seat == person.seat
                && job.name == ["Ann", "Bo"][model.seat.index()]
        );
        let admin = Some(Director::Admin);
        assert!(hub.model_seats(&id, other, admin).unwrap().is_empty());
        assert!(
            hub.model_action(&id, other, admin, &request(ModelAction::Take))
                .is_err()
        );
        let listed = hub.model_seats(&id, who, None).unwrap().remove(0);
        assert!(listed.own && listed.holding && listed.agent.as_deref() == Some("Astra"));

        let decision = hub.view(&person).unwrap().0.decision.unwrap();
        let answer = Answer {
            decision: decision.id.clone(),
            indices: Some(vec![decision.options[0].index]),
            name: None,
        };
        let soon = Duration::from_millis(20);
        assert!(hub.answer_with(&model, &answer, None).is_err());
        assert!(!hub.wait(&model, soon).await.unwrap());
        let advising = |seat| hub.play_observation(seat).unwrap().advising;
        assert_eq!(advising(&model), Some(Advising { wanted: false }));
        assert_eq!(advising(&person), None);

        hub.model_action(&id, who, None, &request(ModelAction::Request))
            .unwrap();
        assert!(hub.wait(&model, soon).await.unwrap());
        assert_eq!(advising(&model), Some(Advising { wanted: true }));
        assert!(hub.advise(&person, "Buy Village.").is_err());
        hub.advise(&model, "Buy Village.").unwrap();
        let listed = hub.model_seats(&id, who, None).unwrap().remove(0);
        assert_eq!(listed.notes[0].text, "Buy Village.");
        assert_eq!(listed.notes[0].decision.as_ref(), Some(&decision.id));
        assert!(!hub.wait(&model, soon).await.unwrap());

        hub.model_action(&id, who, None, &request(ModelAction::Release))
            .unwrap();
        assert!(hub.answer_with(&person, &answer, None).is_err());
        assert!(hub.wait(&model, soon).await.unwrap());
        assert_eq!(advising(&model), None);
        hub.answer_with(&model, &answer, None).unwrap();
        hub.model_action(&id, who, None, &request(ModelAction::Take))
            .unwrap();
        assert!(hub.advise(&model, "Now play Village.").is_ok());

        hub.detach_model(&id, person.seat.0, who).unwrap();
        assert!(hub.find(&job.token).is_err() && hub.attached().is_empty());
        assert!(hub.model_seats(&id, who, None).unwrap().is_empty());
    }
}

use super::*;

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

    pub(crate) fn model_paused(&self, id: &str, seat: u8) -> bool {
        self.game(id).is_ok_and(|g| {
            !lock(&g.table).is_over()
                && lock(&g.meta)
                    .model_controls
                    .get(usize::from(seat))
                    .is_some_and(|c| c.controller.is_some())
        })
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

    pub(crate) fn model_seats(
        &self,
        id: &str,
        admin: Option<&str>,
    ) -> Result<Vec<dominion_protocol::api::ModelSeat>, HubError> {
        let game = self.game(id)?;
        let table = lock(&game.table);
        if table.is_over() {
            return Ok(vec![]);
        }
        let meta = lock(&game.meta);
        Ok((0..table.names().len())
            .filter(|&i| model_seat(&meta, i))
            .map(|i| {
                let seat = i as u8;
                let control = meta
                    .model_controls
                    .get(usize::from(seat))
                    .cloned()
                    .unwrap_or_default();
                if admin.is_some() && control.controller.as_deref() == admin {
                    game.touch(PlayerId(seat));
                }
                control.seat(admin, seat, table.names()[i].clone(), || {
                    table.view(PlayerId(seat))
                })
            })
            .collect())
    }

    pub(crate) fn model_action(
        &self,
        id: &str,
        who: &str,
        admin: bool,
        req: &dominion_protocol::api::ModelRequest,
    ) -> Result<(), HubError> {
        use dominion_protocol::api::ModelAction;
        let game = self.game(id)?;
        self.in_time(&game, id)?;
        let mut table = lock(&game.table);
        let seat = PlayerId(req.seat);
        if req.game != id || table.is_over() || !model_seat(&lock(&game.meta), seat.index()) {
            return Err(HubError::Conflict(
                "This model seat is no longer playing".into(),
            ));
        }
        let mut control = lock(&game.meta)
            .model_controls
            .get(seat.index())
            .cloned()
            .unwrap_or_default();
        control.update(who, admin, &req.action)?;
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
}

/// Whether an agent plays seat `i`: one of the server's, one from elsewhere, or one an
/// admin has taken over.
fn model_seat(meta: &Meta, i: usize) -> bool {
    meta.agents.get(i) == Some(&true)
        || meta.remote_agents.get(i) == Some(&true)
        || meta
            .model_controls
            .get(i)
            .is_some_and(|c| c.controller.is_some())
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
            false,
            &request(ModelAction::Prompt {
                text: "Buy Village".into(),
            }),
        )
        .unwrap();
        hub.model_action(&id, "admin", true, &request(ModelAction::Take))
            .unwrap();
        drop(hub);

        let hub = open();
        assert!(hub.model_paused(&id, 0));
        let seat = hub.model_seats(&id, Some("admin")).unwrap().remove(0);
        assert_eq!(seat.prompts, ["Buy Village"]);
        assert!(seat.playing.unwrap().decision.is_some());
        assert!(hub.model_seats(&id, None).unwrap()[0].playing.is_none());
        hub.model_action(&id, "admin", true, &request(ModelAction::Release))
            .unwrap();
        assert!(!hub.model_paused(&id, 0));
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
        assert!(hub.model_seats(&id, None).unwrap().is_empty());

        hub.mark_remote_agent(&seat);
        let listed = hub.model_seats(&id, None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].seat, seat.seat.0);

        let request = |action| ModelRequest {
            game: id.clone(),
            seat: seat.seat.0,
            action,
        };
        hub.model_action(&id, "admin", true, &request(ModelAction::Take))
            .unwrap();
        let decision = hub.view(&seat).unwrap().0.decision.unwrap();
        let answer = Answer {
            decision: decision.id.clone(),
            indices: Some(vec![decision.options[0].index]),
            name: None,
        };
        assert!(hub.answer_with(&seat, &answer, None).is_err());
        assert!(!hub.wait(&seat, Duration::from_millis(20)).await.unwrap());

        hub.model_action(&id, "admin", true, &request(ModelAction::Release))
            .unwrap();
        assert!(hub.wait(&seat, Duration::from_millis(20)).await.unwrap());
        hub.answer_with(&seat, &answer, None).unwrap();
    }
}

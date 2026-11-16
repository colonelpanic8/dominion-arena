use super::*;
use dominion_online::transport::outgoing::{Command, MatchingCriteria};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct HumanTable {
    pub table: TableDetails,
    pub mode: OnlineMode,
    pub owner: i32,
}
impl HumanTable {
    pub fn validate_players(&self, players: &[i32], own: i32) -> Result<(), String> {
        let mut actual = players.to_vec();
        let mut expected: Vec<_> = self.table.players.iter().map(|p| p.id).collect();
        actual.sort_unstable();
        expected.sort_unstable();
        if own != self.owner
            || !(2..=3).contains(&expected.len())
            || expected.windows(2).any(|p| p[0] == p[1])
            || !expected.contains(&own)
            || actual != expected
        {
            return Err(
                "REFUSED: matched player identities differ from the verified roster".into(),
            );
        }
        Ok(())
    }
    pub fn validate(&self, table: &TableDetails, own: i32) -> Result<(), String> {
        self.validate_players(&table.players.iter().map(|p| p.id).collect::<Vec<_>>(), own)?;
        if table.table_id != self.table.table_id
            || !table.bots.is_empty()
            || self.mode == OnlineMode::Bots
            || table.rules.iter().filter(|(r, _)| r.id == 19).count() != 1
            || !table.rules.iter().any(|(r, _)| {
                r.id == 19 && r.value == RuleValue::Boolean(self.mode == OnlineMode::Rated)
            })
        {
            return Err("REFUSED: matched table identity, opponents or rating rule changed".into());
        }
        Ok(())
    }
}
pub(super) fn criteria(
    mode: OnlineMode,
    options: &OnlineMatching,
    searching: bool,
) -> Result<Vec<u8>, String> {
    if mode == OnlineMode::Bots
        || !(2..=3).contains(&options.players)
        || options.timer > 3
        || !(-100..=100).contains(&options.min_rating)
        || !(-100..=100).contains(&options.max_rating)
        || options.min_rating > options.max_rating
    {
        return Err("REJECTED: choose 2–3 players, a standard timer and an ordered rating range within -100..100".into());
    }
    Command::RequestAutomatch {
        criteria: MatchingCriteria {
            bot: 0,
            human: searching,
            min_rating: options.min_rating,
            max_rating: options.max_rating,
            preferences: vec![
                (0, 0),
                (1, if options.players == 2 { 2 } else { 0 }),
                (2, if mode == OnlineMode::Rated { 2 } else { 0 }),
                (3, 0),
            ],
            timers: (0..4)
                .map(|n| {
                    (
                        n,
                        if n == i32::from(options.timer) {
                            n + 1
                        } else {
                            -1
                        },
                    )
                })
                .collect(),
        },
    }
    .encode()
    .map_err(|e| e.to_string())
}
impl Runtime {
    pub(super) async fn start_match(
        &mut self,
        mode: OnlineMode,
        options: OnlineMatching,
    ) -> Result<(), String> {
        self.update.view()?;
        let bytes = criteria(mode, &options, true)?;
        if self.update.reducer.running || self.update.searching || self.search.is_some() {
            return Err("REFUSED: a game or matchmaking search already exists".into());
        }
        self.leave_finished().await?;
        self.human = None;
        self.update.mode = mode;
        self.search = Some(options);
        self.queue_status = None;
        self.update.queue_feedback.clear();
        self.send(bytes).await?;
        while self.queue_status.is_none() && self.human.is_none() {
            self.receive().await?;
        }
        if !self.update.searching && self.human.is_none() {
            self.search = None;
            return Err(format!(
                "Matchmaking was not started: {}",
                self.update.queue_feedback.join("; ")
            ));
        }
        Ok(())
    }
    pub(super) async fn cancel_match(&mut self) -> Result<(), String> {
        if let Some(options) = &self.search {
            let bytes = criteria(self.update.mode, options, false)?;
            self.queue_status = None;
            self.send(bytes).await?;
            while self.queue_status != Some(false) && self.human.is_none() {
                self.receive().await?;
            }
            self.search = None;
        }
        while self.human.is_some() && self.update.searching {
            self.receive().await?;
        }
        Ok(())
    }
}

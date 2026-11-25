//! A private unrated game between two accounts this server hosts: one opens a table
//! (`pair_open`), the other joins it and says it is ready (`pair_join`), then the first
//! says it is ready too and the game starts (`pair_start`). Neither table nor game ever
//! holds anyone else: each side checks the roster before saying it is ready, and from then
//! on the table is a `HumanTable` like a matched one.
use super::*;

/// What a paired table is set up with.
#[derive(Clone, Debug, Default)]
pub struct PairTable {
    pub kingdom: Vec<String>,
    pub landscapes: Vec<String>,
    /// `TimerPresetOptions` ordinal: 0 Reckless, 1 Hasty, 2 Friendly, 3 Patient, 4 Off.
    pub timer: u8,
}

/// A paired table being set up: the other account's player id, and what the server
/// refused meanwhile, which fails the step rather than the connection.
#[derive(Default)]
pub(super) struct Pairing {
    pub partner: i32,
    timer: u8,
    pub refused: Option<String>,
}

/// The highest timer preset a paired table may use: Off.
const TIMER_OFF: u8 = 4;
const TIMER_RULE: i32 = 16;

fn timer_preset(rule: &TableRule) -> Option<i32> {
    match &rule.value {
        RuleValue::Timer(timer) if rule.id == TIMER_RULE => Some(timer.preset),
        _ => None,
    }
}

/// Whether `table` holds exactly `players`, no bots, spectators or players reconnecting,
/// is unrated and lets no one watch, and runs timer preset `timer`.
pub(super) fn check_table(table: &TableDetails, players: &[i32], timer: u8) -> Result<(), String> {
    let mut seated: Vec<_> = table.players.iter().map(|p| p.id).collect();
    let mut expected = players.to_vec();
    seated.sort_unstable();
    expected.sort_unstable();
    if seated != expected
        || !table.bots.is_empty()
        || !table.spectators.is_empty()
        || !table.reconnecting.is_empty()
    {
        return Err(format!(
            "REFUSED: paired table {} holds {seated:?}, not just {expected:?}",
            table.table_id
        ));
    }
    let only = |id: i32, value: RuleValue| {
        let mut rules = table.rules.iter().filter(|(r, _)| r.id == id);
        rules.next().is_some_and(|(r, _)| r.value == value) && rules.next().is_none()
    };
    if !only(5, RuleValue::Group(Group::Nobody)) || !only(19, RuleValue::Boolean(false)) {
        return Err(
            "REFUSED: paired table is not confirmed unrated and closed to spectators".into(),
        );
    }
    let timers: Vec<_> = (table.rules.iter())
        .filter_map(|(r, _)| timer_preset(r))
        .collect();
    // A table without a timer rule has none.
    let off = timer == TIMER_OFF && timers.is_empty();
    if timers != [i32::from(timer)] && !off {
        return Err(format!(
            "REFUSED: paired table's timer is {timers:?}, not preset {timer}"
        ));
    }
    Ok(())
}

impl Runtime {
    fn ready_to_pair(&self, partner: i32) -> Result<(), String> {
        self.update.view()?;
        if self.update.reducer.running {
            return Err("REFUSED: an official game is already running".into());
        }
        if self.update.searching || self.search.is_some() {
            return Err("REFUSED: cancel matchmaking first".into());
        }
        if partner == self.update.reducer.player_id {
            return Err("REFUSED: an account can't play itself".into());
        }
        Ok(())
    }
    /// The table rule for timer preset `timer`, as dominion.games offered it at login.
    fn timer_rule(&self, timer: u8) -> Result<TableRule, String> {
        if timer > TIMER_OFF {
            return Err("REJECTED: choose a timer preset from 0 (Reckless) to 4 (Off)".into());
        }
        let settings = if timer == TIMER_OFF {
            dominion_online::transport::TimerSettings {
                parameters: Vec::new(),
                force_resign: false,
                preset: i32::from(TIMER_OFF),
            }
        } else {
            self.timer_presets
                .iter()
                .find(|(preset, _)| *preset == i32::from(timer))
                .map(|(_, settings)| settings.clone())
                .ok_or_else(|| format!("REFUSED: dominion.games offered no timer preset {timer}"))?
        };
        Ok(TableRule {
            id: TIMER_RULE,
            value: RuleValue::Timer(settings),
        })
    }
    /// Fails the step on anything the server refused meanwhile.
    fn refused(&mut self) -> Result<(), String> {
        match self.pairing.as_mut().and_then(|p| p.refused.take()) {
            Some(reason) => Err(format!("dominion.games refused: {reason}")),
            None => Ok(()),
        }
    }
    async fn receive_pairing(&mut self) -> Result<(), String> {
        self.receive().await?;
        self.refused()
    }
    /// What a step that may have timed out comes to: a step that timed out closes the
    /// connection, like starting any other game.
    pub(super) async fn pairing_step(
        &mut self,
        result: Result<Result<(), String>, tokio::time::error::Elapsed>,
    ) -> Result<(), String> {
        match result {
            Ok(result) => result,
            Err(_) => {
                let error = "setting up the paired table timed out; reconnect explicitly";
                self.fail(error.into()).await;
                Err(error.into())
            }
        }
    }
    /// Leaves a paired table an earlier setup left behind before its game started.
    async fn leave_unstarted_pairing(&mut self) -> Result<(), String> {
        if self.pairing.is_some() || self.human.as_ref().is_some_and(|h| h.paired) {
            self.abandon_pairing().await?;
        }
        Ok(())
    }
    fn begin_pairing(&mut self, partner: i32, timer: u8) {
        self.human = None;
        self.completed_table = None;
        self.update.mode = OnlineMode::Unrated;
        self.pairing = Some(Pairing {
            partner,
            timer,
            refused: None,
        });
    }
    /// Makes this account's player the host of a new private unrated table for player
    /// `partner`, and returns its id. The game starts with `pair_start`.
    pub(super) async fn pair_open(
        &mut self,
        partner: i32,
        options: PairTable,
    ) -> Result<i64, String> {
        self.ready_to_pair(partner)?;
        let mut rules = table_rules(
            &self.update.reducer.catalog,
            2,
            &options.kingdom,
            &options.landscapes,
        )?;
        rules.push(self.timer_rule(options.timer)?);
        self.leave_unstarted_pairing().await?;
        self.leave_finished().await?;
        self.begin_pairing(partner, options.timer);
        let opened = self.open_table(&rules, options.timer).await;
        if opened.is_err() {
            let _ = self.abandon_pairing().await;
        }
        opened
    }
    async fn open_table(&mut self, rules: &[TableRule], timer: u8) -> Result<i64, String> {
        self.send(command::private_table(rules).map_err(|e| e.to_string())?)
            .await?;
        while self.table.is_none() {
            self.receive_pairing().await?;
        }
        let own = self.update.reducer.player_id;
        let table = (self.table.as_ref()).ok_or("official server has not returned a table")?;
        if table.host_id != own {
            return Err("REFUSED: the account does not host the table it opened".into());
        }
        check_table(table, &[own], timer)?;
        for requested in rules.iter().filter(|r| r.id != TIMER_RULE) {
            if !table.rules.iter().any(|(rule, _)| rule == requested) {
                return Err(format!(
                    "REFUSED: server has not confirmed requested rule {}",
                    requested.id
                ));
            }
        }
        Ok(table.table_id)
    }
    /// Joins table `table_id`, which player `host` opened for this account with timer
    /// preset `timer`; once it holds just the two of them, says it is ready.
    pub(super) async fn pair_join(
        &mut self,
        table_id: i64,
        host: i32,
        timer: u8,
    ) -> Result<(), String> {
        self.ready_to_pair(host)?;
        self.leave_unstarted_pairing().await?;
        self.leave_finished().await?;
        self.begin_pairing(host, timer);
        let joined = self.join_table(table_id, host, timer).await;
        if joined.is_err() {
            let _ = self.abandon_pairing().await;
        }
        joined
    }
    async fn join_table(&mut self, table_id: i64, host: i32, timer: u8) -> Result<(), String> {
        self.send_command(Command::JoinTable {
            table: table_id,
            as_player: true,
        })
        .await?;
        while self.table.as_ref().map(|t| t.table_id) != Some(table_id) {
            self.receive_pairing().await?;
        }
        let own = self.update.reducer.player_id;
        let table = (self.table.as_ref()).ok_or("official server has not returned a table")?;
        if table.host_id != host {
            return Err("REFUSED: the paired table has another host".into());
        }
        check_table(table, &[host, own], timer)?;
        self.seal(table.clone())?;
        self.send(command::ready(table_id, true)).await
    }
    /// Keeps `table` as this account's match: from now on its roster is checked like a
    /// matched table's, and it is resumed after a restart.
    fn seal(&mut self, table: TableDetails) -> Result<(), String> {
        let human = HumanTable {
            mode: OnlineMode::Unrated,
            owner: self.update.reducer.player_id,
            table,
            paired: true,
        };
        human.validate(&human.table, human.owner)?;
        self.store
            .save("own-match", &human)
            .map_err(|e| e.to_string())?;
        self.human = Some(human);
        Ok(())
    }
    /// Once player `guest` has joined the table `pair_open` made, says it is ready and
    /// waits for the game to start.
    pub(super) async fn pair_start(&mut self, guest: i32) -> Result<(), String> {
        let started = self.start_table(guest).await;
        if started.is_err() {
            let _ = self.abandon_pairing().await;
        }
        started
    }
    async fn start_table(&mut self, guest: i32) -> Result<(), String> {
        let timer = self
            .pairing
            .as_ref()
            .filter(|p| p.partner == guest)
            .ok_or("REFUSED: no table was opened for that player")?
            .timer;
        self.refused()?;
        let own = self.update.reducer.player_id;
        let seated = |runtime: &Self| {
            (runtime.table.as_ref()).is_some_and(|t| t.players.iter().any(|p| p.id == guest))
        };
        while !seated(self) {
            if self.table.is_none() {
                return Err("REFUSED: the paired table is gone".into());
            }
            self.receive_pairing().await?;
        }
        let table = (self.table.as_ref()).ok_or("official server has not returned a table")?;
        if table.host_id != own {
            return Err("REFUSED: the account no longer hosts the paired table".into());
        }
        check_table(table, &[own, guest], timer)?;
        let table_id = table.table_id;
        self.seal(table.clone())?;
        let previous_game = self.update.reducer.game_id;
        self.send(command::ready(table_id, true)).await?;
        while self.update.reducer.game_id == previous_game {
            self.receive_pairing().await?;
        }
        self.pairing = None;
        Ok(())
    }
    /// Leaves a paired table whose game hasn't started, forgetting it.
    pub(super) async fn abandon_pairing(&mut self) -> Result<(), String> {
        if self.update.reducer.running
            || self.socket.is_none()
            || (self.pairing.is_none() && !self.human.as_ref().is_some_and(|h| h.paired))
        {
            return Ok(());
        }
        let pairing = self.pairing.take().unwrap_or_default();
        if let Some(table) = self.table.as_ref().map(|t| t.table_id) {
            // A table the server has already closed is refused, not fatal.
            self.pairing = Some(Pairing {
                refused: None,
                ..pairing
            });
            let left = self.leave_table(table).await;
            self.pairing = None;
            left?;
        }
        if self.human.take().is_some() {
            self.store.remove("own-match").map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    async fn leave_table(&mut self, table: i64) -> Result<(), String> {
        self.send_command(Command::LeaveTable {
            table,
            player: self.update.reducer.player_id,
        })
        .await?;
        while self.table.is_some() {
            self.receive().await?;
            if self
                .pairing
                .as_mut()
                .and_then(|p| p.refused.take())
                .is_some()
            {
                self.table = None;
                self.bots.clear();
            }
        }
        Ok(())
    }
}

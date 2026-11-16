use crate::{
    Error, Result,
    reducer::Reducer,
    transport::{Message, TableDetails},
};

/// Receiver for callers already authorized to a fixed Arena game and seat.
/// Official-service login and bot-only policies remain in the official runtime.
#[derive(Clone)]
pub struct AttachedReceiver {
    pub reducer: Reducer,
    pub table: Option<TableDetails>,
}
impl AttachedReceiver {
    pub fn new(reducer: Reducer) -> Self {
        Self {
            reducer,
            table: None,
        }
    }

    /// Apply a decoded binary frame atomically, including terminal framing.
    pub fn apply_batch(&mut self, messages: Vec<Message>) -> Result<()> {
        let mut next = self.clone();
        for message in messages {
            match message {
                Message::TableDetails(table) => next.set_table(table)?,
                Message::Snapshot(snapshot) => {
                    if next
                        .reducer
                        .game_id
                        .is_some_and(|id| id != snapshot.game_id)
                    {
                        return Err(Error::Protocol("attached game changed".into()));
                    }
                    next.reducer.replace(snapshot)?;
                    next.validate_table()?;
                }
                Message::Question(question) => {
                    if !next.reducer.running {
                        return Err(Error::Protocol("question outside running game".into()));
                    }
                    next.reducer.ask(question)?;
                }
                Message::Event(event) => {
                    if !next.reducer.running {
                        return Err(Error::Protocol("event outside running game".into()));
                    }
                    next.reducer.apply(event)?;
                }
                Message::GameFinished { table, result, .. } => {
                    if table.table_id != result.table_id {
                        return Err(Error::Protocol("result table mismatch".into()));
                    }
                    next.set_table(table)?;
                    next.reducer.finish(&result)?;
                }
                Message::Pong => {}
                _ => {
                    return Err(Error::Unsupported(
                        "message outside attached receiver profile".into(),
                    ));
                }
            }
        }
        if self.reducer.result.is_some() && next.reducer.running {
            return Err(Error::Protocol(
                "finished attachment needs terminal framing after replacement".into(),
            ));
        }
        *self = next;
        Ok(())
    }
    fn set_table(&mut self, table: TableDetails) -> Result<()> {
        if self
            .table
            .as_ref()
            .is_some_and(|old| old.table_id != table.table_id)
        {
            return Err(Error::Protocol("attached table changed".into()));
        }
        if table.players.len() != table.ready.len() {
            return Err(Error::Protocol("table player/ready arrays differ".into()));
        }
        let mut names = std::collections::BTreeMap::new();
        for player in &table.players {
            if names.insert(player.id, player.name.clone()).is_some() {
                return Err(Error::Protocol("duplicate table player".into()));
            }
        }
        self.reducer.names = names;
        self.table = Some(table);
        self.validate_table()
    }
    fn validate_table(&self) -> Result<()> {
        if let (Some(table), Ok(state)) = (&self.table, self.reducer.state()) {
            if !table
                .players
                .iter()
                .map(|p| p.id)
                .eq(state.players.iter().map(|p| p.player_id))
            {
                return Err(Error::Protocol("table/snapshot players differ".into()));
            }
            if self.reducer.seat().is_none() {
                return Err(Error::Protocol("attached player is absent".into()));
            }
        }
        Ok(())
    }
}

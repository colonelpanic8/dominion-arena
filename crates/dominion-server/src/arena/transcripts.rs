use super::{Arena, Error, Login, Result};
use crate::house::Job;
use crate::hub::Seat;
use dominion_protocol::lobby::{OFFICIAL_GAME_PREFIX, official_game_id};
use dominion_protocol::transcript::{TranscriptInput, TranscriptPage};
use tokio::sync::watch;

fn check(input: &TranscriptInput) -> Result<()> {
    if input.attempt.is_empty()
        || input.attempt.len() > 96
        || !input
            .attempt
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        || input.block.is_empty()
        || input.block.len() > 256
        || input.text.len() > 8192
        || input.offset > i64::MAX as u64
    {
        return Err(Error::BadRequest("invalid transcript entry".into()));
    }
    Ok(())
}

/// Whether `game` is a dominion.games game's id here ([`official_game_id`]).
pub fn is_official(game: &str) -> bool {
    game.starts_with(OFFICIAL_GAME_PREFIX)
}

impl Arena {
    pub fn append_transcript(&self, seat: &Seat, input: &TranscriptInput) -> Result<()> {
        let meta = self.hub.meta(&seat.game)?;
        let owner = meta
            .owners
            .get(seat.seat.index())
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                Error::Forbidden("only a server agent can publish its transcript".into())
            })?;
        let user = self.user(owner)?;
        if !matches!(user.login, Login::House) {
            return Err(Error::Forbidden(
                "only a server agent can publish its transcript".into(),
            ));
        }
        check(input)?;
        self.db
            .append_transcript(&seat.game, seat.seat.0, &user.name, input)?;
        self.hub.transcript_changed(&seat.game)?;
        Ok(())
    }

    /// Adds to the transcript of the agent `job` sends to play on dominion.games. Everyone
    /// may read it, whether the game is rated or not.
    pub fn append_official_transcript(&self, job: &Job, input: &TranscriptInput) -> Result<()> {
        if job.game_id.is_empty() {
            return Err(Error::Forbidden("the agent has no game yet".into()));
        }
        check(input)?;
        let game = official_game_id(&job.game_id);
        self.db
            .append_transcript(&game, job.seat, &job.agent, input)?;
        self.official_transcripts.send_modify(|n| *n += 1);
        Ok(())
    }

    pub fn transcript(&self, game: &str, after: u64) -> Result<TranscriptPage> {
        if !is_official(game) {
            self.hub.meta(game)?;
        }
        self.db.transcript(game, after)
    }

    /// Bumped whenever a dominion.games game's transcript grows.
    pub fn official_transcript_changes(&self) -> watch::Receiver<u64> {
        self.official_transcripts.subscribe()
    }
}

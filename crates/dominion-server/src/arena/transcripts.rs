use super::{Arena, Error, Login, Result};
use crate::hub::Seat;
use dominion_protocol::transcript::{TranscriptInput, TranscriptPage};

impl Arena {
    pub fn append_transcript(&self, seat: &Seat, input: &TranscriptInput) -> Result<()> {
        let meta = self.hub.meta(&seat.game)?;
        if meta.rated {
            return Err(Error::Forbidden(
                "live transcripts are available in unrated games".into(),
            ));
        }
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
        self.db
            .append_transcript(&seat.game, seat.seat.0, &user.name, input)?;
        self.hub.transcript_changed(&seat.game)?;
        Ok(())
    }

    pub fn transcript(&self, game: &str, after: u64) -> Result<TranscriptPage> {
        if self.hub.meta(game)?.rated {
            return Err(Error::Forbidden(
                "live transcripts are available in unrated games".into(),
            ));
        }
        self.db.transcript(game, after)
    }
}

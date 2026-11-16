//! Engine-free codecs for an authorized, fixed-game Arena attachment.
mod encode;
mod lifecycle;
mod receiver;
pub use lifecycle::{LifecycleCommand, Secret, decode_lifecycle_command, encode_probe};
pub use receiver::AttachedReceiver;

use crate::{Error, Result, transport::Message, wire::Reader};

pub const MAX_COMMAND_BYTES: usize = 64 * 1024;
pub const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientCommand {
    AnswerQuestion {
        question: i32,
        values: Vec<i32>,
        autoplay: bool,
    },
    ReconnectGame,
    Resign,
    Ping,
    AnimationCompensationAck,
}

/// Decode exactly one command. Commands have no message counter or length envelope.
pub fn decode_command(bytes: &[u8]) -> Result<ClientCommand> {
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(Error::Protocol("command exceeds size limit".into()));
    }
    let mut r = Reader::new(bytes);
    let command = match r.int()? {
        35 => ClientCommand::ReconnectGame,
        37 => {
            let question = r.int()?;
            if question < 0 {
                return Err(Error::Protocol("negative question index".into()));
            }
            ClientCommand::AnswerQuestion {
                question,
                values: r.ints()?,
                autoplay: r.boolean()?,
            }
        }
        38 => ClientCommand::Resign,
        44 => ClientCommand::Ping,
        51 => ClientCommand::AnimationCompensationAck,
        opcode => return Err(Error::Unsupported(format!("client opcode {opcode}"))),
    };
    r.finish()?;
    Ok(command)
}

/// One encoder per socket. A failed batch poisons it without consuming counters.
#[derive(Default)]
pub struct ServerEncoder {
    next_counter: i32,
    poisoned: bool,
}
impl ServerEncoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn encode_batch(&mut self, messages: &[Message]) -> Result<Vec<u8>> {
        if self.poisoned {
            return Err(Error::Protocol("encoder is stopped".into()));
        }
        let result = encode::batch(self.next_counter, messages);
        match result {
            Ok((bytes, next)) => {
                self.next_counter = next;
                Ok(bytes)
            }
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_batch_never_consumes_counters_and_stops_encoder() {
        let mut encoder = ServerEncoder::new();
        assert!(
            encoder
                .encode_batch(&[Message::Pong, Message::LobbyNotice])
                .is_err()
        );
        assert_eq!(encoder.next_counter, 0);
        assert!(encoder.poisoned);
    }
    #[test]
    fn closes_before_counter_overflow_including_mid_batch() {
        let mut encoder = ServerEncoder {
            next_counter: i32::MAX - 1,
            poisoned: false,
        };
        assert!(
            encoder
                .encode_batch(&[Message::Pong, Message::Pong])
                .is_err()
        );
        assert_eq!(encoder.next_counter, i32::MAX - 1);
        let mut encoder = ServerEncoder {
            next_counter: i32::MAX,
            poisoned: false,
        };
        assert!(encoder.encode_batch(&[Message::Pong]).is_err());
    }
}

//! Selected host operations. Account and commerce serializers are not host authority.
use super::{ClientCommand, MAX_COMMAND_BYTES, decode_command};
use crate::{
    Error, Result,
    transport::{Preference, ServerState, TableRule},
    wire::{Reader, Writer},
};

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);
impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum LifecycleCommand {
    Game(ClientCommand),
    Login {
        username: String,
        password: Secret,
        version: String,
        language: i32,
    },
    LoginWithSession {
        player: i32,
        version: String,
        session: Secret,
        language: i32,
    },
    ReconnectSession {
        player: i32,
        version: String,
        session: Secret,
    },
    LoginWithCode {
        code: Secret,
        version: String,
    },
    RequestServerState {
        version: String,
        player: Option<i32>,
        session: Option<Secret>,
    },
    Logout,
    SendChat {
        receiver: String,
        sender: String,
        text: String,
    },
    JoinTable {
        table: i64,
        as_player: bool,
    },
    ChangeTableRule {
        table: i64,
        rule: TableRule,
    },
    ChangeHost {
        table: i64,
        player: i32,
    },
    LeaveTable {
        table: i64,
        player: i32,
    },
    SitDown {
        table: i64,
    },
    StandUp {
        table: i64,
        player: i32,
    },
    NewTableRequest {
        rules: Vec<TableRule>,
    },
    ReadyToStart {
        table: i64,
        ready: bool,
    },
    RequestUpdate {
        kind: i32,
    },
    ChangeUserProperty {
        preference: Preference,
    },
    PersistUserProperty {
        preference: Preference,
    },
    RequestPlayerAvatars {
        players: Vec<i32>,
    },
    Unsupported {
        opcode: i32,
    },
}
fn secret(r: &mut Reader<'_>) -> Result<Secret> {
    Ok(Secret(r.string()?))
}
fn finite_rule(rule: &TableRule) -> Result<()> {
    use crate::transport::RuleValue;
    if matches!(&rule.value, RuleValue::Double(v) if !v.is_finite()) {
        return Err(Error::Protocol("non-finite table rule".into()));
    }
    Ok(())
}
pub fn decode_lifecycle_command(bytes: &[u8]) -> Result<LifecycleCommand> {
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(Error::Protocol("command exceeds size limit".into()));
    }
    let mut r = Reader::new(bytes);
    use LifecycleCommand::*;
    let opcode = r.int()?;
    let command = match opcode {
        0 => SendChat {
            receiver: r.string()?,
            sender: r.string()?,
            text: r.string()?,
        },
        1 => Login {
            username: r.string()?,
            password: secret(&mut r)?,
            version: r.string()?,
            language: r.int()?,
        },
        2 => JoinTable {
            table: r.long()?,
            as_player: r.boolean()?,
        },
        4 => {
            let table = r.long()?;
            let rule = TableRule::parse(&mut r)?;
            finite_rule(&rule)?;
            ChangeTableRule { table, rule }
        }
        5 => ChangeHost {
            table: r.long()?,
            player: r.int()?,
        },
        6 => LeaveTable {
            table: r.long()?,
            player: r.int()?,
        },
        7 => SitDown { table: r.long()? },
        8 => StandUp {
            table: r.long()?,
            player: r.int()?,
        },
        9 => {
            let count = r.count()?;
            let mut rules = Vec::with_capacity(count);
            for _ in 0..count {
                let rule = TableRule::parse(&mut r)?;
                finite_rule(&rule)?;
                rules.push(rule);
            }
            NewTableRequest { rules }
        }
        10 => ReadyToStart {
            table: r.long()?,
            ready: r.boolean()?,
        },
        11 => {
            let kind = r.int()?;
            if !(0..4).contains(&kind) {
                return Err(Error::Protocol("invalid update type".into()));
            }
            RequestUpdate { kind }
        }
        12 => ChangeUserProperty {
            preference: Preference::parse(&mut r)?,
        },
        13 => LoginWithCode {
            code: secret(&mut r)?,
            version: r.string()?,
        },
        24 => LoginWithSession {
            player: r.int()?,
            version: r.string()?,
            session: secret(&mut r)?,
            language: r.int()?,
        },
        32 => Logout,
        35 | 37 | 38 | 44 | 51 => return decode_command(bytes).map(Game),
        45 => RequestServerState {
            version: r.string()?,
            player: if r.boolean()? { Some(r.int()?) } else { None },
            session: if r.boolean()? {
                Some(secret(&mut r)?)
            } else {
                None
            },
        },
        46 => ReconnectSession {
            player: r.int()?,
            version: r.string()?,
            session: secret(&mut r)?,
        },
        53 => PersistUserProperty {
            preference: Preference::parse(&mut r)?,
        },
        60 => RequestPlayerAvatars { players: r.ints()? },
        opcode if (0..=67).contains(&opcode) => {
            excluded_payload(opcode, &mut r)?;
            Unsupported { opcode }
        }
        _ => {
            return Err(Error::Unsupported(format!(
                "unknown client opcode {opcode}"
            )));
        }
    };
    r.finish()?;
    Ok(command)
}

/// A probe deliberately has no normal server-message opcode.
pub fn encode_probe(counter: i32, state: ServerState) -> Result<Vec<u8>> {
    if counter < 0 || counter == i32::MAX {
        return Err(Error::Protocol("invalid probe counter".into()));
    }
    let mut w = Writer::new();
    w.int(counter);
    w.int(match state {
        ServerState::AskingReconnect => 0,
        ServerState::Normal => 1,
        ServerState::NoNewGames => 2,
        ServerState::BadClientVersion => 3,
        ServerState::Down => 4,
    });
    Ok(w.finish())
}

// Validate the excluded producer layouts without retaining credential/account data.
fn excluded_payload(opcode: i32, r: &mut Reader<'_>) -> Result<()> {
    fn fields(schema: &str, r: &mut Reader<'_>) -> Result<()> {
        for c in schema.chars() {
            match c {
                'i' => {
                    r.int()?;
                }
                'l' => {
                    r.long()?;
                }
                's' => {
                    r.string()?;
                }
                'b' => {
                    r.boolean()?;
                }
                'd' => {
                    if !r.double()?.is_finite() {
                        return Err(Error::Protocol("non-finite command field".into()));
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }
    match opcode {
        3 => fields("li", r),
        14 => fields("sssi", r),
        15 | 16 => fields("ss", r),
        17 => fields("il", r),
        18 => fields("illi", r),
        19 => fields("sssisis", r),
        20 => fields("sssb", r),
        21 | 30 => fields("isbb", r),
        22 => {
            fields("ibii", r)?;
            for _ in 0..2 {
                for _ in 0..r.count()? {
                    fields("ii", r)?;
                }
            }
            Ok(())
        }
        23 | 27 | 31 => fields("l", r),
        25 => fields("li", r),
        26 => {
            for _ in 0..r.count()? {
                r.long()?;
            }
            Ok(())
        }
        28 => fields("ib", r),
        29 | 36 | 47 | 49 | 56 | 63 => Ok(()),
        33 => fields("is", r),
        34 => fields("isi", r),
        39 => fields("i", r),
        40..=43 => fields("ii", r),
        48 => {
            r.int()?;
            r.ints()?;
            Ok(())
        }
        50 => {
            r.string()?;
            for _ in 0..r.count()? {
                fields("ss", r)?;
            }
            Ok(())
        }
        52 => fields("liiii", r),
        54 => fields("dss", r),
        55 => fields("dsi", r),
        57 => fields("iiii", r),
        58 => fields("ssis", r),
        59 => fields("isi", r),
        61 => fields("iii", r),
        62 | 65 | 66 => fields("s", r),
        64 => fields("ss", r),
        67 => fields("sdsd", r),
        _ => Err(Error::Unsupported(format!("client opcode {opcode}"))),
    }
}

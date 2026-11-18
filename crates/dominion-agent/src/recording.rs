//! A recorded `dominion/events@1` stream as bytes: a [`Header`] record, then every event
//! in order, each a record of the file's [`Encoding`], which its name ends with
//! ([`Encoding::suffix`]). Records are only ever appended, so a write that never finished
//! leaves at most one record cut short at the end, which reading ignores.

use dominion_protocol::events::{Event, Header, SCHEMA, Viewer};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, Write};
use std::str::FromStr;

/// How a stream's records are written. Any self-describing serde format can carry one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    /// One JSON object per line.
    #[default]
    #[serde(rename = "jsonl")]
    JsonLines,
    /// A CBOR sequence (RFC 8742): one CBOR item per record, back to back.
    #[serde(rename = "cbor")]
    Cbor,
}

/// The next record of some bytes.
enum Next<T> {
    /// It, and how many bytes it took.
    Record(T, usize),
    /// Nothing left, or only a record cut short.
    End,
}

impl Encoding {
    pub const ALL: [Encoding; 2] = [Encoding::JsonLines, Encoding::Cbor];

    /// Its name in settings.
    pub fn name(self) -> &'static str {
        match self {
            Encoding::JsonLines => "jsonl",
            Encoding::Cbor => "cbor",
        }
    }

    /// What the name of a stream file in this encoding ends with.
    pub fn suffix(self) -> &'static str {
        match self {
            Encoding::JsonLines => ".events.jsonl",
            Encoding::Cbor => ".events.cbor",
        }
    }

    pub fn media_type(self) -> &'static str {
        match self {
            Encoding::JsonLines => "application/jsonl",
            Encoding::Cbor => "application/cbor-seq",
        }
    }

    /// The encoding of the stream file named `name`.
    pub fn of_file(name: &str) -> Option<Encoding> {
        Encoding::ALL
            .into_iter()
            .find(|e| name.ends_with(e.suffix()))
    }

    /// Writes one record.
    pub fn write(self, out: &mut impl Write, record: &impl Serialize) -> io::Result<()> {
        match self {
            Encoding::JsonLines => {
                serde_json::to_writer(&mut *out, record)?;
                out.write_all(b"\n")
            }
            Encoding::Cbor => ciborium::into_writer(record, out).map_err(|e| match e {
                ciborium::ser::Error::Io(e) => e,
                ciborium::ser::Error::Value(e) => io::Error::other(e),
            }),
        }
    }

    fn next<T: DeserializeOwned>(self, bytes: &[u8]) -> Result<Next<T>, String> {
        if bytes.is_empty() {
            return Ok(Next::End);
        }
        match self {
            Encoding::JsonLines => {
                let Some(end) = bytes.iter().position(|&b| b == b'\n') else {
                    return Ok(Next::End);
                };
                let record = serde_json::from_slice(&bytes[..end]).map_err(|e| e.to_string())?;
                Ok(Next::Record(record, end + 1))
            }
            Encoding::Cbor => {
                let mut rest = bytes;
                match ciborium::from_reader(&mut rest) {
                    Ok(record) => Ok(Next::Record(record, bytes.len() - rest.len())),
                    Err(ciborium::de::Error::Io(_)) => Ok(Next::End),
                    Err(e) => Err(e.to_string()),
                }
            }
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Encoding {
    type Err = String;

    fn from_str(s: &str) -> Result<Encoding, String> {
        Encoding::ALL
            .into_iter()
            .find(|e| e.name() == s)
            .ok_or_else(|| format!("unknown events encoding {s:?}: jsonl or cbor"))
    }
}

/// The first record of the full stream of game `game`.
pub fn header(game: &str) -> Header {
    Header {
        schema: SCHEMA.into(),
        viewer: Viewer::Full,
        game: Some(game.into()),
    }
}

/// A stream read back.
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub header: Header,
    pub events: Vec<Event>,
    /// Where each record ends: the header's, then each event's.
    ends: Vec<usize>,
}

impl Recording {
    /// How many bytes the header and the first `n` events take, if it has that many.
    pub fn len_of(&self, n: usize) -> Option<usize> {
        self.ends.get(n).copied()
    }
}

/// Reads a full stream's header and its events, at most `limit` of them: what follows
/// them is not read. An event out of sequence is an error.
pub fn read(encoding: Encoding, bytes: &[u8], limit: Option<usize>) -> Result<Recording, String> {
    let Next::Record(header, mut at) = encoding.next::<Header>(bytes)? else {
        return Err("no header".into());
    };
    let header: Header = header;
    if header.schema != SCHEMA || header.viewer != Viewer::Full {
        return Err(format!("not a full {SCHEMA} stream"));
    }
    let mut recording = Recording {
        header,
        events: vec![],
        ends: vec![at],
    };
    while limit.is_none_or(|n| recording.events.len() < n) {
        let Next::Record(event, len) = encoding.next::<Event>(&bytes[at..])? else {
            break;
        };
        let event: Event = event;
        if event.seq != recording.events.len() as u64 {
            return Err(format!(
                "event {} is numbered {}",
                recording.events.len(),
                event.seq
            ));
        }
        at += len;
        recording.events.push(event);
        recording.ends.push(at);
    }
    Ok(recording)
}

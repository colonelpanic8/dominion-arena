//! Serde types for `dominion/events@1`: a game of Dominion as a stream of events, and the
//! [`Table`] they describe. The table after event *n* is the fold of events `0..=n`
//! ([`Table::apply`]); no rules engine is needed to follow a game.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

pub const SCHEMA: &str = "dominion/events@1";

/// The first line of a stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub schema: String,
    pub viewer: Viewer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
}

/// Whose eyes a stream is written for: which cards' faces it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Viewer {
    /// Every face, every shuffle's order.
    Full,
    /// What everyone at the table sees.
    Public,
    Seat(u8),
}

/// A card as the stream's viewer sees it: `Name#handle`, or `?` face down.
///
/// A handle names one physical card for as long as the viewer keeps seeing its face. In
/// a full stream that is always.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CardRef {
    Hidden,
    Seen { name: String, handle: u32 },
}

impl CardRef {
    pub fn seen(name: impl Into<String>, handle: u32) -> CardRef {
        CardRef::Seen {
            name: name.into(),
            handle,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            CardRef::Hidden => None,
            CardRef::Seen { name, .. } => Some(name),
        }
    }

    pub fn handle(&self) -> Option<u32> {
        match self {
            CardRef::Hidden => None,
            CardRef::Seen { handle, .. } => Some(*handle),
        }
    }
}

impl fmt::Display for CardRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CardRef::Hidden => f.write_str("?"),
            CardRef::Seen { name, handle } => write!(f, "{name}#{handle}"),
        }
    }
}

impl FromStr for CardRef {
    type Err = String;

    fn from_str(s: &str) -> Result<CardRef, String> {
        if s == "?" {
            return Ok(CardRef::Hidden);
        }
        let bad = || format!("{s:?} is neither \"?\" nor \"Name#handle\"");
        let (name, handle) = s.rsplit_once('#').ok_or_else(bad)?;
        if name.is_empty() {
            return Err(bad());
        }
        Ok(CardRef::Seen {
            name: name.to_string(),
            handle: handle.parse().map_err(|_| bad())?,
        })
    }
}

/// A player's zone, by its name in a [`Zone`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SeatZone {
    Deck,
    Hand,
    Discard,
    Play,
    /// Set aside.
    Aside,
    /// Taken off the deck to be looked at or revealed.
    Looking,
    Tavern,
    Island,
    NativeVillage,
    Exile,
}

impl SeatZone {
    pub const ALL: [SeatZone; 10] = [
        SeatZone::Deck,
        SeatZone::Hand,
        SeatZone::Discard,
        SeatZone::Play,
        SeatZone::Aside,
        SeatZone::Looking,
        SeatZone::Tavern,
        SeatZone::Island,
        SeatZone::NativeVillage,
        SeatZone::Exile,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SeatZone::Deck => "deck",
            SeatZone::Hand => "hand",
            SeatZone::Discard => "discard",
            SeatZone::Play => "play",
            SeatZone::Aside => "aside",
            SeatZone::Looking => "looking",
            SeatZone::Tavern => "tavern",
            SeatZone::Island => "island",
            SeatZone::NativeVillage => "native-village",
            SeatZone::Exile => "exile",
        }
    }
}

/// Where cards are: `<seat>/<zone>`, `pile/<card>`, `aside/<card>` or `trash`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Zone {
    Seat(u8, SeatZone),
    /// A pile, by the card or randomizer it is named after.
    Pile(String),
    /// Cards set aside for the game by a card: the Black Market deck, Way of the Mouse's
    /// card.
    Aside(String),
    Trash,
}

impl Zone {
    /// Whether its cards have positions: `cards[0]` is the top. The others list their
    /// cards in the order they arrived.
    pub fn ordered(&self) -> bool {
        matches!(
            self,
            Zone::Seat(_, SeatZone::Deck | SeatZone::Discard)
                | Zone::Pile(_)
                | Zone::Aside(_)
                | Zone::Trash
        )
    }

    /// Whether it is there in every game, empty or not: a seat's zones and the trash. A
    /// pile exists once the game has it.
    fn standing(&self) -> bool {
        matches!(self, Zone::Seat(..) | Zone::Trash)
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Zone::Seat(seat, zone) => write!(f, "{seat}/{}", zone.name()),
            Zone::Pile(card) => write!(f, "pile/{card}"),
            Zone::Aside(card) => write!(f, "aside/{card}"),
            Zone::Trash => f.write_str("trash"),
        }
    }
}

impl FromStr for Zone {
    type Err = String;

    fn from_str(s: &str) -> Result<Zone, String> {
        let bad = || format!("{s:?} is no zone");
        if s == "trash" {
            return Ok(Zone::Trash);
        }
        let (owner, rest) = s.split_once('/').ok_or_else(bad)?;
        if owner == "pile" || owner == "aside" {
            if rest.is_empty() {
                return Err(bad());
            }
            return Ok(match owner {
                "pile" => Zone::Pile(rest.to_string()),
                _ => Zone::Aside(rest.to_string()),
            });
        }
        let seat = owner.parse().map_err(|_| bad())?;
        SeatZone::ALL
            .into_iter()
            .find(|z| z.name() == rest)
            .map(|z| Zone::Seat(seat, z))
            .ok_or_else(bad)
    }
}

macro_rules! serde_as_string {
    ($($ty:ty),*) => {$(
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let text = String::deserialize(d)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    )*};
}

serde_as_string!(CardRef, Zone);

/// Where cards arrive in an ordered zone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum At {
    #[default]
    Top,
    Bottom,
    /// Under that many cards.
    Under(u32),
}

impl At {
    fn is_top(&self) -> bool {
        *self == At::Top
    }
}

/// Cards changing zone. They leave `from` and arrive in `to` one after the other, in the
/// order listed (so of cards put on top, the last listed ends up on top); a
/// [`What::Shuffle`]'s cards arrive together, in the order listed, top first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Move {
    /// Whose action it is.
    pub player: u8,
    pub cards: Vec<CardRef>,
    pub from: Zone,
    pub to: Zone,
    /// Where they arrive, when `to` is ordered.
    #[serde(default, skip_serializing_if = "At::is_top")]
    pub at: At,
    /// For an ordered `from`: how many cards lay over each card as it left, when any
    /// did. Absent: each came off the top.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pos: Vec<u32>,
    /// Into a set-aside zone or `looking`: whether the cards lie face up (everyone sees
    /// them) or face down (their owner does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<Face>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Face {
    Up,
    Down,
}

/// A card is played: its instructions are followed. It goes into play when `from` and
/// `to` say so; a card played again, or played where it lies, moves nowhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Play {
    pub player: u8,
    pub card: CardRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Zone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Zone>,
    /// For an ordered `from`: how many cards lay over it, when any did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<u32>,
}

/// A choice announced to everyone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub player: u8,
    /// The card that asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub choices: Vec<String>,
}

/// What kind of move a [`Move`] is: the word the rules use for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Verb {
    Draw,
    Discard,
    Gain,
    Trash,
    Topdeck,
    Bottomdeck,
    Insert,
    Reveal,
    Look,
    SetAside,
    Exile,
    Put,
    Return,
    Exchange,
    Pass,
    Shuffle,
    Rotate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Start,
    Action,
    Buy,
    Night,
    Cleanup,
    End,
}

/// A turn beginning. The table's phase becomes `start` and the turn's counters 1 Action,
/// 1 Buy, $0, no Potions and a `next-hand` of 5.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStart {
    pub player: u8,
    /// Which of that player's turns it is, from 1, turns of every kind counted.
    pub number: u32,
    /// The card that gave it, for an extra turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(default)]
    pub kind: TurnKind,
    /// The extra turn waiting it is: it waits no more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_turn: Option<u32>,
    #[serde(default, skip_serializing_if = "TurnLimits::is_none")]
    pub limits: TurnLimits,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TurnKind {
    #[default]
    Regular,
    /// Given by a card: it doesn't count for the tiebreaker.
    Extra,
    /// One of the round played after the game has ended (Fleet).
    Final,
}

/// What a turn forbids beyond the rules of every turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnLimits {
    /// No buying cards (Mission); Events and Projects may still be bought.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_card_buys: bool,
    /// At most this many plays from the hand (Voyage); the turn's `hand-plays` counts them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_plays: Option<u32>,
}

impl TurnLimits {
    fn is_none(&self) -> bool {
        *self == TurnLimits::default()
    }
}

/// When a lasting effect ends, as its players are told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Until {
    /// At the end of this turn.
    EndOfTurn,
    /// When that seat's next turn starts.
    StartOfTurn(u8),
    /// When the extra turn it keeps its card in play for starts, or doesn't happen.
    ExtraTurn,
    /// When something happens that it waits for.
    Removed,
}

/// Something that outlives the play that made it: a Duration's later part, a "this turn"
/// or "until your next turn" effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    /// Names it from when it begins to when it ends.
    pub id: u32,
    /// The card (or landscape, Boon, State) it is of.
    pub card: String,
    /// The physical card it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CardRef>,
    pub owner: u8,
    pub until: Until,
    /// Its card left play: it works to the end of this turn and ends then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ending: bool,
    /// Cards it keeps in play past Clean-up.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keeps: Vec<CardRef>,
    /// Cards it holds set aside.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<CardRef>,
    /// Tokens on its card (Garrison).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
}

/// Why an effect ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectEnd {
    /// An ability removed it: as a rule its own, resolving (a Duration's at the start of
    /// the turn).
    Resolved,
    /// When it said it would end.
    Expired,
    /// Its card had left play: it ended with the turn.
    LeftPlay,
    /// The extra turn it kept its card in play for began, was skipped or was lost.
    ExtraTurn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtraLimit {
    /// It always happens (Seize the Day).
    None,
    /// Not as a third turn in a row: it is lost then.
    ThirdInARow,
}

/// An extra turn waiting to be taken.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraTurn {
    pub id: u32,
    pub player: u8,
    /// The card that gave it.
    pub card: String,
    pub limit: ExtraLimit,
    /// The effect that keeps its card in play until it is taken (Outpost, Voyage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<u32>,
}

/// An extra turn queued: after the others waiting, or before them if `front`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraTurnQueued {
    pub id: u32,
    pub player: u8,
    pub card: String,
    pub limit: ExtraLimit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub front: bool,
}

/// The end condition is met: the game ends once the seats of `final_round` have taken a
/// turn each, in that order (Fleet; usually none).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ending {
    /// `provinces`, `colonies`, `piles` or `turn-limit`.
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub final_round: Vec<u8>,
}

/// A counter changing by `delta` to `value`.
///
/// `counter` names it: the turn's and the game's by name (`actions`, `buys`, `coins`,
/// `potions`, `trade-route-mat`, `sun`), a seat's as `<seat>/<name>` (`coffers`,
/// `villagers`, `favors`, `debt`, `vp-tokens`, `pirate-ship`), a pile's as
/// `pile/<card>/<name>` (`vp`, `debt`, `embargo`, `trade-route`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterChange {
    pub counter: String,
    pub delta: i64,
    pub value: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    /// What decided the game: `dominion-engine`, `dominion.games`.
    pub implementation: String,
    pub version: String,
}

/// The game before anything happens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub rules: Rules,
    /// The players' names, in turn order.
    pub seats: Vec<String>,
    pub kingdom: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landscapes: Vec<String>,
    pub table: Table,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatResult {
    pub seat: u8,
    pub vp: i32,
    pub turns: u32,
    /// From 1; ties share a rank.
    pub rank: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resigned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameEnd {
    /// `provinces`, `colonies`, `piles`, `turn-limit` or `resigned`.
    pub reason: String,
    pub result: Vec<SeatResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum What {
    Setup(Box<Setup>),
    Play(Play),
    /// A card or landscape is bought: paid for, before it is gained or does anything.
    Buy {
        player: u8,
        card: String,
    },
    /// An ability of `card` begins to resolve for `player`.
    Resolve {
        player: u8,
        card: String,
    },
    /// `player` names a card.
    Name {
        player: u8,
        card: String,
    },
    /// `player` plays `card` using `way` instead of following its instructions.
    Way {
        player: u8,
        card: String,
        way: String,
    },
    /// `attack` doesn't affect `player`, because of `card`.
    Unaffected {
        player: u8,
        attack: String,
        card: String,
    },
    /// `player` calls `card` from their Tavern mat.
    Call {
        player: u8,
        card: String,
    },
    /// What `player` chose, said to everyone: a card's modes, a number.
    Choose(Choice),
    /// A turn `player` would have taken doesn't happen: skipped (Lich), or `lost` as a
    /// third turn in a row. `extra` is the card that gave it, for an extra turn, and
    /// `extraTurn` the extra turn waiting it was.
    TurnSkipped {
        player: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        lost: bool,
        #[serde(default)]
        kind: TurnKind,
        #[serde(default, rename = "extraTurn", skip_serializing_if = "Option::is_none")]
        extra_turn: Option<u32>,
    },
    Draw(Move),
    Discard(Move),
    Gain(Move),
    Trash(Move),
    Topdeck(Move),
    Bottomdeck(Move),
    Insert(Move),
    Reveal(Move),
    Look(Move),
    SetAside(Move),
    Exile(Move),
    Put(Move),
    Return(Move),
    Exchange(Move),
    Pass(Move),
    Shuffle(Move),
    /// From the top of a pile to its bottom.
    Rotate(Move),
    /// Cards come into the game in `to`, each on top of the one before. A pile that
    /// wasn't there is.
    Add {
        cards: Vec<CardRef>,
        to: Zone,
    },
    /// `card` is now a card called `name`, with the same handle.
    Rename {
        card: CardRef,
        name: String,
    },
    Turn(TurnStart),
    Phase {
        phase: Phase,
    },
    Counter(CounterChange),
    GameEnd(GameEnd),
    /// An effect begins.
    Effect(Effect),
    /// The effect with this `id` is now as given: what it keeps or holds, when it ends.
    EffectChange(Effect),
    EffectEnd {
        id: u32,
        why: EffectEnd,
    },
    ExtraTurn(ExtraTurnQueued),
    /// The extra turn waiting with this `id` won't happen: the effect it came with ended.
    ExtraTurnRemoved {
        id: u32,
    },
    /// `player`'s turn is over: nobody's turn is in progress, and its counters are gone.
    TurnEnd {
        player: u8,
    },
    GameEnding(Ending),
}

impl What {
    pub fn moved(verb: Verb, m: Move) -> What {
        match verb {
            Verb::Draw => What::Draw(m),
            Verb::Discard => What::Discard(m),
            Verb::Gain => What::Gain(m),
            Verb::Trash => What::Trash(m),
            Verb::Topdeck => What::Topdeck(m),
            Verb::Bottomdeck => What::Bottomdeck(m),
            Verb::Insert => What::Insert(m),
            Verb::Reveal => What::Reveal(m),
            Verb::Look => What::Look(m),
            Verb::SetAside => What::SetAside(m),
            Verb::Exile => What::Exile(m),
            Verb::Put => What::Put(m),
            Verb::Return => What::Return(m),
            Verb::Exchange => What::Exchange(m),
            Verb::Pass => What::Pass(m),
            Verb::Shuffle => What::Shuffle(m),
            Verb::Rotate => What::Rotate(m),
        }
    }

    pub fn as_move_mut(&mut self) -> Option<&mut Move> {
        match self {
            What::Draw(m)
            | What::Discard(m)
            | What::Gain(m)
            | What::Trash(m)
            | What::Topdeck(m)
            | What::Bottomdeck(m)
            | What::Insert(m)
            | What::Reveal(m)
            | What::Look(m)
            | What::SetAside(m)
            | What::Exile(m)
            | What::Put(m)
            | What::Return(m)
            | What::Exchange(m)
            | What::Pass(m)
            | What::Shuffle(m)
            | What::Rotate(m) => Some(m),
            _ => None,
        }
    }

    /// The move this is, if it is one.
    pub fn as_move(&self) -> Option<(Verb, &Move)> {
        Some(match self {
            What::Draw(m) => (Verb::Draw, m),
            What::Discard(m) => (Verb::Discard, m),
            What::Gain(m) => (Verb::Gain, m),
            What::Trash(m) => (Verb::Trash, m),
            What::Topdeck(m) => (Verb::Topdeck, m),
            What::Bottomdeck(m) => (Verb::Bottomdeck, m),
            What::Insert(m) => (Verb::Insert, m),
            What::Reveal(m) => (Verb::Reveal, m),
            What::Look(m) => (Verb::Look, m),
            What::SetAside(m) => (Verb::SetAside, m),
            What::Exile(m) => (Verb::Exile, m),
            What::Put(m) => (Verb::Put, m),
            What::Return(m) => (Verb::Return, m),
            What::Exchange(m) => (Verb::Exchange, m),
            What::Pass(m) => (Verb::Pass, m),
            What::Shuffle(m) => (Verb::Shuffle, m),
            What::Rotate(m) => (Verb::Rotate, m),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Its position in the stream, from 0.
    pub seq: u64,
    /// The event that caused it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<u64>,
    #[serde(flatten)]
    pub what: What,
}

/// The turn in progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnState {
    pub player: u8,
    pub number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    pub phase: Phase,
    #[serde(default)]
    pub kind: TurnKind,
    #[serde(default, skip_serializing_if = "TurnLimits::is_none")]
    pub limits: TurnLimits,
}

/// What the events describe: where every card is, and every counter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// Every zone that exists, empty piles included. An ordered zone's cards are top
    /// first; another's are in the order they arrived.
    pub zones: BTreeMap<Zone, Vec<CardRef>>,
    /// Counters that are not zero, by the name [`CounterChange::counter`] gives them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<String, i64>,
    /// The turn in progress: none before the first and between turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<GameEnd>,
    /// Lasting effects, in the order they began.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// The cards in set-aside zones and `looking` that lie face up.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub face_up: BTreeSet<CardRef>,
    /// Extra turns waiting, in the order the engine keeps them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_turns: Vec<ExtraTurn>,
    /// Set once the end condition is met.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending: Option<Ending>,
}

/// The turn's counters: set as a turn begins, gone when it ends.
const TURN_COUNTERS: [(&str, i64); 7] = [
    ("actions", 1),
    ("buys", 1),
    ("coins", 0),
    ("potions", 0),
    ("next-hand", 5),
    ("bought", 0),
    ("hand-plays", 0),
];

/// Why an event doesn't apply to a table: the stream is not one game's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldError {
    pub seq: u64,
    pub why: String,
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "event {}: {}", self.seq, self.why)
    }
}

impl std::error::Error for FoldError {}

impl Table {
    pub fn zone(&self, zone: &Zone) -> &[CardRef] {
        self.zones.get(zone).map_or(&[], Vec::as_slice)
    }

    pub fn counter(&self, name: &str) -> i64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    /// How many cards lie over the card with `handle` in `zone`.
    pub fn depth(&self, zone: &Zone, handle: u32) -> Option<u32> {
        self.zone(zone)
            .iter()
            .position(|c| c.handle() == Some(handle))
            .map(|i| i as u32)
    }

    /// The table after `event`.
    pub fn apply(&mut self, event: &Event) -> Result<(), FoldError> {
        let fail = |why: String| FoldError {
            seq: event.seq,
            why,
        };
        if let Some((verb, m)) = event.what.as_move() {
            return self.apply_move(verb, m).map_err(fail);
        }
        match &event.what {
            What::Setup(setup) => *self = setup.table.clone(),
            What::Play(play) => match (&play.from, &play.to) {
                (Some(from), Some(to)) => {
                    let m = Move {
                        player: play.player,
                        cards: vec![play.card.clone()],
                        from: from.clone(),
                        to: to.clone(),
                        at: At::Top,
                        pos: play.pos.into_iter().collect(),
                        face: None,
                    };
                    self.apply_move(Verb::Put, &m).map_err(fail)?;
                }
                (None, None) => {}
                _ => return Err(fail("a play goes from somewhere to somewhere".into())),
            },
            What::Buy { .. }
            | What::Resolve { .. }
            | What::Name { .. }
            | What::Way { .. }
            | What::Unaffected { .. }
            | What::Call { .. }
            | What::Choose(_) => {}
            What::TurnSkipped {
                player,
                kind,
                extra_turn,
                ..
            } => self.turn_taken(*player, *kind, *extra_turn).map_err(fail)?,
            What::Turn(t) => {
                self.turn_taken(t.player, t.kind, t.extra_turn)
                    .map_err(fail)?;
                self.turn = Some(TurnState {
                    player: t.player,
                    number: t.number,
                    extra: t.extra.clone(),
                    phase: Phase::Start,
                    kind: t.kind,
                    limits: t.limits,
                });
                for (name, value) in TURN_COUNTERS {
                    self.set_counter(name, value);
                }
            }
            What::Phase { phase } => match &mut self.turn {
                Some(turn) => turn.phase = *phase,
                None => return Err(fail("a phase with no turn".into())),
            },
            What::Counter(c) => {
                let was = self.counter(&c.counter);
                if was + c.delta != c.value {
                    return Err(fail(format!(
                        "{} was {was}: {:+} doesn't make it {}",
                        c.counter, c.delta, c.value
                    )));
                }
                self.set_counter(&c.counter, c.value);
            }
            What::Add { cards, to } => {
                let zone = self.zones.entry(to.clone()).or_default();
                for card in cards {
                    let at = if to.ordered() { 0 } else { zone.len() };
                    zone.insert(at, card.clone());
                }
            }
            What::Rename { card, name } => {
                let handle = card.handle();
                let found = (self.zones.values_mut().flatten())
                    .find(|c| handle.is_some() && c.handle() == handle);
                match found {
                    Some(c) => *c = CardRef::seen(name.clone(), handle.expect("found by it")),
                    None => return Err(fail(format!("{card} is nowhere"))),
                }
            }
            What::GameEnd(end) => self.ended = Some(end.clone()),
            What::Effect(e) => {
                if self.effects.iter().any(|x| x.id == e.id) {
                    return Err(fail(format!("effect {} has begun already", e.id)));
                }
                let at = self.effects.partition_point(|x| x.id < e.id);
                self.effects.insert(at, e.clone());
            }
            What::EffectChange(e) => match self.effects.iter_mut().find(|x| x.id == e.id) {
                Some(x) => *x = e.clone(),
                None => return Err(fail(format!("there is no effect {}", e.id))),
            },
            What::EffectEnd { id, .. } => match self.effects.iter().position(|x| x.id == *id) {
                Some(i) => {
                    self.effects.remove(i);
                }
                None => return Err(fail(format!("there is no effect {id}"))),
            },
            What::ExtraTurn(q) => {
                if self.extra_turns.iter().any(|x| x.id == q.id) {
                    return Err(fail(format!("extra turn {} is waiting already", q.id)));
                }
                let turn = ExtraTurn {
                    id: q.id,
                    player: q.player,
                    card: q.card.clone(),
                    limit: q.limit,
                    effect: q.effect,
                };
                let at = if q.front { 0 } else { self.extra_turns.len() };
                self.extra_turns.insert(at, turn);
            }
            What::ExtraTurnRemoved { id } => self.unqueue(*id).map_err(fail)?,
            What::TurnEnd { player } => {
                if self.turn.as_ref().map(|t| t.player) != Some(*player) {
                    return Err(fail(format!("seat {player} has no turn to end")));
                }
                self.turn = None;
                for (name, _) in TURN_COUNTERS {
                    self.set_counter(name, 0);
                }
            }
            What::GameEnding(ending) => self.ending = Some(ending.clone()),
            _ => unreachable!("moves are applied above"),
        }
        Ok(())
    }

    /// A turn of `player` begins or is skipped: what it was waiting as stops waiting.
    fn turn_taken(&mut self, player: u8, kind: TurnKind, id: Option<u32>) -> Result<(), String> {
        if let Some(id) = id {
            self.unqueue(id)?;
        }
        if kind == TurnKind::Final {
            let next = (self.ending.as_mut())
                .and_then(|e| (!e.final_round.is_empty()).then(|| e.final_round.remove(0)));
            if next != Some(player) {
                return Err(format!("seat {player} has no turn next in the final round"));
            }
        }
        Ok(())
    }

    fn unqueue(&mut self, id: u32) -> Result<(), String> {
        match self.extra_turns.iter().position(|x| x.id == id) {
            Some(i) => {
                self.extra_turns.remove(i);
                Ok(())
            }
            None => Err(format!("there is no extra turn {id} waiting")),
        }
    }

    fn set_counter(&mut self, name: &str, value: i64) {
        if value == 0 {
            self.counters.remove(name);
        } else {
            self.counters.insert(name.to_string(), value);
        }
    }

    fn apply_move(&mut self, verb: Verb, m: &Move) -> Result<(), String> {
        if !m.pos.is_empty() && m.pos.len() != m.cards.len() {
            return Err("pos doesn't give one position per card".into());
        }
        if !m.to.standing() && !self.zones.contains_key(&m.to) {
            return Err(format!("there is no zone {}", m.to));
        }
        self.zones.entry(m.to.clone()).or_default();
        let mut block = vec![];
        for (i, card) in m.cards.iter().enumerate() {
            let said = m.pos.get(i).copied();
            self.take(&m.from, card, said)?;
            self.face_up.remove(card);
            if m.face == Some(Face::Up)
                && matches!(m.to, Zone::Seat(_, SeatZone::Aside | SeatZone::Looking))
            {
                self.face_up.insert(card.clone());
            }
            if verb == Verb::Shuffle {
                block.push(card.clone());
            } else {
                self.place(&m.to, card.clone(), m.at)?;
            }
        }
        if verb == Verb::Shuffle {
            let to = self.zones.get_mut(&m.to).expect("checked above");
            let at = insertion(to.len(), m.at, &m.to)?;
            to.splice(at..at, block);
        }
        self.tidy(&m.from);
        self.tidy(&m.to);
        Ok(())
    }

    /// An empty zone that is always there is not listed.
    fn tidy(&mut self, zone: &Zone) {
        if zone.standing() && self.zone(zone).is_empty() {
            self.zones.remove(zone);
        }
    }

    /// Takes `card` out of `zone`: the one with its handle, or for a face-down card the
    /// one under `said` cards (the top one when not said).
    fn take(&mut self, zone: &Zone, card: &CardRef, said: Option<u32>) -> Result<(), String> {
        let ordered = zone.ordered();
        let cards = self
            .zones
            .get_mut(zone)
            .ok_or_else(|| format!("there is no zone {zone}"))?;
        let found = match card.handle() {
            Some(h) => cards.iter().position(|c| c.handle() == Some(h)),
            None => None,
        };
        let index = match (found, said) {
            (Some(i), Some(p)) if ordered && i != p as usize => {
                return Err(format!("{card} lies under {i} cards in {zone}, not {p}"));
            }
            (Some(i), _) => i,
            // A card the viewer sees only as it leaves: one of the face-down ones.
            (None, _) if ordered => said.unwrap_or(0) as usize,
            (None, _) => cards
                .iter()
                .position(|c| *c == CardRef::Hidden)
                .ok_or_else(|| format!("{card} is not in {zone}"))?,
        };
        match cards.get(index) {
            None => Err(format!("{zone} has no card under {index} others")),
            Some(there) if found.is_none() && *there != CardRef::Hidden => {
                Err(format!("{card} is not in {zone}: {there} lies there"))
            }
            Some(_) => {
                cards.remove(index);
                Ok(())
            }
        }
    }

    fn place(&mut self, zone: &Zone, card: CardRef, at: At) -> Result<(), String> {
        let cards = self.zones.get_mut(zone).expect("checked by apply_move");
        let i = insertion(cards.len(), at, zone)?;
        cards.insert(i, card);
        Ok(())
    }
}

/// Where a card arriving `at` goes among `len` cards.
fn insertion(len: usize, at: At, zone: &Zone) -> Result<usize, String> {
    if !zone.ordered() {
        return match at {
            At::Top => Ok(len),
            _ => Err(format!("{zone} has no positions")),
        };
    }
    match at {
        At::Top => Ok(0),
        At::Bottom => Ok(len),
        At::Under(n) if n as usize <= len => Ok(n as usize),
        At::Under(n) => Err(format!("{zone} has no {n} cards to go under")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards(text: &[&str]) -> Vec<CardRef> {
        text.iter().map(|c| c.parse().unwrap()).collect()
    }

    fn table(zones: &[(&str, &[&str])]) -> Table {
        Table {
            zones: zones
                .iter()
                .map(|(z, c)| (z.parse().unwrap(), cards(c)))
                .collect(),
            ..Table::default()
        }
    }

    fn event(seq: u64, json: &str) -> Event {
        let mut value: serde_json::Value = serde_json::from_str(json).unwrap();
        value["seq"] = seq.into();
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn cards_and_zones_are_written_as_text() {
        for text in ["?", "Copper#12", "King's Court#0"] {
            assert_eq!(text.parse::<CardRef>().unwrap().to_string(), text);
        }
        for text in [
            "0/hand",
            "3/native-village",
            "pile/Village",
            "aside/Black Market",
            "trash",
        ] {
            assert_eq!(text.parse::<Zone>().unwrap().to_string(), text);
        }
        for text in ["Copper", "#3", "Copper#x"] {
            assert!(text.parse::<CardRef>().is_err(), "{text}");
        }
        for text in ["hand", "0/lap", "pile/", "x/hand"] {
            assert!(text.parse::<Zone>().is_err(), "{text}");
        }
    }

    #[test]
    fn an_event_is_one_flat_object() {
        let e = event(
            7,
            r#"{"type":"topdeck","player":1,"cards":["Gold#4"],"from":"1/hand","to":"1/deck"}"#,
        );
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"seq":7,"type":"topdeck","player":1,"cards":["Gold#4"],"from":"1/hand","to":"1/deck"}"#
        );
    }

    #[test]
    fn moves_take_and_place_cards_one_after_the_other() {
        let mut t = table(&[
            ("0/deck", &["Copper#1", "Estate#2", "Gold#3"]),
            ("0/hand", &["Silver#4"]),
            ("0/discard", &[]),
        ]);
        t.apply(&event(
            1,
            r#"{"type":"draw","player":0,"cards":["Copper#1","Estate#2"],"from":"0/deck","to":"0/hand"}"#,
        ))
        .unwrap();
        assert_eq!(
            t.zone(&"0/hand".parse().unwrap()),
            cards(&["Silver#4", "Copper#1", "Estate#2"])
        );
        t.apply(&event(
            2,
            r#"{"type":"discard","player":0,"cards":["Silver#4","Estate#2"],"from":"0/hand","to":"0/discard"}"#,
        ))
        .unwrap();
        // The last one discarded is on top.
        assert_eq!(
            t.zone(&"0/discard".parse().unwrap()),
            cards(&["Estate#2", "Silver#4"])
        );
        t.apply(&event(
            3,
            r#"{"type":"insert","player":0,"cards":["Copper#1"],"from":"0/hand","to":"0/deck","at":{"under":1}}"#,
        ))
        .unwrap();
        assert_eq!(
            t.zone(&"0/deck".parse().unwrap()),
            cards(&["Gold#3", "Copper#1"])
        );
    }

    #[test]
    fn a_shuffle_puts_its_cards_under_the_deck_in_the_order_listed() {
        let mut t = table(&[
            ("0/deck", &["Gold#3"]),
            ("0/discard", &["Copper#1", "Estate#2", "Silver#4"]),
        ]);
        t.apply(&event(
            1,
            r#"{"type":"shuffle","player":0,"cards":["Silver#4","Copper#1","Estate#2"],"from":"0/discard","to":"0/deck","at":"bottom","pos":[2,0,0]}"#,
        ))
        .unwrap();
        assert_eq!(
            t.zone(&"0/deck".parse().unwrap()),
            cards(&["Gold#3", "Silver#4", "Copper#1", "Estate#2"])
        );
        assert!(t.zone(&"0/discard".parse().unwrap()).is_empty());
    }

    #[test]
    fn face_down_cards_are_followed_by_position() {
        let mut t = table(&[
            ("1/deck", &["?", "?", "?"]),
            ("1/hand", &["?"]),
            ("1/play", &[]),
        ]);
        t.apply(&event(
            1,
            r#"{"type":"draw","player":1,"cards":["?"],"from":"1/deck","to":"1/hand"}"#,
        ))
        .unwrap();
        // Seen as it is played: it was one of the face-down cards in the hand.
        t.apply(&event(
            2,
            r#"{"type":"play","player":1,"card":"Village#9","from":"1/hand","to":"1/play"}"#,
        ))
        .unwrap();
        assert_eq!(t.zone(&"1/hand".parse().unwrap()), cards(&["?"]));
        assert_eq!(t.zone(&"1/play".parse().unwrap()), cards(&["Village#9"]));
        assert_eq!(t.zone(&"1/deck".parse().unwrap()).len(), 2);
    }

    #[test]
    fn a_stream_that_is_not_one_games_is_refused() {
        let t = table(&[("0/deck", &["Copper#1", "Gold#3"]), ("0/hand", &[])]);
        for (json, why) in [
            (
                r#"{"type":"draw","player":0,"cards":["Silver#8"],"from":"0/deck","to":"0/hand"}"#,
                "Silver#8 is not in 0/deck",
            ),
            (
                r#"{"type":"draw","player":0,"cards":["Gold#3"],"from":"0/deck","to":"0/hand","pos":[0]}"#,
                "lies under 1 cards",
            ),
            (
                r#"{"type":"draw","player":0,"cards":["Copper#1"],"from":"0/deck","to":"pile/Gold"}"#,
                "no zone pile/Gold",
            ),
            (
                r#"{"type":"counter","counter":"coins","delta":2,"value":3}"#,
                "doesn't make it 3",
            ),
            (r#"{"type":"phase","phase":"buy"}"#, "no turn"),
            (r#"{"type":"turn-end","player":0}"#, "no turn to end"),
            (
                r#"{"type":"effect-end","id":4,"why":"expired"}"#,
                "no effect 4",
            ),
            (
                r#"{"type":"turn","player":0,"number":2,"kind":"extra","extra":"Outpost","extraTurn":3}"#,
                "no extra turn 3",
            ),
            (
                r#"{"type":"turn","player":0,"number":9,"kind":"final"}"#,
                "no turn next in the final round",
            ),
        ] {
            let err = t.clone().apply(&event(5, json)).unwrap_err();
            assert_eq!(err.seq, 5);
            assert!(err.why.contains(why), "{}", err.why);
        }
    }

    #[test]
    fn a_final_round_is_taken_in_its_order_and_a_turn_end_clears_the_turn() {
        let mut t = table(&[]);
        for (seq, json) in [
            r#"{"type":"game-ending","reason":"provinces","finalRound":[1,2]}"#,
            r#"{"type":"turn-skipped","player":1,"kind":"final"}"#,
            r#"{"type":"turn","player":2,"number":7,"kind":"final"}"#,
        ]
        .into_iter()
        .enumerate()
        {
            t.apply(&event(seq as u64, json)).unwrap();
        }
        assert!(t.ending.as_ref().unwrap().final_round.is_empty());
        assert_eq!(t.counter("next-hand"), 5);
        t.apply(&event(3, r#"{"type":"turn-end","player":2}"#))
            .unwrap();
        assert_eq!((t.turn.as_ref(), t.counter("actions")), (None, 0));
    }

    #[test]
    fn a_card_set_aside_face_up_is_face_up_until_it_moves() {
        let mut t = table(&[("0/hand", &["Gold#3", "Copper#1"])]);
        t.apply(&event(
            1,
            r#"{"type":"set-aside","player":0,"cards":["Gold#3"],"from":"0/hand","to":"0/aside","face":"up"}"#,
        ))
        .unwrap();
        assert!(t.face_up.contains(&"Gold#3".parse().unwrap()));
        t.apply(&event(
            2,
            r#"{"type":"put","player":0,"cards":["Gold#3"],"from":"0/aside","to":"0/hand"}"#,
        ))
        .unwrap();
        assert!(t.face_up.is_empty());
    }

    #[test]
    fn a_turn_starts_with_one_action_and_one_buy() {
        let mut t = table(&[]);
        t.counters.insert("coins".into(), 5);
        t.apply(&event(
            1,
            r#"{"type":"turn","player":1,"number":4,"extra":"Outpost"}"#,
        ))
        .unwrap();
        t.apply(&event(2, r#"{"type":"phase","phase":"action"}"#))
            .unwrap();
        t.apply(&event(
            3,
            r#"{"type":"counter","counter":"actions","delta":2,"value":3}"#,
        ))
        .unwrap();
        let turn = t.turn.as_ref().unwrap();
        assert_eq!(
            (turn.player, turn.number, turn.phase),
            (1, 4, Phase::Action)
        );
        assert_eq!(turn.extra.as_deref(), Some("Outpost"));
        assert_eq!(
            (t.counter("actions"), t.counter("buys"), t.counter("coins")),
            (3, 1, 0)
        );
    }
}

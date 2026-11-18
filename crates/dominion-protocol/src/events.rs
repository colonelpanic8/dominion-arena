//! Serde types for `dominion/events@1`: a game of Dominion as a stream of events, and the
//! [`Table`] they describe. The table after event *n* is the fold of events `0..=n`
//! ([`Table::apply`]); no rules engine is needed to follow a game. The specification is
//! `protocol/events.md` at the repository's root.
//!
//! The format is these types, not an encoding of them. JSON Lines is what people read;
//! any self-describing serde format carries the same stream (CBOR is tested). A format
//! that isn't self-describing (bincode, postcard) can't: an event is a tagged, flattened
//! map.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
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
    /// The stream was not written as the game was played but rebuilt afterwards; absent
    /// for a stream recorded live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconstructed: Option<Reconstructed>,
}

/// How a stream not recorded as its game was played was rebuilt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reconstructed {
    /// When, as an RFC 3339 UTC time.
    pub at: String,
    /// From what: `"history"`, the host's saved game (its setup and every answer it
    /// took) played again by the engine `setup.rules` names and checked against it.
    pub from: String,
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

/// A card as the stream's viewer sees it: its name, or `?` face down.
///
/// Copies of a card are interchangeable, so nothing numbers them: what can tell two
/// copies apart is something attached to one (its position in an ordered zone, the effect
/// that keeps or holds it, its face), and each of those is said where it applies.
///
/// Ordered by `?` first, then by name: the order of a bag ([`Zone::ordered`]).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CardRef {
    Hidden,
    Name(String),
}

impl CardRef {
    pub fn named(name: impl Into<String>) -> CardRef {
        CardRef::Name(name.into())
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            CardRef::Hidden => None,
            CardRef::Name(name) => Some(name),
        }
    }
}

impl fmt::Display for CardRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CardRef::Hidden => f.write_str("?"),
            CardRef::Name(name) => f.write_str(name),
        }
    }
}

impl FromStr for CardRef {
    type Err = String;

    fn from_str(s: &str) -> Result<CardRef, String> {
        match s {
            "?" => Ok(CardRef::Hidden),
            "" => Err("a card has a name or is \"?\"".into()),
            _ => Ok(CardRef::Name(s.to_string())),
        }
    }
}

/// Puts `card` into `bag`, a list sorted in [`CardRef`]'s order.
pub fn bag_insert(bag: &mut Vec<CardRef>, card: CardRef) {
    bag_insert_at(bag, card);
}

/// Puts `card` into `bag`; where it went.
fn bag_insert_at(bag: &mut Vec<CardRef>, card: CardRef) -> usize {
    let i = bag.partition_point(|c| *c <= card);
    bag.insert(i, card);
    i
}

/// Takes one `card` out of `bag`; whether there was one.
pub fn bag_remove(bag: &mut Vec<CardRef>, card: &CardRef) -> bool {
    match bag.binary_search(card) {
        Ok(i) => {
            bag.remove(i);
            true
        }
        Err(_) => false,
    }
}

/// How many of `card` `cards` has.
fn count(cards: &[CardRef], card: &CardRef) -> usize {
    cards.iter().filter(|c| *c == card).count()
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
    /// Boons the player took, is receiving or keeps.
    Boons,
    /// A Hex the player turned over, while it is received.
    Hexes,
}

impl SeatZone {
    pub const ALL: [SeatZone; 12] = [
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
        SeatZone::Boons,
        SeatZone::Hexes,
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
            SeatZone::Boons => "boons",
            SeatZone::Hexes => "hexes",
        }
    }
}

/// Part of the Boons' or the Hexes' table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pack {
    Deck,
    Discard,
    /// Druid's three Boons, set aside for the game.
    Druid,
}

impl Pack {
    pub fn name(self) -> &'static str {
        match self {
            Pack::Deck => "deck",
            Pack::Discard => "discard",
            Pack::Druid => "druid",
        }
    }
}

/// Where cards are: `<seat>/<zone>`, `pile/<card>`, `aside/<card>`, `trash`,
/// `boons/<pack>` or `hexes/<pack>`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Zone {
    Seat(u8, SeatZone),
    /// A pile, by the card or randomizer it is named after.
    Pile(String),
    /// Cards set aside for the game by a card: the Black Market deck, Way of the Mouse's
    /// card.
    Aside(String),
    Trash,
    Boons(Pack),
    Hexes(Pack),
}

impl Zone {
    /// Whether its cards have positions: `cards[0]` is the top. The others are bags: how
    /// many of each card, written sorted in [`CardRef`]'s order.
    pub fn ordered(&self) -> bool {
        matches!(
            self,
            Zone::Seat(_, SeatZone::Deck | SeatZone::Discard)
                | Zone::Pile(_)
                | Zone::Aside(_)
                | Zone::Trash
                | Zone::Boons(_)
                | Zone::Hexes(_)
        )
    }

    /// Whether its cards lie face up or face down: a seat's set-aside zone and `looking`.
    pub fn has_faces(&self) -> bool {
        matches!(self, Zone::Seat(_, SeatZone::Aside | SeatZone::Looking))
    }

    /// Whether it is there in every game, empty or not: a seat's zones, the trash, and
    /// the Boons' and Hexes'. A pile exists once the game has it.
    fn standing(&self) -> bool {
        matches!(
            self,
            Zone::Seat(..) | Zone::Trash | Zone::Boons(_) | Zone::Hexes(_)
        )
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Zone::Seat(seat, zone) => write!(f, "{seat}/{}", zone.name()),
            Zone::Pile(card) => write!(f, "pile/{card}"),
            Zone::Aside(card) => write!(f, "aside/{card}"),
            Zone::Trash => f.write_str("trash"),
            Zone::Boons(pack) => write!(f, "boons/{}", pack.name()),
            Zone::Hexes(pack) => write!(f, "hexes/{}", pack.name()),
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
        if owner == "boons" || owner == "hexes" {
            let pack = [Pack::Deck, Pack::Discard, Pack::Druid]
                .into_iter()
                .find(|p| p.name() == rest)
                .ok_or_else(bad)?;
            return match (owner, pack) {
                ("boons", pack) => Ok(Zone::Boons(pack)),
                (_, Pack::Druid) => Err(bad()),
                (_, pack) => Ok(Zone::Hexes(pack)),
            };
        }
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

serde_as_string!(CardRef, Zone, CounterKey, TokenAt);

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
    /// Whose action it is: the owner of `from`, else of `to`; between zones nobody owns,
    /// the player of the occurrence it happens in. Absent when nobody acts (the Boons
    /// reshuffled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    pub cards: Vec<CardRef>,
    pub from: Zone,
    pub to: Zone,
    /// Where they arrive, when `to` is ordered.
    #[serde(default, skip_serializing_if = "At::is_top")]
    pub at: At,
    /// For an ordered `from`: how many cards lay over each card as it left, when any
    /// did. Absent: each came off the top. Of a run of cards of one name, the topmost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pos: Vec<u32>,
    /// Into a set-aside zone or `looking`: whether the cards lie face up (everyone sees
    /// them) or face down (their owner does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<Face>,
    /// How the cards lay in `from`, when not as that zone's cards ordinarily lie: `up` out
    /// of a set-aside zone or `looking` (absent: face down there), `down` out of the trash
    /// (absent: face up there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_face: Option<Face>,
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
    /// As [`Move::from_face`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_face: Option<Face>,
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
    Take,
}

/// The moment a triggered ability answers: what its `resolve` event's `by` names, or (for
/// a moment before the thing it answers happens) the cause of that thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Moment {
    /// Before the first turn.
    GameStart,
    StartTurn,
    StartBuyPhase,
    /// As the Buy phase ends, before the next phase.
    EndBuyPhase,
    StartNight,
    StartCleanup,
    /// Before cards are discarded from play.
    DiscardFromPlay,
    /// After Clean-up's draw.
    EndTurn,
    /// A card is played, before its instructions are followed.
    Play,
    /// Its instructions have been followed.
    AfterPlay,
    /// An Attack is played against `target`.
    Attack,
    Buy,
    /// A card is about to be gained.
    WouldGain,
    Gain,
    Trash,
    /// A card left a Supply pile other than by being gained or trashed.
    LeaveSupply,
    Discard,
    /// A discard pile is about to be shuffled.
    WouldShuffle,
    Shuffle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Start,
    Action,
    Buy,
    Night,
    Cleanup,
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
    /// The card it came from, when a card made it: the card played (a Way's effect is the
    /// Way's, its source the card played), the Estate playing an inherited card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CardRef>,
    pub owner: u8,
    pub until: Until,
    /// Its card left play: it works to the end of this turn and ends then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ending: bool,
    /// Cards it keeps in play past Clean-up: a bag of its owner's play area.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keeps: Vec<CardRef>,
    /// Cards it holds set aside: a bag of its owner's set-aside zone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<CardRef>,
    /// Tokens on its card (Garrison).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
    /// Of `holds`, those lying face up: a bag. The zone's face-up cards don't say whose
    /// they are when it has cards of one name lying both ways.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_up: Vec<CardRef>,
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

/// Whose a counter is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CounterOf {
    /// The turn in progress.
    Turn,
    /// The game's own.
    Game,
    Seat(u8),
    /// Tokens on a pile.
    Pile(Zone),
    /// Tokens on a landscape.
    Landscape(String),
}

/// Which counter of its owner's a counter is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CounterName {
    Actions,
    Buys,
    Coins,
    Potions,
    TradeRouteMat,
    Sun,
    Coffers,
    Villagers,
    Favors,
    Debt,
    VpTokens,
    PirateShip,
    /// VP tokens on a pile or landscape.
    Vp,
    Embargo,
    TradeRoute,
    /// A player's tokens on Sinister Plot.
    SinisterPlot,
    NextHand,
    Bought,
    HandPlays,
    SkipTurns,
    TurnsSkipped,
}

impl CounterName {
    const ALL: [CounterName; 21] = [
        CounterName::Actions,
        CounterName::Buys,
        CounterName::Coins,
        CounterName::Potions,
        CounterName::TradeRouteMat,
        CounterName::Sun,
        CounterName::Coffers,
        CounterName::Villagers,
        CounterName::Favors,
        CounterName::Debt,
        CounterName::VpTokens,
        CounterName::PirateShip,
        CounterName::Vp,
        CounterName::Embargo,
        CounterName::TradeRoute,
        CounterName::SinisterPlot,
        CounterName::NextHand,
        CounterName::Bought,
        CounterName::HandPlays,
        CounterName::SkipTurns,
        CounterName::TurnsSkipped,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CounterName::Actions => "actions",
            CounterName::Buys => "buys",
            CounterName::Coins => "coins",
            CounterName::Potions => "potions",
            CounterName::TradeRouteMat => "trade-route-mat",
            CounterName::Sun => "sun",
            CounterName::Coffers => "coffers",
            CounterName::Villagers => "villagers",
            CounterName::Favors => "favors",
            CounterName::Debt => "debt",
            CounterName::VpTokens => "vp-tokens",
            CounterName::PirateShip => "pirate-ship",
            CounterName::Vp => "vp",
            CounterName::Embargo => "embargo",
            CounterName::TradeRoute => "trade-route",
            CounterName::SinisterPlot => "sinister-plot",
            CounterName::NextHand => "next-hand",
            CounterName::Bought => "bought",
            CounterName::HandPlays => "hand-plays",
            CounterName::SkipTurns => "skip-turns",
            CounterName::TurnsSkipped => "turns-skipped",
        }
    }

    fn of_turn(self) -> bool {
        matches!(
            self,
            CounterName::Actions
                | CounterName::Buys
                | CounterName::Coins
                | CounterName::Potions
                | CounterName::NextHand
                | CounterName::Bought
                | CounterName::HandPlays
        )
    }
}

/// A counter: whose it is and which of theirs. Written as text: the turn's and the
/// game's by name (`actions`, `sun`), a seat's as `<seat>/<name>` (`0/coffers`), a
/// pile's as `<pile>/<name>` (`pile/Village/vp`), a landscape's as
/// `landscape/<card>/<name>` (`landscape/Arena/vp`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CounterKey {
    pub of: CounterOf,
    pub name: CounterName,
}

impl CounterKey {
    pub fn new(of: CounterOf, name: CounterName) -> CounterKey {
        CounterKey { of, name }
    }

    /// The turn's or the game's counter `name`.
    pub fn global(name: CounterName) -> CounterKey {
        let of = match name.of_turn() {
            true => CounterOf::Turn,
            false => CounterOf::Game,
        };
        CounterKey { of, name }
    }

    pub fn seat(seat: u8, name: CounterName) -> CounterKey {
        CounterKey::new(CounterOf::Seat(seat), name)
    }
}

impl fmt::Display for CounterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.name.name();
        match &self.of {
            CounterOf::Turn | CounterOf::Game => f.write_str(name),
            CounterOf::Seat(seat) => write!(f, "{seat}/{name}"),
            CounterOf::Pile(zone) => write!(f, "{zone}/{name}"),
            CounterOf::Landscape(card) => write!(f, "landscape/{card}/{name}"),
        }
    }
}

impl FromStr for CounterKey {
    type Err = String;

    fn from_str(s: &str) -> Result<CounterKey, String> {
        let bad = || format!("{s:?} is no counter");
        let (owner, name) = match s.rsplit_once('/') {
            Some((owner, name)) => (Some(owner), name),
            None => (None, s),
        };
        let name = (CounterName::ALL.into_iter())
            .find(|n| n.name() == name)
            .ok_or_else(bad)?;
        let of = match owner {
            None => return Ok(CounterKey::global(name)),
            Some(owner) => match owner.strip_prefix("landscape/") {
                Some("") => return Err(bad()),
                Some(card) => CounterOf::Landscape(card.to_string()),
                None => match (owner.parse::<u8>(), owner.parse::<Zone>()) {
                    (Ok(seat), _) => CounterOf::Seat(seat),
                    (_, Ok(zone @ (Zone::Pile(_) | Zone::Aside(_)))) => CounterOf::Pile(zone),
                    _ => return Err(bad()),
                },
            },
        };
        Ok(CounterKey { of, name })
    }
}

/// A counter changing by `delta` to `value`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterChange {
    pub counter: CounterKey,
    pub delta: i64,
    pub value: i64,
}

/// A player's token: an Adventures token, the -1 Card or -$1 token, or the Journey token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TokenKind {
    PlusCard,
    PlusAction,
    PlusBuy,
    PlusCoin,
    MinusCost,
    Trashing,
    Estate,
    MinusCard,
    MinusCoin,
    Journey,
}

/// Where a token is, when the table lists it. Written as text: a pile's zone, `player`
/// or `face-down`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TokenAt {
    /// On that pile.
    Pile(Zone),
    /// With its player and on no pile: the -1 Card token on their deck, the -$1 token in
    /// front of them, the Estate token on Inheritance's card, an Adventures token whose
    /// pile left the game.
    Player,
    /// The Journey token, turned face down.
    FaceDown,
}

impl fmt::Display for TokenAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenAt::Pile(zone) => zone.fmt(f),
            TokenAt::Player => f.write_str("player"),
            TokenAt::FaceDown => f.write_str("face-down"),
        }
    }
}

impl FromStr for TokenAt {
    type Err = String;

    fn from_str(s: &str) -> Result<TokenAt, String> {
        match s {
            "player" => Ok(TokenAt::Player),
            "face-down" => Ok(TokenAt::FaceDown),
            _ => match s.parse()? {
                zone @ Zone::Pile(_) => Ok(TokenAt::Pile(zone)),
                _ => Err(format!("{s:?} is nowhere a token can be")),
            },
        }
    }
}

/// A token the table lists: one that is out. A token it doesn't list is not placed (an
/// Adventures token), not had (the -1 Card and -$1 tokens), or face up (Journey).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenState {
    pub player: u8,
    pub token: TokenKind,
    pub at: TokenAt,
}

/// What setup made a pile for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Young Witch's Bane.
    Bane,
    /// A pile the card adds to the game (Ferryman's).
    ExtraPile,
    /// A pile the card chose (Obelisk's).
    ChosenPile,
    /// A card set aside for the game (Riverboat's, Way of the Mouse's).
    SetAside,
    /// A deck the card uses, whose contents are public and order is not (the Black
    /// Market deck).
    Deck,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PileRole {
    /// The card or landscape it is for.
    pub card: String,
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeKind {
    /// Groups of cards one on another, in the order listed.
    Stacked,
    /// Groups whose top cards go to the bottom (the Augurs, ...).
    Rotating,
    /// The cards shuffled together.
    Shuffled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeGroup {
    pub card: String,
    pub count: u32,
}

/// What a pile was made of in setup: never its contents now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub kind: RecipeKind,
    /// Top first, or a pool for a shuffled pile.
    pub cards: Vec<RecipeGroup>,
    /// How many of the pool it took, when it took only some.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<u32>,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// A pile's properties.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PileState {
    /// The card or randomizer it is named after.
    pub card: String,
    /// In the Supply.
    #[serde(default, skip_serializing_if = "is_false")]
    pub supply: bool,
    /// Face down: not even its top card is public.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Out of the game (Divine Wind): it is no card's pile any more, though its cards and
    /// Traits are still its own.
    #[serde(default, skip_serializing_if = "is_false")]
    pub removed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<PileRole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<Recipe>,
}

/// Public state a card or landscape keeps that the table has no other place for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pile: Option<Zone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub count: i32,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// What a card costs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub coins: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub potions: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub debt: u32,
}

/// A pile's properties are now `state`. A pile that wasn't there now is, empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PileChange {
    pub pile: Zone,
    #[serde(flatten)]
    pub state: PileState,
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
    /// What the game was set up from besides `seats` and `kingdom`. Absent from a stream
    /// recorded before it was written, which can't be replayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<SetupConfig>,
    /// What setup decided by chance, in the order drawn, besides the opening shuffles (the
    /// `shuffle` events after this one).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chance: Vec<Chance>,
}

/// What a game was set up from: with the seats, the kingdom and the chance outcomes, enough
/// to set up the same game again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupConfig {
    /// The landscapes given; setup may add more (an Ally or Prophecy: `chance`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landscapes: Vec<String>,
    /// Platinum and Colony are in the game, as decided.
    pub colonies: bool,
    /// Shelters replace the starting Estates, as decided.
    pub shelters: bool,
    /// The game ends once this many turns have been taken in all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_limit: Option<u32>,
    /// Choices setup makes that were given rather than drawn, by the card making them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub choices: BTreeMap<String, Vec<String>>,
    /// The engine's seed. It reproduces the chance outcomes only in the build that drew
    /// them; nothing needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// What a chance outcome decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChanceSite {
    /// The Boons' deck as it was made, top first.
    Boons,
    /// The Hexes' deck as it was made, top first.
    Hexes,
    /// A shuffled pile as it was made (named by `card`), top first.
    Pile,
    /// A choice setup made at random, by `card`, in its order.
    Choice,
    /// The game's Ally.
    Ally,
    /// The game's Prophecy.
    Prophecy,
    /// The Kingdom piles Divine Wind set up, in order.
    Kingdom,
}

/// A chance outcome no `shuffle` event shows: what was drawn, by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chance {
    pub site: ChanceSite,
    /// The pile's card, or the card making the choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub cards: Vec<String>,
}

/// What a seat was asked: a canonical projection of the decision onto agent-view@2, the
/// same whatever the seat knows. It has one option group, and `min` and `max` always.
/// What the seat was shown can differ only where nothing hangs on an index: a name
/// question there may suggest cards the seat knows of, here only the visible piles' cards;
/// and Arena's hint about answering a Way question in advance is not in `prompt`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub seat: u8,
    pub prompt: String,
    pub kind: String,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ordered: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub name_any: bool,
    pub min: u32,
    pub max: u32,
    pub options: Vec<crate::view::AgentOption>,
}

/// What a seat answered the question just asked: option indices, or a card's name where
/// any card may be named.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub seat: u8,
    /// The options picked, in the order picked: written whenever the answer picks options,
    /// `[]` when it picks none. Absent only when `name` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indices: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// An ability of `card` begins to resolve for `player`, answering the moment `when`.
    Resolve {
        player: u8,
        card: String,
        when: Moment,
        /// The card the moment is about, where it is about one: the card gained, trashed,
        /// discarded, bought or about to be gained. For a gain, trash or discard, one of
        /// the cards of the move `by` names; which copy is no matter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<CardRef>,
        /// For an Attack: the player it is played against.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<u8>,
        /// When `by` is a move of several cards, each its own occurrence (cards trashed or
        /// discarded together): which of its `cards` it answers, from 0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
        /// The event that made the lasting effect the ability belongs to (a Duration's
        /// play, turns ago).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<u64>,
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
    /// `cards` is the order the shuffle gave, top first, Shadow cards already at the
    /// bottom, where the rules put them: no move of theirs follows, unless their owner
    /// orders two or more (`bottomdeck`). Into the zone it takes them from (a deck), some
    /// of its cards shuffled again where they lay go back under `at`.
    Shuffle(Move),
    /// From the top of a pile to its bottom.
    Rotate(Move),
    /// Cards come into the game in `to`, each on top of the one before. A pile that
    /// wasn't there is.
    Add {
        cards: Vec<CardRef>,
        to: Zone,
    },
    /// The card `card` in `zone` is now a card called `name`: in an ordered zone the one
    /// under exactly `pos` cards; in a set-aside zone or `looking`, one lying `face` (absent:
    /// face down).
    Rename {
        card: CardRef,
        name: String,
        zone: Zone,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pos: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        face: Option<Face>,
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
    /// A Boon or Hex off its deck, or out of its discard pile, to the player taking or
    /// receiving it.
    Take(Move),
    /// `player` receives `card`: a Boon or Hex (its instructions are followed), a State
    /// or Artifact (they take it), the Prophecy (it comes into effect).
    Receive {
        player: u8,
        card: String,
    },
    /// `player`'s `token` is now `at`; without `at`, the table no longer lists it.
    Token {
        player: u8,
        token: TokenKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<TokenAt>,
    },
    /// A Project cube, Artifact or State changes hands: `from` no longer holds `card`,
    /// `to` does.
    Holding {
        card: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<u8>,
    },
    Pile(PileChange),
    /// A landscape or Artifact joins the game; `prophecy` when it is the game's Prophecy.
    Landscape {
        card: String,
        #[serde(default, skip_serializing_if = "is_false")]
        prophecy: bool,
    },
    /// The table's notes are now `notes`. Always written: `[]` clears them.
    Notes {
        notes: Vec<Note>,
    },
    /// What the rules derive changed: what cards cost now (`null`: no card of that name is
    /// in the game any more), and their types where they are not the printed ones (`null`:
    /// printed again).
    Derived {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        costs: BTreeMap<String, Option<Cost>>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        types: BTreeMap<String, Option<Vec<String>>>,
    },
    /// A seat is asked something: the game waits on its answer.
    Ask(Ask),
    /// The seat asked answers.
    Answer(Answer),
    /// A chance outcome during play that no `shuffle` shows (Divine Wind's new piles and
    /// what their setup draws).
    Chance(Chance),
    /// The card `card` in `zone` (under exactly `pos` cards, in an ordered zone) now lies
    /// `face`: a card Necromancer played turning face down in the trash, or face up again
    /// as the turn ends.
    Face {
        card: CardRef,
        zone: Zone,
        face: Face,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pos: Option<u32>,
    },
    /// The cards of `zone`, a play area, that some effect keeps in play are now `cards`: a
    /// bag, each card once however many effects keep it. The others leave play in
    /// Clean-up.
    Kept {
        zone: Zone,
        cards: Vec<CardRef>,
    },
    /// Cards are seen where they lie, without moving.
    Show(Show),
    /// The cards `cards` of `zone` (under `pos` cards each, in an ordered zone) are
    /// rearranged where nobody can follow which is which: cards discarded together unseen,
    /// put back in an order only their owner chose, cards left over after a "when you
    /// shuffle" choice. Not written for a shuffle, which says as much itself.
    Mix {
        zone: Zone,
        cards: Vec<CardRef>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pos: Vec<u32>,
    },
}

/// Cards seen where they lie: revealed, or looked at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Show {
    /// Whose cards they are: the owner of `zone`, else the player of the occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<u8>,
    pub cards: Vec<CardRef>,
    pub zone: Zone,
    /// In an ordered zone, how many cards lie over each, exactly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pos: Vec<u32>,
    /// The seat who sees them; absent, everyone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<u8>,
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
            Verb::Take => What::Take(m),
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
            | What::Rotate(m)
            | What::Take(m) => Some(m),
            _ => None,
        }
    }

    /// Whether it is an occurrence: something other events can happen inside.
    pub fn is_occurrence(&self) -> bool {
        matches!(
            self,
            What::Turn(_)
                | What::Phase { .. }
                | What::TurnEnd { .. }
                | What::Receive { .. }
                | What::Play(_)
                | What::Buy { .. }
                | What::Resolve { .. }
                | What::Gain(_)
                | What::Trash(_)
                | What::Discard(_)
                | What::Shuffle(_)
        )
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
            What::Take(m) => (Verb::Take, m),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Its position in the stream, from 0.
    pub seq: u64,
    /// The event that caused it: the occurrence (a `turn`, `phase`, `turn-end`, `play`,
    /// `buy`, `resolve`, `receive`, `gain`, `trash`, `discard` or `shuffle`) it happened
    /// inside. Always earlier.
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
    /// first; another's are a bag, sorted.
    pub zones: BTreeMap<Zone, Vec<CardRef>>,
    /// Counters that are not zero.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<CounterKey, i64>,
    /// The turn in progress: none before the first and between turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<GameEnd>,
    /// Lasting effects, in the order they began.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Of the cards in each set-aside zone and `looking`, those that lie face up: a bag.
    /// The rest of the zone lies face down.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub face_up: BTreeMap<Zone, Vec<CardRef>>,
    /// Extra turns waiting, in the order the engine keeps them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_turns: Vec<ExtraTurn>,
    /// Set once the end condition is met.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending: Option<Ending>,
    /// Every pile's properties, by its zone.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub piles: BTreeMap<Zone, PileState>,
    /// Every landscape and Artifact in the game, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landscapes: Vec<String>,
    /// The game's Prophecy: in effect while the `sun` counter is 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prophecy: Option<String>,
    /// Who holds each Project (a cube), Artifact and State that someone holds, by seat.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub holdings: BTreeMap<String, Vec<u8>>,
    /// Tokens that are out, by player and token.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<TokenState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// Derived: what each card in the game costs now, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub costs: BTreeMap<String, Cost>,
    /// Derived: the types of the cards whose types are not the printed ones now, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, Vec<String>>,
    /// Where the fold that made it is in its stream.
    #[serde(default, skip_serializing_if = "Progress::is_fresh")]
    pub progress: Progress,
    /// Of the cards in each zone whose cards lie face up (the trash), those that lie face
    /// down: a bag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub face_down: BTreeMap<Zone, Vec<CardRef>>,
    /// Of the cards in each play area, those some effect keeps in play: a bag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kept: BTreeMap<Zone, Vec<CardRef>>,
}

/// How far a fold has read its stream: what [`Table::apply`] needs to refuse an event out
/// of place. It is no part of the table, so tables are equal whatever it says.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    /// The `seq` the next event must have; 0 before `setup`.
    next: u64,
    /// Which events so far were occurrences, a bit per `seq`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    occurrences: Vec<u64>,
}

impl Progress {
    fn is_fresh(&self) -> bool {
        self.next == 0
    }

    fn occurrence(&self, seq: u64) -> bool {
        (self.occurrences.get((seq / 64) as usize)).is_some_and(|bits| bits >> (seq % 64) & 1 == 1)
    }

    fn applied(&mut self, seq: u64, occurrence: bool) {
        self.next = seq + 1;
        if occurrence {
            let word = (seq / 64) as usize;
            if self.occurrences.len() <= word {
                self.occurrences.resize(word + 1, 0);
            }
            self.occurrences[word] |= 1 << (seq % 64);
        }
    }
}

impl PartialEq for Progress {
    fn eq(&self, _: &Progress) -> bool {
        true
    }
}

impl Eq for Progress {}

/// The turn's counters: set as a turn begins, gone when it ends.
const TURN_COUNTERS: [(CounterName, i64); 7] = [
    (CounterName::Actions, 1),
    (CounterName::Buys, 1),
    (CounterName::Coins, 0),
    (CounterName::Potions, 0),
    (CounterName::NextHand, 5),
    (CounterName::Bought, 0),
    (CounterName::HandPlays, 0),
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

    pub fn counter(&self, key: &CounterKey) -> i64 {
        self.counters.get(key).copied().unwrap_or(0)
    }

    /// The cards of `zone` that lie face up.
    pub fn face_up(&self, zone: &Zone) -> &[CardRef] {
        self.face_up.get(zone).map_or(&[], Vec::as_slice)
    }

    /// The `seq` the next event must have.
    pub fn next_seq(&self) -> u64 {
        self.progress.next
    }

    /// The table after `event`. Refused, the table is as it was.
    pub fn apply(&mut self, event: &Event) -> Result<(), FoldError> {
        let fail = |why: String| FoldError {
            seq: event.seq,
            why,
        };
        self.in_place(event).map_err(fail)?;
        self.change(&event.what).map_err(fail)?;
        self.progress.applied(event.seq, event.what.is_occurrence());
        Ok(())
    }

    /// Whether `event` can come next in the stream so far.
    fn in_place(&self, event: &Event) -> Result<(), String> {
        let next = self.progress.next;
        if event.seq != next {
            return Err(format!("the next event is {next}"));
        }
        match (&event.what, next) {
            (What::Setup(_), 0) => {}
            (What::Setup(_), _) => return Err("setup comes only first".into()),
            (_, 0) => return Err("a stream begins with setup".into()),
            _ => {}
        }
        if self.ended.is_some() {
            return Err("the game is over".into());
        }
        let origin = match event.what {
            What::Resolve { origin, .. } => origin,
            _ => None,
        };
        for cause in [event.by, origin].into_iter().flatten() {
            if cause >= event.seq {
                return Err(format!("event {cause} is not earlier"));
            }
            if !self.progress.occurrence(cause) {
                return Err(format!("event {cause} is no occurrence"));
            }
        }
        Ok(())
    }

    /// What `what` does to the table, with no regard to where it stands in a stream: how a
    /// table is built from changes (a setup's). Refused, the table is as it was.
    pub fn change(&mut self, what: &What) -> Result<(), String> {
        if let Some((verb, m)) = what.as_move() {
            return self.apply_move(verb, m);
        }
        match what {
            What::Setup(setup) => {
                let progress = std::mem::take(&mut self.progress);
                *self = setup.table.clone();
                self.progress = progress;
            }
            What::Play(play) => match (&play.from, &play.to) {
                (Some(from), Some(to)) => {
                    let m = Move {
                        player: Some(play.player),
                        cards: vec![play.card.clone()],
                        from: from.clone(),
                        to: to.clone(),
                        at: At::Top,
                        pos: play.pos.into_iter().collect(),
                        face: None,
                        from_face: play.from_face,
                    };
                    self.apply_move(Verb::Put, &m)?;
                }
                (None, None) => {}
                _ => return Err("a play goes from somewhere to somewhere".into()),
            },
            What::Ask(_) | What::Answer(_) | What::Chance(_) => {}
            What::Face {
                card,
                zone,
                face,
                pos,
            } => self.turn_over(card, zone, *face, *pos)?,
            What::Kept { zone, cards } => {
                if !matches!(zone, Zone::Seat(_, SeatZone::Play)) {
                    return Err(format!("{zone} is no play area"));
                }
                let mut sorted = cards.clone();
                sorted.sort();
                if !sorted
                    .iter()
                    .all(|c| count(&sorted, c) <= count(self.zone(zone), c))
                {
                    return Err(format!("{zone} hasn't the cards kept"));
                }
                match sorted.is_empty() {
                    true => self.kept.remove(zone),
                    false => self.kept.insert(zone.clone(), sorted),
                };
            }
            What::Show(Show {
                cards, zone, pos, ..
            })
            | What::Mix { cards, zone, pos } => self.lie(zone, cards, pos)?,
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
            } => {
                if let Some(turn) = &self.turn {
                    return Err(format!("seat {}'s turn is in progress", turn.player));
                }
                self.turn_taken(*player, *kind, *extra_turn)?;
            }
            What::Turn(t) => {
                if let Some(turn) = &self.turn {
                    return Err(format!("seat {}'s turn is in progress", turn.player));
                }
                self.turn_taken(t.player, t.kind, t.extra_turn)?;
                self.turn = Some(TurnState {
                    player: t.player,
                    number: t.number,
                    extra: t.extra.clone(),
                    phase: Phase::Start,
                    kind: t.kind,
                    limits: t.limits,
                });
                for (name, value) in TURN_COUNTERS {
                    self.set_counter(&CounterKey::global(name), value);
                }
            }
            What::Phase { phase } => match &mut self.turn {
                Some(turn) => turn.phase = *phase,
                None => return Err("a phase with no turn".into()),
            },
            What::Counter(c) => {
                if let CounterOf::Pile(pile) = &c.counter.of
                    && !self.piles.contains_key(pile)
                {
                    return Err(format!("there is no pile {pile}"));
                }
                let was = self.counter(&c.counter);
                if was + c.delta != c.value {
                    return Err(format!(
                        "{} was {was}: {:+} doesn't make it {}",
                        c.counter, c.delta, c.value
                    ));
                }
                self.set_counter(&c.counter, c.value);
            }
            What::Add { cards, to } => {
                let zone = self.zones.entry(to.clone()).or_default();
                for card in cards {
                    match to.ordered() {
                        true => zone.insert(0, card.clone()),
                        false => bag_insert(zone, card.clone()),
                    }
                }
            }
            What::Rename {
                card,
                name,
                zone,
                pos,
                face,
            } => self.rename(card, name, zone, *pos, *face)?,
            What::GameEnd(end) => self.ended = Some(end.clone()),
            What::Effect(e) => {
                if self.effects.iter().any(|x| x.id == e.id) {
                    return Err(format!("effect {} has begun already", e.id));
                }
                let at = self.effects.partition_point(|x| x.id < e.id);
                self.effects.insert(at, e.clone());
            }
            What::EffectChange(e) => match self.effects.iter_mut().find(|x| x.id == e.id) {
                Some(x) => *x = e.clone(),
                None => return Err(format!("there is no effect {}", e.id)),
            },
            What::EffectEnd { id, .. } => match self.effects.iter().position(|x| x.id == *id) {
                Some(i) => {
                    self.effects.remove(i);
                }
                None => return Err(format!("there is no effect {id}")),
            },
            What::ExtraTurn(q) => {
                if self.extra_turns.iter().any(|x| x.id == q.id) {
                    return Err(format!("extra turn {} is waiting already", q.id));
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
            What::ExtraTurnRemoved { id } => {
                let i = self.waiting(*id)?;
                self.extra_turns.remove(i);
            }
            What::TurnEnd { player } => {
                if self.turn.as_ref().map(|t| t.player) != Some(*player) {
                    return Err(format!("seat {player} has no turn to end"));
                }
                self.turn = None;
                for (name, _) in TURN_COUNTERS {
                    self.set_counter(&CounterKey::global(name), 0);
                }
            }
            What::GameEnding(ending) => self.ending = Some(ending.clone()),
            What::Receive { .. } => {}
            What::Token { player, token, at } => {
                if let Some(TokenAt::Pile(pile)) = at
                    && !self.piles.contains_key(pile)
                {
                    return Err(format!("there is no pile {pile}"));
                }
                self.tokens
                    .retain(|t| (t.player, t.token) != (*player, *token));
                if let Some(at) = at {
                    let i = (self.tokens.iter())
                        .position(|t| (t.player, t.token) > (*player, *token))
                        .unwrap_or(self.tokens.len());
                    let state = TokenState {
                        player: *player,
                        token: *token,
                        at: at.clone(),
                    };
                    self.tokens.insert(i, state);
                }
            }
            What::Holding { card, from, to } => {
                let holders = self.holdings.get(card).map_or(&[][..], Vec::as_slice);
                if from.is_none() && to.is_none() {
                    return Err(format!("{card} goes from nobody to nobody"));
                }
                if let Some(from) = from
                    && !holders.contains(from)
                {
                    return Err(format!("{from} doesn't hold {card}"));
                }
                if let Some(to) = to
                    && holders.contains(to)
                {
                    return Err(format!("{to} already holds {card}"));
                }
                let holders = self.holdings.entry(card.clone()).or_default();
                if let Some(from) = from {
                    holders.retain(|h| h != from);
                }
                if let Some(to) = to
                    && let Err(i) = holders.binary_search(to)
                {
                    holders.insert(i, *to);
                }
                if holders.is_empty() {
                    self.holdings.remove(card);
                }
            }
            What::Pile(change) => {
                self.zones.entry(change.pile.clone()).or_default();
                self.piles.insert(change.pile.clone(), change.state.clone());
            }
            What::Landscape { card, prophecy } => {
                if let Err(i) = self.landscapes.binary_search(card) {
                    self.landscapes.insert(i, card.clone());
                }
                if *prophecy {
                    self.prophecy = Some(card.clone());
                }
            }
            What::Notes { notes } => self.notes = notes.clone(),
            What::Derived { costs, types } => {
                for (card, cost) in costs {
                    match cost {
                        Some(cost) => self.costs.insert(card.clone(), *cost),
                        None => self.costs.remove(card),
                    };
                }
                for (card, now) in types {
                    match now {
                        Some(types) => self.types.insert(card.clone(), types.clone()),
                        None => self.types.remove(card),
                    };
                }
            }
            _ => unreachable!("moves are applied above"),
        }
        Ok(())
    }

    /// A turn of `player` begins or is skipped: what it was waiting as stops waiting.
    fn turn_taken(&mut self, player: u8, kind: TurnKind, id: Option<u32>) -> Result<(), String> {
        let extra = match (kind, id) {
            (TurnKind::Extra, Some(id)) => {
                let i = self.waiting(id)?;
                if self.extra_turns[i].player != player {
                    return Err(format!("extra turn {id} is not seat {player}'s"));
                }
                Some(i)
            }
            (TurnKind::Extra, None) => return Err("an extra turn names no extra turn".into()),
            (_, Some(id)) => return Err(format!("only an extra turn is extra turn {id}")),
            (_, None) => None,
        };
        if kind == TurnKind::Final {
            match self.ending.as_mut().map(|e| &mut e.final_round) {
                Some(round) if round.first() == Some(&player) => {
                    round.remove(0);
                }
                _ => return Err(format!("seat {player} has no turn next in the final round")),
            }
        }
        if let Some(i) = extra {
            self.extra_turns.remove(i);
        }
        Ok(())
    }

    /// Where the extra turn `id` is among those waiting.
    fn waiting(&self, id: u32) -> Result<usize, String> {
        (self.extra_turns.iter().position(|x| x.id == id))
            .ok_or_else(|| format!("there is no extra turn {id} waiting"))
    }

    fn set_counter(&mut self, key: &CounterKey, value: i64) {
        if value == 0 {
            self.counters.remove(key);
        } else {
            self.counters.insert(key.clone(), value);
        }
    }

    fn apply_move(&mut self, verb: Verb, m: &Move) -> Result<(), String> {
        if !m.pos.is_empty() && m.pos.len() != m.cards.len() {
            return Err("pos doesn't give one position per card".into());
        }
        if !m.to.standing() && !self.zones.contains_key(&m.to) {
            return Err(format!("there is no zone {}", m.to));
        }
        if verb == Verb::Shuffle && !m.to.ordered() {
            return Err(format!("{} has no order to shuffle into", m.to));
        }
        self.zones.entry(m.to.clone()).or_default();
        let mut done = vec![];
        let moved = self.move_cards(verb, m, &mut done);
        if moved.is_err() {
            self.undo(m, done);
        }
        self.tidy(&m.from);
        self.tidy(&m.to);
        moved
    }

    /// The cards of `m` leave and arrive. What each card did is kept in `done` while a later
    /// step may be refused: not for the last card once it has arrived.
    fn move_cards(&mut self, verb: Verb, m: &Move, done: &mut Vec<Done>) -> Result<(), String> {
        let mark = m.face == Some(Face::Up) && m.to.has_faces();
        let last = m.cards.len().saturating_sub(1);
        if last > 0 {
            done.reserve_exact(m.cards.len());
        }
        for (i, card) in m.cards.iter().enumerate() {
            let said = m.pos.get(i).copied();
            let mut step = self.take(&m.from, card, said, m.from_face)?;
            if mark {
                bag_insert(self.face_up.entry(m.to.clone()).or_default(), card.clone());
            }
            if verb != Verb::Shuffle {
                match self.place(&m.to, card.clone(), m.at) {
                    Ok(at) => step.placed = Some(at),
                    Err(why) => {
                        done.push(step);
                        return Err(why);
                    }
                }
            }
            if i < last || verb == Verb::Shuffle {
                done.push(step);
            }
        }
        if verb == Verb::Shuffle {
            let to = self.zones.get_mut(&m.to).expect("checked by apply_move");
            let at = insertion(to.len(), m.at, &m.to)?;
            to.splice(at..at, m.cards.iter().cloned());
        }
        Ok(())
    }

    /// Takes back what each card of a refused move did, last first.
    fn undo(&mut self, m: &Move, done: Vec<Done>) {
        let mark = m.face == Some(Face::Up) && m.to.has_faces();
        for (step, card) in done.into_iter().zip(&m.cards).rev() {
            if let Some(at) = step.placed {
                self.zones.get_mut(&m.to).expect("placed there").remove(at);
            }
            if mark {
                bag_remove(self.face_up.get_mut(&m.to).expect("marked there"), card);
            }
            let back = if step.hidden {
                CardRef::Hidden
            } else {
                card.clone()
            };
            let from = self.zones.entry(m.from.clone()).or_default();
            from.insert(step.at, back);
            if step.up {
                bag_insert(
                    self.face_up.entry(m.from.clone()).or_default(),
                    card.clone(),
                );
            }
            if step.down {
                bag_insert(
                    self.face_down.entry(m.from.clone()).or_default(),
                    card.clone(),
                );
            }
        }
    }

    /// An empty zone that is always there is not listed.
    fn tidy(&mut self, zone: &Zone) {
        if zone.standing() && self.zone(zone).is_empty() {
            self.zones.remove(zone);
        }
        if self.face_up(zone).is_empty() {
            self.face_up.remove(zone);
        }
        if self.face_down.get(zone).is_some_and(Vec::is_empty) {
            self.face_down.remove(zone);
        }
    }

    /// Takes `card` out of `zone`: in an ordered zone the one under `said` cards (the top
    /// one when not said), in a bag one of its name, lying face up if `up`.
    fn take(
        &mut self,
        zone: &Zone,
        card: &CardRef,
        said: Option<u32>,
        face: Option<Face>,
    ) -> Result<Done, String> {
        let up = face == Some(Face::Up);
        let down_wanted = face == Some(Face::Down) && !zone.has_faces();
        let cards = self
            .zones
            .get_mut(zone)
            .ok_or_else(|| match zone.standing() {
                true => format!("{card} is not in {zone}"),
                false => format!("there is no zone {zone}"),
            })?;
        if zone.ordered() {
            let i = said.unwrap_or(0) as usize;
            let downs = self.face_down.get(zone).map_or(&[][..], Vec::as_slice);
            let down = count(downs, card);
            return match cards.get(i) {
                None => Err(format!("{zone} has no card under {i} others")),
                Some(_) if down_wanted && down == 0 => {
                    Err(format!("no {card} lies face down in {zone}"))
                }
                Some(_) if !down_wanted && down > 0 && count(cards, card) <= down => {
                    Err(format!("no {card} lies face up in {zone}"))
                }
                // A card the viewer sees only as it leaves: one of the face-down ones.
                Some(there) if there == card || *there == CardRef::Hidden => {
                    let hidden = there != card;
                    cards.remove(i);
                    if down_wanted && let Some(downs) = self.face_down.get_mut(zone) {
                        bag_remove(downs, card);
                    }
                    Ok(Done {
                        at: i,
                        hidden,
                        up: false,
                        down: down_wanted,
                        placed: None,
                    })
                }
                Some(there) => Err(format!("{card} is not in {zone}: {there} lies there")),
            };
        }
        if said.is_some() {
            return Err(format!("{zone} has no positions"));
        }
        let had_up = count(self.face_up.get(zone).map_or(&[], Vec::as_slice), card);
        if up && had_up == 0 {
            return Err(format!("no {card} lies face up in {zone}"));
        }
        if !up && had_up > 0 && count(cards, card) <= had_up {
            return Err(format!("no {card} lies face down in {zone}"));
        }
        let (at, hidden) = match cards.binary_search(card) {
            Ok(i) => (i, false),
            Err(_) => match cards.binary_search(&CardRef::Hidden) {
                Ok(i) => (i, true),
                Err(_) => return Err(format!("{card} is not in {zone}")),
            },
        };
        cards.remove(at);
        if up && let Some(ups) = self.face_up.get_mut(zone) {
            bag_remove(ups, card);
        }
        Ok(Done {
            at,
            hidden,
            up,
            down: false,
            placed: None,
        })
    }

    /// Places `card` in `zone` at `at`; the index it went to.
    fn place(&mut self, zone: &Zone, card: CardRef, at: At) -> Result<usize, String> {
        let cards = self.zones.get_mut(zone).expect("checked by apply_move");
        if !zone.ordered() && at == At::Top {
            return Ok(bag_insert_at(cards, card));
        }
        let i = insertion(cards.len(), at, zone)?;
        cards.insert(i, card);
        Ok(i)
    }

    /// The card `card` in `zone` (under exactly `pos` cards) is now called `name`. In a
    /// zone with faces, one lying `face` (absent: face down).
    fn rename(
        &mut self,
        card: &CardRef,
        name: &str,
        zone: &Zone,
        pos: Option<u32>,
        face: Option<Face>,
    ) -> Result<(), String> {
        let now = CardRef::named(name);
        let ups = count(self.face_up(zone), card);
        let cards = (self.zones.get_mut(zone)).ok_or_else(|| format!("there is no zone {zone}"))?;
        if zone.ordered() {
            let i = pos.unwrap_or(0) as usize;
            return match cards.get_mut(i) {
                Some(there) if there == card => {
                    *there = now;
                    Ok(())
                }
                _ => Err(format!("{card} doesn't lie under {i} cards in {zone}")),
            };
        }
        if pos.is_some() {
            return Err(format!("{zone} has no positions"));
        }
        let up = face == Some(Face::Up);
        if up && ups == 0 {
            return Err(format!("no {card} lies face up in {zone}"));
        }
        if !up && count(cards, card) <= ups {
            return Err(format!("no {card} lies face down in {zone}"));
        }
        bag_remove(cards, card);
        bag_insert(cards, now.clone());
        if up && let Some(ups) = self.face_up.get_mut(zone) {
            bag_remove(ups, card);
            bag_insert(ups, now);
        }
        Ok(())
    }

    /// `card` in `zone` (under exactly `pos` cards, in an ordered zone) turns to `face`.
    fn turn_over(
        &mut self,
        card: &CardRef,
        zone: &Zone,
        face: Face,
        pos: Option<u32>,
    ) -> Result<(), String> {
        let pos = match (zone.ordered(), pos) {
            (true, pos) => vec![pos.unwrap_or(0)],
            (false, None) => vec![],
            (false, Some(_)) => return Err(format!("{zone} has no positions")),
        };
        self.lie(zone, std::slice::from_ref(card), &pos)?;
        let (faces, to) = match zone.has_faces() {
            true => (&mut self.face_up, Face::Up),
            false => (&mut self.face_down, Face::Down),
        };
        let marked = faces.entry(zone.clone()).or_default();
        let had = count(marked, card);
        let all = count(self.zones.get(zone).map_or(&[][..], Vec::as_slice), card);
        if face == to {
            if had >= all {
                return Err(format!("every {card} in {zone} lies {face:?} already"));
            }
            bag_insert(marked, card.clone());
        } else if !bag_remove(marked, card) {
            return Err(format!("no {card} in {zone} lies other than {face:?}"));
        }
        self.tidy(zone);
        Ok(())
    }

    /// Whether `cards` lie in `zone`: under `pos` cards each in an ordered zone.
    fn lie(&self, zone: &Zone, cards: &[CardRef], pos: &[u32]) -> Result<(), String> {
        let there = self.zone(zone);
        if zone.ordered() && !pos.is_empty() {
            if pos.len() != cards.len() {
                return Err("pos doesn't give one position per card".into());
            }
            for (card, &i) in cards.iter().zip(pos) {
                if there.get(i as usize) != Some(card) {
                    return Err(format!("{card} doesn't lie under {i} cards in {zone}"));
                }
            }
            return Ok(());
        }
        if !cards.iter().all(|c| count(cards, c) <= count(there, c)) {
            return Err(format!("{zone} hasn't those cards"));
        }
        Ok(())
    }
}

/// What a card of a move did, to take back if a later step is refused.
struct Done {
    /// It left `from` at this index.
    at: usize,
    /// It was a `?` there.
    hidden: bool,
    /// It lay face up there.
    up: bool,
    /// It lay face down in a zone whose cards lie face up.
    down: bool,
    /// The index it arrived at in `to`, once it has.
    placed: Option<usize>,
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

    /// A table folded from a setup with these zones: its next event is 1.
    fn table(zones: &[(&str, &[&str])]) -> Table {
        let setup = Table {
            zones: zones
                .iter()
                .map(|(z, c)| (z.parse::<Zone>().unwrap(), cards(c)))
                .filter(|(z, c)| !z.standing() || !c.is_empty())
                .collect(),
            ..Table::default()
        };
        let mut t = Table::default();
        t.apply(&setup_event(setup)).unwrap();
        t
    }

    fn setup_event(table: Table) -> Event {
        let rules = Rules {
            implementation: "test".into(),
            version: "0".into(),
        };
        let setup = Setup {
            rules,
            seats: vec!["A".into(), "B".into()],
            kingdom: vec![],
            landscapes: vec![],
            table,
            config: None,
            chance: vec![],
        };
        Event {
            seq: 0,
            by: None,
            what: What::Setup(Box::new(setup)),
        }
    }

    fn event(seq: u64, json: &str) -> Event {
        let mut value: serde_json::Value = serde_json::from_str(json).unwrap();
        value["seq"] = seq.into();
        serde_json::from_value(value).unwrap()
    }

    /// `json` as the event `t` takes next.
    fn next(t: &Table, json: &str) -> Event {
        event(t.next_seq(), json)
    }

    /// Why `t` refuses `event`, which leaves it as it was.
    fn refused(t: &Table, event: &Event) -> FoldError {
        let mut after = t.clone();
        let err = (after.apply(event)).expect_err(&serde_json::to_string(event).unwrap());
        assert_eq!(err.seq, event.seq);
        let json = |t: &Table| serde_json::to_string(t).unwrap();
        assert_eq!(json(&after), json(t), "refused, {}", err.why);
        err
    }

    #[test]
    fn cards_and_zones_are_written_as_text() {
        for text in ["?", "Copper", "King's Court", "The Sea's Gift"] {
            assert_eq!(text.parse::<CardRef>().unwrap().to_string(), text);
        }
        for text in [
            "0/hand",
            "3/native-village",
            "pile/Village",
            "aside/Black Market",
            "trash",
            "boons/druid",
            "hexes/discard",
            "1/hexes",
        ] {
            assert_eq!(text.parse::<Zone>().unwrap().to_string(), text);
        }
        for text in [
            "coins",
            "sun",
            "2/vp-tokens",
            "pile/Village (2)/embargo",
            "landscape/Arena/vp",
            "0/sinister-plot",
        ] {
            assert_eq!(text.parse::<CounterKey>().unwrap().to_string(), text);
        }
        assert_eq!(
            "pile/Village/vp".parse::<CounterKey>().unwrap(),
            CounterKey::new(
                CounterOf::Pile(Zone::Pile("Village".into())),
                CounterName::Vp
            )
        );
        assert!("".parse::<CardRef>().is_err());
        for text in ["hand", "0/lap", "pile/", "x/hand", "hexes/druid", "boons/"] {
            assert!(text.parse::<Zone>().is_err(), "{text}");
        }
        for text in ["0/lap", "trash/vp", "landscape//vp", "x/coins"] {
            assert!(text.parse::<CounterKey>().is_err(), "{text}");
        }
    }

    #[test]
    fn an_event_is_one_flat_object() {
        let e = event(
            7,
            r#"{"type":"topdeck","player":1,"cards":["Gold"],"from":"1/hand","to":"1/deck"}"#,
        );
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"seq":7,"type":"topdeck","player":1,"cards":["Gold"],"from":"1/hand","to":"1/deck"}"#
        );
    }

    #[test]
    fn moves_take_and_place_cards_one_after_the_other() {
        let mut t = table(&[
            ("0/deck", &["Copper", "Estate", "Gold"]),
            ("0/hand", &["Silver"]),
            ("0/discard", &[]),
        ]);
        t.apply(&next(&t,
            r#"{"type":"draw","player":0,"cards":["Copper","Estate"],"from":"0/deck","to":"0/hand"}"#,
        ))
        .unwrap();
        // A hand is a bag: sorted, whatever the order the cards came in.
        assert_eq!(
            t.zone(&"0/hand".parse().unwrap()),
            cards(&["Copper", "Estate", "Silver"])
        );
        t.apply(&next(&t,
            r#"{"type":"discard","player":0,"cards":["Silver","Estate"],"from":"0/hand","to":"0/discard"}"#,
        ))
        .unwrap();
        // The last one discarded is on top.
        assert_eq!(
            t.zone(&"0/discard".parse().unwrap()),
            cards(&["Estate", "Silver"])
        );
        t.apply(&next(&t,
            r#"{"type":"insert","player":0,"cards":["Copper"],"from":"0/hand","to":"0/deck","at":{"under":1}}"#,
        ))
        .unwrap();
        assert_eq!(
            t.zone(&"0/deck".parse().unwrap()),
            cards(&["Gold", "Copper"])
        );
    }

    #[test]
    fn a_card_leaves_an_ordered_zone_from_its_position() {
        let mut t = table(&[
            ("0/discard", &["Estate", "Gold", "Copper"]),
            ("0/hand", &[]),
        ]);
        t.apply(&next(&t,
            r#"{"type":"put","player":0,"cards":["Gold"],"from":"0/discard","to":"0/hand","pos":[1]}"#,
        ))
        .unwrap();
        assert_eq!(
            t.zone(&"0/discard".parse().unwrap()),
            cards(&["Estate", "Copper"])
        );
    }

    #[test]
    fn a_shuffle_puts_its_cards_under_the_deck_in_the_order_listed() {
        let mut t = table(&[
            ("0/deck", &["Gold"]),
            ("0/discard", &["Copper", "Estate", "Silver"]),
        ]);
        t.apply(&next(&t,
            r#"{"type":"shuffle","player":0,"cards":["Silver","Copper","Estate"],"from":"0/discard","to":"0/deck","at":"bottom","pos":[2,0,0]}"#,
        ))
        .unwrap();
        assert_eq!(
            t.zone(&"0/deck".parse().unwrap()),
            cards(&["Gold", "Silver", "Copper", "Estate"])
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
        t.apply(&next(
            &t,
            r#"{"type":"draw","player":1,"cards":["?"],"from":"1/deck","to":"1/hand"}"#,
        ))
        .unwrap();
        // Seen as it is played: it was one of the face-down cards in the hand.
        t.apply(&next(
            &t,
            r#"{"type":"play","player":1,"card":"Village","from":"1/hand","to":"1/play"}"#,
        ))
        .unwrap();
        assert_eq!(t.zone(&"1/hand".parse().unwrap()), cards(&["?"]));
        assert_eq!(t.zone(&"1/play".parse().unwrap()), cards(&["Village"]));
        assert_eq!(t.zone(&"1/deck".parse().unwrap()).len(), 2);
    }

    #[test]
    fn a_stream_that_is_not_one_games_is_refused() {
        let t = table(&[("0/deck", &["Copper", "Gold"]), ("0/hand", &[])]);
        for (json, why) in [
            (
                r#"{"type":"draw","player":0,"cards":["Silver"],"from":"0/deck","to":"0/hand"}"#,
                "Silver is not in 0/deck",
            ),
            (
                r#"{"type":"draw","player":0,"cards":["Gold"],"from":"0/deck","to":"0/hand"}"#,
                "Copper lies there",
            ),
            (
                r#"{"type":"draw","player":0,"cards":["Copper"],"from":"0/deck","to":"pile/Gold"}"#,
                "no zone pile/Gold",
            ),
            (
                r#"{"type":"put","player":0,"cards":["Copper"],"from":"0/hand","to":"0/deck"}"#,
                "Copper is not in 0/hand",
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
            (
                r#"{"type":"counter","counter":"pile/Gold/embargo","delta":1,"value":1}"#,
                "no pile pile/Gold",
            ),
            (
                r#"{"type":"turn","player":0,"number":2,"kind":"extra","extra":"Outpost"}"#,
                "names no extra turn",
            ),
            (
                r#"{"type":"setup","rules":{"implementation":"x","version":"0"},"seats":[],"kingdom":[],"table":{"zones":{}}}"#,
                "setup comes only first",
            ),
        ] {
            let err = refused(&t, &next(&t, json));
            assert!(err.why.contains(why), "{}", err.why);
        }
        let draw = r#"{"type":"draw","player":0,"cards":["Copper"],"from":"0/deck","to":"0/hand"}"#;
        for seq in [0, 2] {
            let err = refused(&t, &event(seq, draw));
            assert!(err.why.contains("the next event is 1"), "{}", err.why);
        }
        let err = refused(&Table::default(), &event(0, draw));
        assert!(err.why.contains("begins with setup"), "{}", err.why);
    }

    #[test]
    fn a_cause_is_an_earlier_occurrence() {
        let mut t = table(&[("0/deck", &["Copper", "Gold"]), ("0/hand", &[])]);
        for json in [
            r#"{"type":"turn","player":0,"number":1}"#,
            r#"{"type":"counter","counter":"0/coffers","delta":1,"value":1}"#,
        ] {
            t.apply(&next(&t, json)).unwrap();
        }
        let draw = r#"{"type":"draw","player":0,"cards":["Copper"],"from":"0/deck","to":"0/hand"}"#;
        let resolve = r#"{"type":"resolve","player":0,"card":"Haven","when":"start-turn"}"#;
        for (cause, why) in [
            (0, "no occurrence"),
            (2, "no occurrence"),
            (3, "not earlier"),
        ] {
            let mut e = next(&t, draw);
            e.by = Some(cause);
            assert!(refused(&t, &e).why.contains(why));
            let mut e = next(&t, resolve);
            e.by = Some(1);
            if let What::Resolve { origin, .. } = &mut e.what {
                *origin = Some(cause);
            }
            assert!(refused(&t, &e).why.contains(why));
        }
        let mut e = next(&t, draw);
        e.by = Some(1);
        t.apply(&e).unwrap();
    }

    #[test]
    fn turns_follow_one_another_until_the_game_is_over() {
        let mut t = table(&[]);
        let turn = r#"{"type":"turn","player":0,"number":1}"#;
        t.apply(&next(&t, turn)).unwrap();
        assert!(
            refused(&t, &next(&t, turn))
                .why
                .contains("turn is in progress")
        );
        let skipped = r#"{"type":"turn-skipped","player":1}"#;
        assert!(
            refused(&t, &next(&t, skipped))
                .why
                .contains("turn is in progress")
        );
        for json in [
            r#"{"type":"extra-turn","id":1,"player":1,"card":"Outpost","limit":"third-in-a-row"}"#,
            r#"{"type":"turn-end","player":0}"#,
        ] {
            t.apply(&next(&t, json)).unwrap();
        }
        let theirs = r#"{"type":"turn","player":0,"number":2,"kind":"extra","extra":"Outpost","extraTurn":1}"#;
        assert!(refused(&t, &next(&t, theirs)).why.contains("not seat 0's"));
        let regular = r#"{"type":"turn","player":1,"number":1,"extraTurn":1}"#;
        assert!(
            refused(&t, &next(&t, regular))
                .why
                .contains("only an extra turn")
        );
        let end = r#"{"type":"game-end","reason":"resigned","result":[]}"#;
        t.apply(&next(&t, end)).unwrap();
        assert!(
            refused(&t, &next(&t, turn))
                .why
                .contains("the game is over")
        );
    }

    #[test]
    fn a_refused_move_takes_nothing() {
        let t = table(&[
            ("0/deck", &["Copper", "Gold", "Estate"]),
            ("0/hand", &["Silver"]),
            ("0/aside", &[]),
        ]);
        for json in [
            // The third card isn't where the move says, once the first two have left.
            r#"{"type":"draw","player":0,"cards":["Copper","Gold","Gold"],"from":"0/deck","to":"0/hand"}"#,
            r#"{"type":"set-aside","player":0,"cards":["Copper","Estate"],"from":"0/deck","to":"0/aside","face":"up","pos":[0,0]}"#,
            r#"{"type":"insert","player":0,"cards":["Silver","Copper"],"from":"0/hand","to":"0/deck","at":{"under":4}}"#,
            r#"{"type":"shuffle","player":0,"cards":["Copper","Gold","Estate","Silver"],"from":"0/deck","to":"0/deck"}"#,
        ] {
            refused(&t, &next(&t, json));
        }
    }

    #[test]
    fn a_final_round_is_taken_in_its_order_and_a_turn_end_clears_the_turn() {
        let mut t = table(&[]);
        for json in [
            r#"{"type":"game-ending","reason":"provinces","finalRound":[1,2]}"#,
            r#"{"type":"turn-skipped","player":1,"kind":"final"}"#,
            r#"{"type":"turn","player":2,"number":7,"kind":"final"}"#,
        ] {
            t.apply(&next(&t, json)).unwrap();
        }
        assert!(t.ending.as_ref().unwrap().final_round.is_empty());
        assert_eq!(t.counter(&CounterKey::global(CounterName::NextHand)), 5);
        t.apply(&next(&t, r#"{"type":"turn-end","player":2}"#))
            .unwrap();
        assert_eq!(
            (
                t.turn.as_ref(),
                t.counter(&CounterKey::global(CounterName::Actions))
            ),
            (None, 0)
        );
    }

    #[test]
    fn a_card_set_aside_face_up_is_face_up_until_it_moves() {
        let mut t = table(&[("0/hand", &["Gold", "Gold"])]);
        let aside: Zone = "0/aside".parse().unwrap();
        for json in [
            r#"{"type":"set-aside","player":0,"cards":["Gold"],"from":"0/hand","to":"0/aside","face":"up"}"#,
            r#"{"type":"set-aside","player":0,"cards":["Gold"],"from":"0/hand","to":"0/aside","face":"down"}"#,
        ] {
            t.apply(&next(&t, json)).unwrap();
        }
        assert_eq!(t.zone(&aside), cards(&["Gold", "Gold"]));
        assert_eq!(t.face_up(&aside), cards(&["Gold"]));
        // Of two Golds lying different ways, the move says which way the one leaving lay.
        let up = r#"{"type":"put","player":0,"cards":["Gold"],"from":"0/aside","to":"0/hand","fromFace":"up"}"#;
        let mut first = t.clone();
        first.apply(&next(&first, up)).unwrap();
        assert!(first.face_up.is_empty());
        let err = refused(&first, &next(&first, up));
        assert!(err.why.contains("no Gold lies face up"), "{}", err.why);
        let down = r#"{"type":"put","player":0,"cards":["Gold"],"from":"0/aside","to":"0/hand"}"#;
        t.apply(&next(&t, down)).unwrap();
        assert_eq!(t.face_up(&aside), cards(&["Gold"]));
        let err = refused(&t, &next(&t, down));
        assert!(err.why.contains("no Gold lies face down"), "{}", err.why);
    }

    #[test]
    fn a_renamed_card_is_found_where_it_lies() {
        let mut t = table(&[("0/deck", &["Copper", "Estate", "Copper"])]);
        t.apply(&next(
            &t,
            r#"{"type":"rename","card":"Copper","name":"Estate","zone":"0/deck","pos":2}"#,
        ))
        .unwrap();
        assert_eq!(
            t.zone(&"0/deck".parse().unwrap()),
            cards(&["Copper", "Estate", "Estate"])
        );
        let err = refused(
            &t,
            &next(
                &t,
                r#"{"type":"rename","card":"Copper","name":"Estate","zone":"0/deck","pos":1}"#,
            ),
        );
        assert!(err.why.contains("doesn't lie under 1"), "{}", err.why);
    }

    #[test]
    fn a_turn_starts_with_one_action_and_one_buy() {
        let mut t = table(&[]);
        t.counters.insert(CounterKey::global(CounterName::Coins), 5);
        t.apply(&next(
            &t,
            r#"{"type":"turn","player":1,"number":4,"extra":"Outpost"}"#,
        ))
        .unwrap();
        t.apply(&next(&t, r#"{"type":"phase","phase":"action"}"#))
            .unwrap();
        t.apply(&next(
            &t,
            r#"{"type":"counter","counter":"actions","delta":2,"value":3}"#,
        ))
        .unwrap();
        let turn = t.turn.as_ref().unwrap();
        assert_eq!(
            (turn.player, turn.number, turn.phase),
            (1, 4, Phase::Action)
        );
        assert_eq!(turn.extra.as_deref(), Some("Outpost"));
        let counter = |name| t.counter(&CounterKey::global(name));
        assert_eq!(
            (
                counter(CounterName::Actions),
                counter(CounterName::Buys),
                counter(CounterName::Coins)
            ),
            (3, 1, 0)
        );
    }

    #[test]
    fn holdings_tokens_and_derived_values_fold() {
        let mut t = table(&[]);
        t.apply(&next(
            &t,
            r#"{"type":"pile","pile":"pile/Village","card":"Village"}"#,
        ))
        .unwrap();
        for json in [
            r#"{"type":"holding","card":"Flag","to":0}"#,
            r#"{"type":"holding","card":"Flag","from":0,"to":1}"#,
            r#"{"type":"holding","card":"Deluded","to":1}"#,
            r#"{"type":"holding","card":"Deluded","to":0}"#,
            r#"{"type":"token","player":0,"token":"plus-card","at":"pile/Village"}"#,
            r#"{"type":"token","player":0,"token":"journey","at":"face-down"}"#,
            r#"{"type":"token","player":0,"token":"journey"}"#,
            r#"{"type":"derived","costs":{"Village":{"coins":2}},"types":{"Copper":["Treasure","Action"]}}"#,
            r#"{"type":"derived","types":{"Copper":null}}"#,
        ] {
            t.apply(&next(&t, json)).unwrap();
        }
        assert_eq!(t.holdings["Flag"], [1]);
        assert_eq!(t.holdings["Deluded"], [0, 1]);
        assert_eq!(t.tokens.len(), 1);
        assert_eq!(
            t.tokens[0].at,
            TokenAt::Pile("pile/Village".parse().unwrap())
        );
        assert_eq!(t.costs["Village"].coins, 2);
        assert!(t.types.is_empty());
        for (json, why) in [
            (
                r#"{"type":"holding","card":"Flag","from":0}"#,
                "0 doesn't hold Flag",
            ),
            (
                r#"{"type":"holding","card":"Deluded","to":0}"#,
                "already holds",
            ),
            (
                r#"{"type":"holding","card":"Flag","from":1,"to":1}"#,
                "already holds",
            ),
            (
                r#"{"type":"token","player":1,"token":"plus-buy","at":"pile/Smithy"}"#,
                "no pile pile/Smithy",
            ),
        ] {
            let err = refused(&t, &next(&t, json));
            assert!(err.why.contains(why), "{}", err.why);
        }
    }

    #[test]
    fn a_card_turned_face_down_in_the_trash_leaves_it_face_down() {
        let mut t = table(&[("trash", &["Village", "Smithy", "Village"])]);
        let trash = Zone::Trash;
        t.apply(&next(
            &t,
            r#"{"type":"face","card":"Village","zone":"trash","face":"down","pos":2}"#,
        ))
        .unwrap();
        assert_eq!(t.face_down[&trash], cards(&["Village"]));
        let err = refused(
            &t,
            &next(
                &t,
                r#"{"type":"face","card":"Smithy","zone":"trash","face":"down"}"#,
            ),
        );
        assert!(
            err.why.contains("Smithy doesn't lie under 0"),
            "{}",
            err.why
        );
        let down = r#"{"type":"gain","player":0,"cards":["Village"],"from":"trash","to":"0/discard","fromFace":"down"}"#;
        let up =
            r#"{"type":"gain","player":0,"cards":["Village"],"from":"trash","to":"0/discard"}"#;
        let mut gained = t.clone();
        gained.apply(&next(&gained, down)).unwrap();
        assert!(gained.face_down.is_empty());
        let err = refused(&gained, &next(&gained, down));
        assert!(err.why.contains("no Village lies face down"), "{}", err.why);
        t.apply(&next(&t, up)).unwrap();
        assert_eq!(t.face_down[&trash], cards(&["Village"]));
        let err = refused(
            &t,
            &next(
                &t,
                r#"{"type":"gain","player":0,"cards":["Village"],"from":"trash","to":"0/discard","pos":[1]}"#,
            ),
        );
        assert!(err.why.contains("no Village lies face up"), "{}", err.why);
        t.apply(&next(
            &t,
            r#"{"type":"face","card":"Village","zone":"trash","face":"up","pos":1}"#,
        ))
        .unwrap();
        assert!(t.face_down.is_empty());
    }

    #[test]
    fn a_rename_says_which_card() {
        let mut t = table(&[
            ("0/deck", &["Copper", "Copper", "Estate"]),
            ("0/hand", &["Copper", "Copper"]),
        ]);
        for json in [
            r#"{"type":"set-aside","player":0,"cards":["Copper"],"from":"0/hand","to":"0/aside","face":"up"}"#,
            r#"{"type":"set-aside","player":0,"cards":["Copper"],"from":"0/hand","to":"0/aside","face":"down"}"#,
            r#"{"type":"rename","card":"Copper","name":"Estate","zone":"0/aside","face":"up"}"#,
            r#"{"type":"rename","card":"Copper","name":"Silver","zone":"0/deck","pos":1}"#,
        ] {
            t.apply(&next(&t, json)).unwrap();
        }
        let aside: Zone = "0/aside".parse().unwrap();
        assert_eq!(t.zone(&aside), cards(&["Copper", "Estate"]));
        assert_eq!(t.face_up(&aside), cards(&["Estate"]));
        assert_eq!(
            t.zone(&"0/deck".parse().unwrap()),
            cards(&["Copper", "Silver", "Estate"])
        );
        let err = refused(
            &t,
            &next(
                &t,
                r#"{"type":"rename","card":"Copper","name":"Gold","zone":"0/aside","face":"up"}"#,
            ),
        );
        assert!(err.why.contains("no Copper lies face up"), "{}", err.why);
    }

    #[test]
    fn kept_cards_are_cards_of_a_play_area() {
        let mut t = table(&[("0/play", &["Wharf", "Wharf"])]);
        let play: Zone = "0/play".parse().unwrap();
        t.apply(&next(
            &t,
            r#"{"type":"kept","zone":"0/play","cards":["Wharf"]}"#,
        ))
        .unwrap();
        assert_eq!(t.kept[&play], cards(&["Wharf"]));
        for (json, why) in [
            (
                r#"{"type":"kept","zone":"0/hand","cards":[]}"#,
                "no play area",
            ),
            (
                r#"{"type":"kept","zone":"0/play","cards":["Wharf","Wharf","Wharf"]}"#,
                "hasn't the cards kept",
            ),
        ] {
            let err = refused(&t, &next(&t, json));
            assert!(err.why.contains(why), "{}", err.why);
        }
        t.apply(&next(&t, r#"{"type":"kept","zone":"0/play","cards":[]}"#))
            .unwrap();
        assert!(t.kept.is_empty());
    }

    #[test]
    fn empty_lists_that_mean_something_are_written() {
        let notes = What::Notes { notes: vec![] };
        assert_eq!(
            serde_json::to_string(&notes).unwrap(),
            r#"{"type":"notes","notes":[]}"#
        );
        assert!(serde_json::from_str::<What>(r#"{"type":"notes"}"#).is_err());
        let none = What::Answer(Answer {
            seat: 0,
            indices: Some(vec![]),
            name: None,
        });
        let json = serde_json::to_string(&none).unwrap();
        assert_eq!(json, r#"{"type":"answer","seat":0,"indices":[]}"#);
        assert_eq!(serde_json::from_str::<What>(&json).unwrap(), none);
    }
}

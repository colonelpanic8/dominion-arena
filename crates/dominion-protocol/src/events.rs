//! Serde types for `dominion/events@1`: a game of Dominion as a stream of events, and the
//! [`Table`] they describe. The table after event *n* is the fold of events `0..=n`
//! ([`Table::apply`]); no rules engine is needed to follow a game.

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
    Seen {
        name: String,
        handle: u32,
    },
    /// A card that is no physical card and the only one of its name: a Boon or Hex,
    /// written by its name alone.
    Named(String),
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
            CardRef::Seen { name, .. } | CardRef::Named(name) => Some(name),
        }
    }

    pub fn handle(&self) -> Option<u32> {
        match self {
            CardRef::Seen { handle, .. } => Some(*handle),
            CardRef::Hidden | CardRef::Named(_) => None,
        }
    }
}

impl fmt::Display for CardRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CardRef::Hidden => f.write_str("?"),
            CardRef::Seen { name, handle } => write!(f, "{name}#{handle}"),
            CardRef::Named(name) => f.write_str(name),
        }
    }
}

impl FromStr for CardRef {
    type Err = String;

    fn from_str(s: &str) -> Result<CardRef, String> {
        if s == "?" {
            return Ok(CardRef::Hidden);
        }
        let bad = || format!("{s:?} is neither \"?\", \"Name#handle\" nor \"Name\"");
        let Some((name, handle)) = s.rsplit_once('#') else {
            return match s.is_empty() {
                true => Err(bad()),
                false => Ok(CardRef::Named(s.to_string())),
            };
        };
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
    /// Whether its cards have positions: `cards[0]` is the top. The others list their
    /// cards in the order they arrived.
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
    Take,
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
/// 1 Buy, $0 and no Potions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStart {
    pub player: u8,
    /// Which of that player's turns it is, from 1, extra turns not counted.
    pub number: u32,
    /// The card that gave it, for an extra turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
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
}

impl CounterName {
    const ALL: [CounterName; 16] = [
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
        }
    }

    fn of_turn(self) -> bool {
        matches!(
            self,
            CounterName::Actions | CounterName::Buys | CounterName::Coins | CounterName::Potions
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
    /// third turn in a row. `extra` is the card that gave it, for an extra turn.
    TurnSkipped {
        player: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        lost: bool,
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
    /// The table's notes are now `notes`.
    Notes {
        notes: Vec<Note>,
    },
    /// What the rules derive changed: what cards cost now, and their types where they are
    /// not the printed ones (`null`: printed again).
    Derived {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        costs: BTreeMap<String, Cost>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        types: BTreeMap<String, Option<Vec<String>>>,
    },
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
}

/// What the events describe: where every card is, and every counter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// Every zone that exists, empty piles included. An ordered zone's cards are top
    /// first; another's are in the order they arrived.
    pub zones: BTreeMap<Zone, Vec<CardRef>>,
    /// Counters that are not zero.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counters: BTreeMap<CounterKey, i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<GameEnd>,
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
}

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
            | What::Choose(_)
            | What::TurnSkipped { .. } => {}
            What::Turn(t) => {
                self.turn = Some(TurnState {
                    player: t.player,
                    number: t.number,
                    extra: t.extra.clone(),
                    phase: Phase::Start,
                });
                for (name, value) in [
                    (CounterName::Actions, 1),
                    (CounterName::Buys, 1),
                    (CounterName::Coins, 0),
                    (CounterName::Potions, 0),
                ] {
                    self.set_counter(&CounterKey::global(name), value);
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
            What::Receive { .. } => {}
            What::Token { player, token, at } => {
                self.tokens
                    .retain(|t| (t.player, t.token) != (*player, *token));
                if let Some(at) = at {
                    if let TokenAt::Pile(pile) = at
                        && !self.piles.contains_key(pile)
                    {
                        return Err(fail(format!("there is no pile {pile}")));
                    }
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
                let holders = self.holdings.entry(card.clone()).or_default();
                if let Some(from) = from {
                    let i = (holders.iter().position(|h| h == from))
                        .ok_or_else(|| fail(format!("{from} doesn't hold {card}")))?;
                    holders.remove(i);
                }
                if let Some(to) = to {
                    match holders.binary_search(to) {
                        Ok(_) => return Err(fail(format!("{to} already holds {card}"))),
                        Err(i) => holders.insert(i, *to),
                    }
                }
                if from.is_none() && to.is_none() {
                    return Err(fail(format!("{card} goes from nobody to nobody")));
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
                    self.costs.insert(card.clone(), *cost);
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
        self.zones.entry(m.to.clone()).or_default();
        let mut block = vec![];
        for (i, card) in m.cards.iter().enumerate() {
            let said = m.pos.get(i).copied();
            self.take(&m.from, card, said)?;
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
        let found = match card {
            CardRef::Seen { handle, .. } => cards.iter().position(|c| c.handle() == Some(*handle)),
            CardRef::Named(_) => cards.iter().position(|c| c == card),
            CardRef::Hidden => None,
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
        for text in ["?", "Copper#12", "King's Court#0", "The Sea's Gift"] {
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
        for text in ["", "#3", "Copper#x"] {
            assert!(text.parse::<CardRef>().is_err(), "{text}");
        }
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
        ] {
            let err = t.clone().apply(&event(5, json)).unwrap_err();
            assert_eq!(err.seq, 5);
            assert!(err.why.contains(why), "{}", err.why);
        }
    }

    #[test]
    fn a_turn_starts_with_one_action_and_one_buy() {
        let mut t = table(&[]);
        t.counters.insert(CounterKey::global(CounterName::Coins), 5);
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
        let mut t = table(&[("pile/Village", &[])]);
        t.piles
            .insert("pile/Village".parse().unwrap(), PileState::default());
        for (seq, json) in [
            r#"{"type":"holding","card":"Flag","to":0}"#,
            r#"{"type":"holding","card":"Flag","from":0,"to":1}"#,
            r#"{"type":"holding","card":"Deluded","to":1}"#,
            r#"{"type":"holding","card":"Deluded","to":0}"#,
            r#"{"type":"token","player":0,"token":"plus-card","at":"pile/Village"}"#,
            r#"{"type":"token","player":0,"token":"journey","at":"face-down"}"#,
            r#"{"type":"token","player":0,"token":"journey"}"#,
            r#"{"type":"derived","costs":{"Village":{"coins":2}},"types":{"Copper":["Treasure","Action"]}}"#,
            r#"{"type":"derived","types":{"Copper":null}}"#,
        ]
        .into_iter()
        .enumerate()
        {
            t.apply(&event(seq as u64, json)).unwrap();
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
                r#"{"type":"token","player":1,"token":"plus-buy","at":"pile/Smithy"}"#,
                "no pile pile/Smithy",
            ),
        ] {
            let err = t.clone().apply(&event(9, json)).unwrap_err();
            assert!(err.why.contains(why), "{}", err.why);
        }
    }
}

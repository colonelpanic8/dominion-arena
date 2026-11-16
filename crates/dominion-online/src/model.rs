//! Raw server model for the hash-pinned 2.3.7 protocol. Allocation identities are
//! not a player-visible view; consumers must apply the visibility reducer.
use crate::{Error, Result, questions::LogArgument, wire::Reader};
use serde::{Deserialize, Serialize};

fn ordinal(r: &mut Reader<'_>, n: i32, name: &str) -> Result<i32> {
    let value = r.int()?;
    if !(0..n).contains(&value) {
        return Err(Error::Protocol(format!("unknown {name} ordinal {value}")));
    }
    Ok(value)
}
fn card_name(r: &mut Reader<'_>) -> Result<i32> {
    ordinal(r, 881, "CardNames")
}
fn types(r: &mut Reader<'_>) -> Result<Vec<i32>> {
    list(r, |r| ordinal(r, 48, "Types"))
}
fn names(r: &mut Reader<'_>) -> Result<Vec<i32>> {
    list(r, card_name)
}
fn list<T>(
    r: &mut Reader<'_>,
    mut parse: impl FnMut(&mut Reader<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let count = r.count()?;
    (0..count).map(|_| parse(r)).collect()
}
fn booleans(r: &mut Reader<'_>) -> Result<Vec<bool>> {
    list(r, |r| r.boolean())
}
fn same_lengths(name: &str, expected: usize, lengths: &[usize]) -> Result<()> {
    if lengths.iter().any(|n| *n != expected) {
        return Err(Error::Protocol(format!(
            "{name} parallel arrays have different lengths"
        )));
    }
    Ok(())
}
fn int_map(r: &mut Reader<'_>, bound: i32, name: &str) -> Result<Vec<(i32, i32)>> {
    list(r, |r| Ok((ordinal(r, bound, name)?, r.int()?)))
}
macro_rules! raw {
    ($name:ident { $($field:ident : $ty:ty = $read:expr),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        pub struct $name { $(pub $field: $ty),* }
        impl $name { pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
            Ok(Self { $($field: ($read)(r)?),* })
        } }
    };
}
raw!(CardFrequency {
    card_name: i32 = card_name,
    frequency: i32 = Reader::int
});
raw!(Reminder {
    card_name: i32 = card_name,
    blinking: bool = Reader::boolean,
    frequency: i32 = Reader::int
});
raw!(PileMarker {
    kind: i32 = |r| ordinal(r, 5, "PileMarkerTypes"),
    zone_index: i32 = Reader::int
});
raw!(Player { player_id:i32=Reader::int, card_frequencies:Vec<CardFrequency>=|r| list(r,CardFrequency::parse), reminders:Vec<Reminder>=|r| list(r,Reminder::parse), bought_projects:Vec<i32>=names });
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub id: i32,
    pub card_name: i32,
    pub pile_name: i32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub index: i32,
    pub kind: i32,
    pub pile_name: i32,
    pub owner: i32,
    pub cards: Vec<i32>,
    pub created_by: i32,
    pub attached_trait: i32,
    pub is_visible: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub kind: i32,
    pub owner: i32,
    pub zone: i32,
    pub is_flipped: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Counter {
    pub index: i32,
    pub kind: i32,
    pub owner: i32,
    pub value: i32,
    pub zone: i32,
    pub card: i32,
    pub is_visible: bool,
}
raw!(TurnDescription {
    owner_id: i32 = Reader::int,
    turn_number: i32 = Reader::int,
    turn_type: i32 = |r| ordinal(r, 10, "ExtraTurnTypes"),
    controller_id: i32 = Reader::int
});
raw!(Cost {
    coins: i32 = Reader::int,
    potions: i32 = Reader::int,
    debt: i32 = Reader::int
});
raw!(CostReduction {
    generic:i32=Reader::int,
    by_type:Vec<(i32,i32)>=|r| int_map(r,48,"Types"),
    by_card:Vec<(i32,i32)>=|r| int_map(r,881,"CardNames"),
    cost_override:Vec<(i32,Cost)>=|r| list(r,|r| Ok((card_name(r)?,Cost::parse(r)?))),
});
raw!(CardMove { from_zone:i32=Reader::int, to_zone:i32=Reader::int, before:Vec<i32>=Reader::ints, after:Vec<i32>=Reader::ints, movement_type:i32=|r| ordinal(r,25,"MovementTypes"), animation_class:i32=|r| ordinal(r,7,"AnimationClassIds") });
raw!(CounterChange {
    index: i32 = Reader::int,
    new_value: i32 = Reader::int
});
raw!(PileUpdate {
    index: i32 = Reader::int,
    top_card_id: i32 = Reader::int
});
raw!(Shuffle {
    owner: i32 = Reader::int,
    include_discard: bool = Reader::boolean
});
raw!(Inspection { player:i32=Reader::int, cards:Vec<i32>=Reader::ints, from_zone:i32=Reader::int, to_zone:i32=Reader::int, is_public:bool=Reader::boolean, linked_to_question:bool=Reader::boolean });
raw!(TokenChange {
    token: i32 = Reader::int,
    zone: i32 = Reader::int,
    is_flipped: bool = Reader::boolean
});
raw!(ZoneCreated {
    index: i32 = Reader::int,
    owner: i32 = Reader::int,
    kind: i32 = |r| ordinal(r, 35, "ZoneNames"),
    created_by: i32 = Reader::int
});
raw!(ZoneDestroyed {
    index: i32 = Reader::int
});
raw!(Rename {
    id: i32 = Reader::int,
    card_name: i32 = card_name
});
raw!(BehaviourChange {
    id: i32 = Reader::int,
    card_name: i32 = card_name
});
raw!(PilesStatus {
    draw_index: i32 = Reader::int,
    draw_size: i32 = Reader::int,
    discard_index: i32 = Reader::int,
    discard_size: i32 = Reader::int,
    top_card_id: i32 = Reader::int
});
raw!(RemindersChange { player:i32=Reader::int, reminders:Vec<Reminder>=|r| list(r,Reminder::parse) });
raw!(ImageChange {
    id: i32 = Reader::int,
    card_name: i32 = card_name
});
raw!(Association { card:i32=Reader::int, linked_cards:Vec<i32>=Reader::ints });
raw!(CardStatus { card:i32=Reader::int, stays_out:bool=Reader::boolean, played_card_lists:Vec<Vec<i32>>=|r| list(r,|r| r.ints()), set_aside_card_lists:Vec<Vec<i32>>=|r| list(r,|r| r.ints()), properties:Vec<i32>=|r| list(r,|r| ordinal(r,3,"CardProperties")) });
raw!(ProjectBought {
    player: i32 = Reader::int,
    project: i32 = card_name
});
raw!(ChangeCardTypes { cards:Vec<i32>=Reader::ints, added:Vec<i32>=types, removed:Vec<i32>=types });
raw!(PileReorder { pile:i32=Reader::int, cards:Vec<i32>=Reader::ints });
raw!(ProjectRemoved {
    player: i32 = Reader::int,
    project: i32 = card_name
});
raw!(CardTypeChange { id:i32=Reader::int, added:Vec<i32>=types });
raw!(TemporaryEffects { renames:Vec<Rename>=|r| list(r,Rename::parse), behaviour_changes:Vec<BehaviourChange>=|r| list(r,BehaviourChange::parse), image_changes:Vec<ImageChange>=|r| list(r,ImageChange::parse), card_type_changes:Vec<CardTypeChange>=|r| list(r,CardTypeChange::parse) });
raw!(AutoplaySetting {
    card_name: i32 = card_name,
    value: i32 = Reader::int
});
raw!(TimerSettings { settings:Vec<(i32,i64)>=|r| list(r,|r| Ok((ordinal(r,29,"TimeSettingsParameters")?,r.long()?))), force_resign:bool=Reader::boolean, preset:i32=|r| ordinal(r,8,"TimerPresetOptions") });
raw!(TableOptions {
    players_see_spectator_chat: bool = Reader::boolean,
    timer_settings: TimerSettings = TimerSettings::parse
});
raw!(Snapshot { reconnect:bool=Reader::boolean, game_id:i64=Reader::long, state:GameState=GameState::parse, autoplay_settings:Vec<AutoplaySetting>=|r| list(r,AutoplaySetting::parse), table_options:TableOptions=TableOptions::parse, events:Vec<Event>=|r| list(r,Event::parse) });
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameState {
    pub players: Vec<Player>,
    pub card_names: Vec<i32>,
    pub pile_names: Vec<i32>,
    pub card_counts: Vec<i32>,
    pub cards: Vec<Card>,
    pub zones: Vec<Zone>,
    pub pile_markers: Vec<PileMarker>,
    pub tokens: Vec<Token>,
    pub counters: Vec<Counter>,
    pub turn: TurnDescription,
    pub cost_reduction: CostReduction,
    pub temporary_effects: TemporaryEffects,
    pub associations: Vec<Association>,
    pub card_statuses: Vec<CardStatus>,
}
impl GameState {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let players = list(r, Player::parse)?;
        let card_names = names(r)?;
        let pile_names = names(r)?;
        let card_counts = r.ints()?;
        same_lengths(
            "cards",
            card_names.len(),
            &[pile_names.len(), card_counts.len()],
        )?;
        let mut cards = Vec::new();
        for ((card_name, pile_name), count) in card_names.iter().zip(&pile_names).zip(&card_counts)
        {
            let count = usize::try_from(*count)
                .map_err(|_| Error::Protocol("negative card allocation count".into()))?;
            if count > 65536 || cards.len() + count > 65536 {
                return Err(Error::Protocol("too many allocated cards".into()));
            }
            for _ in 0..count {
                cards.push(Card {
                    id: cards.len() as i32,
                    card_name: *card_name,
                    pile_name: *pile_name,
                });
            }
        }
        let indices = r.ints()?;
        let kinds = list(r, |r| ordinal(r, 35, "ZoneNames"))?;
        let zone_names = names(r)?;
        let owners = r.ints()?;
        let members = list(r, |r| r.ints())?;
        let creators = r.ints()?;
        let zone_piles = names(r)?;
        let visible = booleans(r)?;
        let n = members.len();
        same_lengths(
            "zones",
            n,
            &[
                indices.len(),
                kinds.len(),
                zone_names.len(),
                owners.len(),
                creators.len(),
                zone_piles.len(),
                visible.len(),
            ],
        )?;
        let zones = (0..n)
            .map(|i| Zone {
                index: indices[i],
                kind: kinds[i],
                pile_name: zone_names[i],
                owner: owners[i],
                cards: members[i].clone(),
                created_by: creators[i],
                attached_trait: zone_piles[i],
                is_visible: visible[i],
            })
            .collect();
        let pile_markers = list(r, PileMarker::parse)?;
        let token_kinds = list(r, |r| ordinal(r, 9, "TokenNames"))?;
        let token_owners = r.ints()?;
        let token_zones = r.ints()?;
        let flipped = booleans(r)?;
        let n = token_kinds.len();
        same_lengths(
            "tokens",
            n,
            &[token_owners.len(), token_zones.len(), flipped.len()],
        )?;
        let tokens = (0..n)
            .map(|i| Token {
                kind: token_kinds[i],
                owner: token_owners[i],
                zone: token_zones[i],
                is_flipped: flipped[i],
            })
            .collect();
        let counter_indices = r.ints()?;
        let counter_kinds = list(r, |r| ordinal(r, 19, "CounterNames"))?;
        let counter_owners = r.ints()?;
        let values = r.ints()?;
        let counter_zones = r.ints()?;
        let counter_cards = r.ints()?;
        let visible = booleans(r)?;
        let n = counter_kinds.len();
        same_lengths(
            "counters",
            n,
            &[
                counter_indices.len(),
                counter_owners.len(),
                values.len(),
                counter_zones.len(),
                counter_cards.len(),
                visible.len(),
            ],
        )?;
        let counters = (0..n)
            .map(|i| Counter {
                index: counter_indices[i],
                kind: counter_kinds[i],
                owner: counter_owners[i],
                value: values[i],
                zone: counter_zones[i],
                card: counter_cards[i],
                is_visible: visible[i],
            })
            .collect();
        Ok(Self {
            players,
            card_names,
            pile_names,
            card_counts,
            cards,
            zones,
            pile_markers,
            tokens,
            counters,
            turn: TurnDescription::parse(r)?,
            cost_reduction: CostReduction::parse(r)?,
            temporary_effects: TemporaryEffects::parse(r)?,
            associations: list(r, Association::parse)?,
            card_statuses: list(r, CardStatus::parse)?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    CardMove(CardMove),
    CounterChange(CounterChange),
    PileUpdate(PileUpdate),
    TurnDescription(TurnDescription),
    Shuffle(Shuffle),
    Inspection(Inspection),
    CostReduction(CostReduction),
    TokenChange(TokenChange),
    ZoneCreated(ZoneCreated),
    ZoneDestroyed(ZoneDestroyed),
    Rename(Rename),
    BehaviourChange(BehaviourChange),
    PilesStatus(PilesStatus),
    RemindersChange(RemindersChange),
    ImageChange(ImageChange),
    Association(Association),
    CardStatus(CardStatus),
    ProjectBought(ProjectBought),
    ChangeCardTypes(ChangeCardTypes),
    PileReorder(PileReorder),
    ProjectRemoved(ProjectRemoved),
}
impl Event {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(match r.int()? {
            0 => {
                let movement = CardMove::parse(r)?;
                same_lengths(
                    "card movement",
                    movement.before.len(),
                    &[movement.after.len()],
                )?;
                Self::CardMove(movement)
            }
            1 => Self::CounterChange(CounterChange::parse(r)?),
            2 => Self::PileUpdate(PileUpdate::parse(r)?),
            3 => Self::TurnDescription(TurnDescription::parse(r)?),
            4 => Self::Shuffle(Shuffle::parse(r)?),
            5 => Self::Inspection(Inspection::parse(r)?),
            6 => Self::CostReduction(CostReduction::parse(r)?),
            7 => Self::TokenChange(TokenChange::parse(r)?),
            8 => Self::ZoneCreated(ZoneCreated::parse(r)?),
            9 => Self::ZoneDestroyed(ZoneDestroyed::parse(r)?),
            10 => Self::Rename(Rename::parse(r)?),
            11 => Self::BehaviourChange(BehaviourChange::parse(r)?),
            12 => Self::PilesStatus(PilesStatus::parse(r)?),
            13 => Self::RemindersChange(RemindersChange::parse(r)?),
            14 => Self::ImageChange(ImageChange::parse(r)?),
            15 => Self::Association(Association::parse(r)?),
            16 => Self::CardStatus(CardStatus::parse(r)?),
            17 => Self::ProjectBought(ProjectBought::parse(r)?),
            18 => Self::ChangeCardTypes(ChangeCardTypes::parse(r)?),
            19 => Self::PileReorder(PileReorder::parse(r)?),
            20 => Self::ProjectRemoved(ProjectRemoved::parse(r)?),
            n => return Err(Error::Protocol(format!("unknown game event ordinal {n}"))),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogEntry {
    Named {
        name: i32,
        depth: i32,
        arguments: Vec<LogArgument>,
    },
    Decision {
        decision_index: i32,
        player: i32,
        decision: Vec<i32>,
        autoplayed: bool,
    },
}
impl LogEntry {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        match r.int()? {
            0 => Self::parse_named(r),
            1 => Ok(Self::Decision {
                decision_index: r.int()?,
                player: r.int()?,
                decision: r.ints()?,
                autoplayed: r.boolean()?,
            }),
            n => Err(Error::Protocol(format!("unknown log entry type {n}"))),
        }
    }
    fn parse_named(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self::Named {
            name: ordinal(r, 244, "LogEntryNames")?,
            depth: r.int()?,
            arguments: list(r, LogArgument::parse)?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogChange {
    pub start_index: i32,
    pub entries: Vec<LogEntry>,
}
impl LogChange {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let start_index = r.int()?;
        if start_index < 0 {
            return Err(Error::Protocol("negative log start index".into()));
        }
        Ok(Self {
            start_index,
            entries: list(r, LogEntry::parse)?,
        })
    }
}
raw!(Ticker {
    entry: LogEntry = LogEntry::parse,
    is_start_game: bool = Reader::boolean
});
raw!(ScorePart {
    card_name: i32 = card_name,
    points: i32 = Reader::int,
    frequency: i32 = Reader::int,
    reserved: i32 = Reader::int,
    explanation: LogEntry = LogEntry::parse_named
});
raw!(Score { total_points:i32=Reader::int, used_turns:i32=Reader::int, parts:Vec<ScorePart>=|r| list(r,ScorePart::parse) });
fn optional_ordinal(r: &mut Reader<'_>, bound: i32, name: &str) -> Result<Option<i32>> {
    let value = r.int()?;
    if value == -1 {
        Ok(None)
    } else if (0..bound).contains(&value) {
        Ok(Some(value))
    } else {
        Err(Error::Protocol(format!("unknown {name} ordinal {value}")))
    }
}
raw!(PlayerResult { player_id:i32=Reader::int, rank:i32=Reader::int, score:Score=Score::parse, card_names:Vec<CardFrequency>=|r| list(r,CardFrequency::parse), resign_index:i32=Reader::int, resignation_type:Option<i32>=|r| optional_ordinal(r,4,"ResignationTypes") });
raw!(GameResult { table_id:i64=Reader::long, game_id:i64=Reader::long, rating_type:Option<i32>=|r| optional_ordinal(r,4,"RatingTypes"), empty_piles:Vec<i32>=names, players:Vec<PlayerResult>=|r| list(r,PlayerResult::parse), auto_continue:bool=Reader::boolean });

#[cfg(test)]
mod tests;

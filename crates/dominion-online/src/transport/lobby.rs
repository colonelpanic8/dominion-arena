use crate::{
    Error, Result,
    wire::{Reader, Writer},
};
use serde::{Deserialize, Serialize};

fn ordinal(r: &mut Reader<'_>, count: i32, name: &str) -> Result<i32> {
    let value = r.int()?;
    if (0..count).contains(&value) {
        Ok(value)
    } else {
        Err(Error::Unsupported(format!("{name} ordinal {value}")))
    }
}
fn array<T>(r: &mut Reader<'_>, parse: impl Fn(&mut Reader<'_>) -> Result<T>) -> Result<Vec<T>> {
    (0..r.count()?).map(|_| parse(r)).collect()
}
fn card_name(r: &mut Reader<'_>) -> Result<i32> {
    ordinal(r, 881, "card name")
}
fn card_names(r: &mut Reader<'_>) -> Result<Vec<i32>> {
    array(r, card_name)
}
fn expansion(r: &mut Reader<'_>) -> Result<i32> {
    ordinal(r, 20, "expansion")
}
fn bools(r: &mut Reader<'_>) -> Result<Vec<bool>> {
    array(r, |r| r.boolean())
}
fn optional(r: &mut Reader<'_>, count: i32, name: &str) -> Result<Option<i32>> {
    let value = r.int()?;
    if value == -1 {
        Ok(None)
    } else if (0..count).contains(&value) {
        Ok(Some(value))
    } else {
        Err(Error::Unsupported(format!("{name} ordinal {value}")))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NamedId {
    pub id: i32,
    pub name: String,
}
impl NamedId {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            id: r.int()?,
            name: r.string()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.int(self.id);
        w.string(&self.name);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Group {
    Nobody,
    Everybody,
    FriendsOf(NamedId),
    Players(Vec<NamedId>),
}
impl Group {
    fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(match ordinal(r, 4, "group")? {
            0 => Self::Nobody,
            1 => Self::Everybody,
            2 => Self::FriendsOf(NamedId::parse(r)?),
            3 => Self::Players(array(r, NamedId::parse)?),
            _ => unreachable!(),
        })
    }
    fn write(&self, w: &mut Writer) {
        match self {
            Self::Nobody => w.int(0),
            Self::Everybody => w.int(1),
            Self::FriendsOf(p) => {
                w.int(2);
                p.write(w);
            }
            Self::Players(ps) => {
                w.int(3);
                w.int(ps.len() as i32);
                for p in ps {
                    p.write(w);
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TimerSettings {
    pub parameters: Vec<(i32, i64)>,
    pub force_resign: bool,
    pub preset: i32,
}
impl TimerSettings {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            parameters: array(r, |r| Ok((ordinal(r, 29, "timer parameter")?, r.long()?)))?,
            force_resign: r.boolean()?,
            preset: ordinal(r, 8, "timer preset")?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.int(self.parameters.len() as i32);
        for &(id, value) in &self.parameters {
            w.int(id);
            w.long(value);
        }
        w.boolean(self.force_resign);
        w.int(self.preset);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum RequiredSlot {
    Card(i32),
    Expansion(i32),
    LandscapeTypes(i32),
}
impl RequiredSlot {
    fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(match ordinal(r, 3, "required kingdom slot")? {
            0 => Self::Card(card_name(r)?),
            1 => Self::Expansion(expansion(r)?),
            2 => Self::LandscapeTypes(r.int()?),
            _ => unreachable!(),
        })
    }
    fn write(&self, w: &mut Writer) {
        let (kind, value) = match *self {
            Self::Card(x) => (0, x),
            Self::Expansion(x) => (1, x),
            Self::LandscapeTypes(x) => (2, x),
        };
        w.int(kind);
        w.int(value);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpecialKingdom {
    pub colonies: i32,
    pub shelters: i32,
    pub setup: Vec<(i32, Vec<i32>)>,
    pub trait_targets: Vec<(i32, i32)>,
}
impl SpecialKingdom {
    fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let colonies = ordinal(r, 3, "colonies")?;
        let shelters = ordinal(r, 3, "shelters")?;
        let setup = array(r, |r| {
            let id = ordinal(r, 13, "setup rule")?;
            let values = if [1, 2, 4, 5, 10].contains(&id) {
                card_names(r)?
            } else {
                vec![card_name(r)?]
            };
            Ok((id, values))
        })?;
        Ok(Self {
            colonies,
            shelters,
            setup,
            trait_targets: array(r, |r| Ok((card_name(r)?, card_name(r)?)))?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.int(self.colonies);
        w.int(self.shelters);
        w.int(self.setup.len() as i32);
        for (id, vs) in &self.setup {
            w.int(*id);
            if [1, 2, 4, 5, 10].contains(id) {
                w.ints(vs);
            } else {
                w.int(vs[0]);
            }
        }
        w.int(self.trait_targets.len() as i32);
        for &(a, b) in &self.trait_targets {
            w.int(a);
            w.int(b);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum RuleValue {
    Boolean(bool),
    Int(i32),
    Double(f64),
    Ints(Vec<i32>),
    Group(Group),
    Required(Vec<RequiredSlot>),
    Seed {
        enabled: bool,
        seed: i64,
    },
    Special(SpecialKingdom),
    PlayerOrder(Vec<(NamedId, i32)>),
    Timer(TimerSettings),
    Replay {
        game_id: i64,
        decision: i32,
        players: Vec<NamedId>,
    },
    Text(String),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TableRule {
    pub id: i32,
    pub value: RuleValue,
}
impl TableRule {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let id = ordinal(r, 25, "table rule")?;
        let value = match id {
            0 | 13 | 19 | 20 | 23 => RuleValue::Boolean(r.boolean()?),
            1 => RuleValue::Ints(card_names(r)?),
            3 => RuleValue::Ints(array(r, expansion)?),
            12 => RuleValue::Ints(r.ints()?),
            21 => RuleValue::Ints(array(r, |r| ordinal(r, 4, "automatch question"))?),
            2 => RuleValue::Required(array(r, RequiredSlot::parse)?),
            4 | 5 | 17 => RuleValue::Group(Group::parse(r)?),
            6 | 7 => RuleValue::Double(r.double()?),
            8 => RuleValue::Seed {
                enabled: r.boolean()?,
                seed: r.long()?,
            },
            9 | 10 | 22 => RuleValue::Int(r.int()?),
            11 => RuleValue::Special(SpecialKingdom::parse(r)?),
            14 => RuleValue::PlayerOrder(array(r, |r| Ok((NamedId::parse(r)?, r.int()?)))?),
            15 => RuleValue::Int(ordinal(r, 3, "undo setting")?),
            16 => RuleValue::Timer(TimerSettings::parse(r)?),
            18 => RuleValue::Replay {
                game_id: r.long()?,
                decision: r.int()?,
                players: array(r, NamedId::parse)?,
            },
            24 => RuleValue::Text(r.string()?),
            _ => unreachable!(),
        };
        Ok(Self { id, value })
    }
    pub fn write(&self, w: &mut Writer) -> Result<()> {
        let correct = matches!(
            (&self.value, self.id),
            (RuleValue::Boolean(_), 0 | 13 | 19 | 20 | 23)
                | (RuleValue::Ints(_), 1 | 3 | 12 | 21)
                | (RuleValue::Required(_), 2)
                | (RuleValue::Group(_), 5)
                | (RuleValue::Double(_), 6 | 7)
                | (RuleValue::Seed { .. }, 8)
                | (RuleValue::Int(_), 9 | 10 | 22)
                | (RuleValue::Special(_), 11)
                | (RuleValue::PlayerOrder(_), 14)
                | (RuleValue::Timer(_), 16)
                | (RuleValue::Replay { .. }, 18)
                | (RuleValue::Text(_), 24)
        );
        if matches!(&self.value, RuleValue::Group(Group::Players(_))) {
            return Err(Error::Unsupported(
                "outgoing group player-list has no evidenced table-rule serializer".into(),
            ));
        }
        if !correct {
            return Err(Error::Unsupported(format!(
                "outgoing table rule {} / value",
                self.id
            )));
        }
        if let RuleValue::Special(s) = &self.value {
            for (id, values) in &s.setup {
                if !(0..13).contains(id) || (![1, 2, 4, 5, 10].contains(id) && values.len() != 1) {
                    return Err(Error::Protocol("invalid setup rule".into()));
                }
            }
        }
        w.int(self.id);
        match &self.value {
            RuleValue::Boolean(v) => w.boolean(*v),
            RuleValue::Int(v) => w.int(*v),
            RuleValue::Double(v) => w.double(*v),
            RuleValue::Ints(v) => w.ints(v),
            RuleValue::Group(v) => v.write(w),
            RuleValue::Required(v) => {
                w.int(v.len() as i32);
                for v in v {
                    v.write(w);
                }
            }
            RuleValue::Seed { enabled, seed } => {
                w.boolean(*enabled);
                w.long(*seed);
            }
            RuleValue::Special(v) => v.write(w),
            RuleValue::PlayerOrder(v) => {
                w.int(v.len() as i32);
                for (p, n) in v {
                    p.write(w);
                    w.int(*n);
                }
            }
            RuleValue::Timer(v) => v.write(w),
            RuleValue::Replay {
                game_id,
                decision,
                players,
            } => {
                w.long(*game_id);
                w.int(*decision);
                w.int(players.len() as i32);
                for p in players {
                    p.write(w);
                }
            }
            RuleValue::Text(v) => w.string(v),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MatchScore {
    pub host_wins: f64,
    pub opponent_wins: f64,
    pub host_name: String,
    pub opponent_name: String,
    pub winner: Option<i32>,
    pub completed: bool,
    pub host_id: i32,
    pub opponent_id: Option<i32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TableDetails {
    pub table_id: i64,
    pub host_id: i32,
    pub players: Vec<NamedId>,
    pub ready: Vec<bool>,
    pub bots: Vec<NamedId>,
    pub spectators: Vec<NamedId>,
    pub reconnecting: Vec<NamedId>,
    pub rules: Vec<(TableRule, bool)>,
    pub division: String,
    pub match_games: Vec<i64>,
    pub match_score: Option<MatchScore>,
    pub tournament: Option<i32>,
}
impl TableDetails {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let table_id = r.long()?;
        let host_id = r.int()?;
        let players = array(r, NamedId::parse)?;
        let ready = bools(r)?;
        if ready.len() != players.len() {
            return Err(Error::Protocol(
                "table player/ready arrays differ in length".into(),
            ));
        }
        let bots = array(r, NamedId::parse)?;
        let spectators = array(r, NamedId::parse)?;
        let reconnecting = array(r, NamedId::parse)?;
        let rules = array(r, |r| Ok((TableRule::parse(r)?, r.boolean()?)))?;
        let division = r.string()?;
        let (match_games, match_score) = if r.boolean()? {
            let games = array(r, |r| r.long())?;
            let score = MatchScore {
                host_wins: r.double()?,
                opponent_wins: r.double()?,
                host_name: r.string()?,
                opponent_name: r.string()?,
                winner: if r.boolean()? { Some(r.int()?) } else { None },
                completed: r.boolean()?,
                host_id: r.int()?,
                opponent_id: if r.boolean()? { Some(r.int()?) } else { None },
            };
            (games, Some(score))
        } else {
            (vec![], None)
        };
        Ok(Self {
            table_id,
            host_id,
            players,
            ready,
            bots,
            spectators,
            reconnecting,
            rules,
            division,
            match_games,
            match_score,
            tournament: optional(r, 2, "tournament")?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TableSummary {
    pub table_id: i64,
    pub host: NamedId,
    pub players: i32,
    pub bots: i32,
    pub spectators: i32,
    pub min_players: i32,
    pub max_players: i32,
    pub observable: bool,
    pub joinable: bool,
    pub status: i32,
    pub start_time: Option<i64>,
}
impl TableSummary {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let table_id = r.long()?;
        let host = NamedId::parse(r)?;
        let players = r.int()?;
        let bots = r.int()?;
        let spectators = r.int()?;
        let min_players = r.int()?;
        let max_players = r.int()?;
        let observable = r.boolean()?;
        let joinable = r.boolean()?;
        let status = ordinal(r, 5, "table status")?;
        Ok(Self {
            table_id,
            host,
            players,
            bots,
            spectators,
            min_players,
            max_players,
            observable,
            joinable,
            status,
            start_time: if status == 2 { Some(r.long()?) } else { None },
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Relationships {
    pub following: Vec<NamedId>,
    pub friends: Vec<NamedId>,
    pub followers: Vec<NamedId>,
    pub blacklisted: Vec<NamedId>,
}
impl Relationships {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            following: array(r, NamedId::parse)?,
            friends: array(r, NamedId::parse)?,
            followers: array(r, NamedId::parse)?,
            blacklisted: array(r, NamedId::parse)?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PreferenceValue {
    Text(String),
    Int(i32),
    Boolean(bool),
    Deleted,
    Cards {
        list_id: i32,
        cards: Vec<i32>,
    },
    Ints(Vec<i32>),
    Ownership {
        expansions: Vec<(i64, Vec<i32>)>,
        levels: Vec<(i64, Vec<i32>)>,
    },
    Matching {
        bot: i32,
        human: bool,
        max_rating: i32,
        min_rating: i32,
        preferences: Vec<(i32, i32)>,
        timers: Vec<(i32, i32)>,
    },
    Autoplay(Vec<(i32, i32)>),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Preference {
    pub id: i32,
    pub value: PreferenceValue,
}
impl Preference {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        // The client dispatches by the transmitted type even for migrated preferences.
        let id = ordinal(r, 52, "preference")?;
        let kind = ordinal(r, 9, "preference type")?;
        let value = match kind {
            0 => PreferenceValue::Text(r.string()?),
            1 => PreferenceValue::Int(r.int()?),
            2 => PreferenceValue::Ownership {
                expansions: array(r, |r| Ok((r.long()?, array(r, expansion)?)))?,
                levels: array(r, |r| Ok((r.long()?, r.ints()?)))?,
            },
            3 => PreferenceValue::Cards {
                list_id: r.int()?,
                cards: card_names(r)?,
            },
            4 => PreferenceValue::Boolean(r.boolean()?),
            5 => PreferenceValue::Matching {
                bot: r.int()?,
                human: r.boolean()?,
                max_rating: r.int()?,
                min_rating: r.int()?,
                preferences: array(r, |r| {
                    Ok((
                        ordinal(r, 4, "automatch preference")?,
                        ordinal(r, 3, "matching preference")?,
                    ))
                })?,
                timers: array(r, |r| Ok((ordinal(r, 8, "timer preference")?, r.int()?)))?,
            },
            6 => PreferenceValue::Autoplay(array(r, |r| Ok((card_name(r)?, r.int()?)))?),
            7 => PreferenceValue::Ints(array(r, expansion)?),
            8 => PreferenceValue::Deleted,
            _ => unreachable!(),
        };
        Ok(Self { id, value })
    }
}

#[derive(Clone)]
pub struct LoginSuccess {
    pub player_id: i32,
    pub username: String,
    pub reconnecting: bool,
    pub previous_reconnects: i32,
    pub preferences: Vec<Preference>,
    session_id: String,
    pub unread_mail: i32,
    pub relationships: Relationships,
    pub card_levels: Vec<(i32, i32)>,
    pub timer_presets: Vec<(i32, TimerSettings)>,
    pub feature_words: Vec<i32>,
}
impl std::fmt::Debug for LoginSuccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginSuccess")
            .field("player_id", &self.player_id)
            .field("username", &self.username)
            .field("reconnecting", &self.reconnecting)
            .field("session_id", &"[redacted]")
            .finish_non_exhaustive()
    }
}
impl LoginSuccess {
    /// Local bootstrap values for every non-deleted preference consumed by 2.3.7.
    pub fn new(player_id: i32, username: String, session_id: String) -> Self {
        use PreferenceValue::*;
        let mut preferences = Vec::new();
        for id in 0..52 {
            let value = match id {
                0 => Ownership {
                    expansions: vec![(4_102_444_800_000, (0..20).collect())],
                    levels: vec![(4_102_444_800_000, (1..=10).collect())],
                },
                1 => Text(String::new()),
                2 | 14 | 38 | 42 | 48 => Int(0),
                30 => Int(3),
                29 => Int(10),
                34 => Int(1),
                36 | 37 => Int(50),
                8 | 9 | 23 => Cards {
                    list_id: id,
                    cards: vec![],
                },
                17 => Matching {
                    bot: -1,
                    human: false,
                    max_rating: 100,
                    min_rating: 0,
                    preferences: vec![],
                    timers: vec![],
                },
                19 => Autoplay(vec![]),
                3 | 4 | 7 | 12 | 27 | 28 | 31 | 32 => continue,
                18 | 25 => Boolean(true),
                _ => Boolean(false),
            };
            preferences.push(Preference { id, value });
        }
        Self {
            player_id,
            username,
            session_id,
            reconnecting: false,
            previous_reconnects: 0,
            preferences,
            unread_mail: 0,
            relationships: Relationships {
                following: vec![],
                friends: vec![],
                followers: vec![],
                blacklisted: vec![],
            },
            card_levels: (0..881)
                .filter(|id| {
                    ![
                        370, 371, 372, 373, 374, 591, 592, 593, 594, 595, 596, 597, 598, 599, 600,
                        601, 602, 603, 604, 605, 606, 607, 608, 609, 610, 638, 639, 640, 641, 642,
                        643, 644, 645, 646, 647, 648, 649, 650, 732,
                    ]
                    .contains(id)
                })
                .map(|id| (id, 1))
                .collect(),
            timer_presets: (0..8)
                .map(|preset| {
                    (
                        preset,
                        TimerSettings {
                            parameters: (0..29).map(|id| (id, 0)).collect(),
                            force_resign: false,
                            preset,
                        },
                    )
                })
                .collect(),
            feature_words: vec![],
        }
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            player_id: r.int()?,
            username: r.string()?,
            reconnecting: r.boolean()?,
            previous_reconnects: r.int()?,
            preferences: array(r, Preference::parse)?,
            session_id: r.string()?,
            unread_mail: r.int()?,
            relationships: Relationships::parse(r)?,
            card_levels: array(r, |r| Ok((card_name(r)?, r.int()?)))?,
            timer_presets: array(r, |r| {
                Ok((ordinal(r, 8, "timer preset")?, TimerSettings::parse(r)?))
            })?,
            feature_words: r.ints()?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FriendActivity {
    pub player_id: i32,
    pub name: String,
    pub status: i32,
    pub table_id: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutomatchStatus {
    pub friends: Vec<FriendActivity>,
    pub searching: bool,
    pub searching_with: Vec<(i32, i32)>,
    pub searching_without: Vec<(i32, i32)>,
    pub time_in_queue: i64,
    pub levels: [f64; 4],
    pub feedback: Vec<NamedId>,
    pub preview_queue: bool,
    pub subscription_days: f64,
    pub other_server: bool,
}
impl AutomatchStatus {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            friends: array(r, |r| {
                Ok(FriendActivity {
                    player_id: r.int()?,
                    name: r.string()?,
                    status: ordinal(r, 6, "activity")?,
                    table_id: r.long()?,
                })
            })?,
            searching: r.boolean()?,
            searching_with: array(r, |r| Ok((ordinal(r, 4, "automatch question")?, r.int()?)))?,
            searching_without: array(r, |r| Ok((ordinal(r, 4, "automatch question")?, r.int()?)))?,
            time_in_queue: r.long()?,
            levels: [r.double()?, r.double()?, r.double()?, r.double()?],
            feedback: array(r, NamedId::parse)?,
            preview_queue: r.boolean()?,
            subscription_days: r.double()?,
            other_server: r.boolean()?,
        })
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn transmitted_preference_type_controls_migrated_values() {
        let mut w = Writer::new();
        w.int(42);
        w.int(4);
        w.boolean(true);
        w.int(123);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        assert_eq!(
            Preference::parse(&mut r).unwrap().value,
            PreferenceValue::Boolean(true)
        );
        assert_eq!(r.int().unwrap(), 123);
        r.finish().unwrap();
    }
}

//! Typed question trees for the hash-pinned 2.3.7 client. Choice answers are
//! positions; only Name answers contain global card-name ordinals.
use crate::{Error, Result, wire::Reader};
use dominion_protocol::{AgentDecision, AgentOption, Answer, DecisionGroup};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_DEPTH: usize = 48;
const MAX_OPTIONS: usize = 65_536;
const TYPE_NAMES: &[&str] = &[
    "PLAY",
    "GAIN",
    "DISCARD",
    "TRASH",
    "TRASH_UNIQUE",
    "SELECT_ZONE",
    "SELECT_TURN",
    "RESOLVE_ABILITY",
    "ORDER_CARDS",
    "BUY",
    "CHOOSE_MODE",
    "HOW_MUCH_REPAY",
    "HOW_MANY_SPEND",
    "TOPDECK",
    "BOTTOMDECK",
    "SET_ASIDE",
    "REVEAL",
    "PASS",
    "COMPLEX_OR",
    "COMPLEX_AND",
    "WISH",
    "DELAYED",
    "PUT_IN_HAND",
    "KEEP_IN_DISCARD",
    "BID",
    "OVERPAY",
    "SHUFFLE_INTO_DECK",
    "INSERT_INTO_DECK",
    "LOCATION",
    "DONATE",
    "TAKE_HOW_MANY",
    "RETURN_TO_SUPPLY",
    "GAIN_PRIZE",
    "PRIORITIZE_CARDS",
    "BUTTON",
    "CLEANUP",
    "IMPERSONATE",
    "RECEIVE",
    "START_GAME",
    "PLAY_USING_VILLAGER",
    "EXILE",
    "USE_VILLAGERS",
    "EXILE_WITH_SAME_NAME",
    "WAY",
    "HOW_MANY_FAVORS",
    "ROTATE",
    "KEEP",
    "OVERPAY_COINS",
    "OVERPAY_POTIONS",
    "REJECT_ABILITY",
    "REVEAL_SELECT",
];
fn protocol(message: impl Into<String>) -> Error {
    Error::Protocol(message.into())
}
fn rejected(message: impl Into<String>) -> Error {
    Error::Rejected(message.into())
}
fn ordinal(r: &mut Reader<'_>, count: i32, label: &str) -> Result<i32> {
    let value = r.int()?;
    if !(0..count).contains(&value) {
        return Err(protocol(format!("unknown {label} ordinal {value}")));
    }
    Ok(value)
}
fn depth(level: usize) -> Result<()> {
    if level > MAX_DEPTH {
        Err(protocol("nested question/story limit exceeded"))
    } else {
        Ok(())
    }
}
fn array<T>(
    r: &mut Reader<'_>,
    mut parse: impl FnMut(&mut Reader<'_>) -> Result<T>,
) -> Result<Vec<T>> {
    let count = r.count()?;
    (0..count).map(|_| parse(r)).collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardFrequency {
    pub name: i32,
    pub frequency: i32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ability {
    pub description_type: i32,
    pub name: i32,
    pub association: i32,
    pub arguments: Vec<LogArgument>,
    pub optional: bool,
    pub copy_index: i32,
    pub copy_total: i32,
}
impl Ability {
    fn parse(r: &mut Reader<'_>, question: bool, level: usize) -> Result<Self> {
        depth(level)?;
        let description_type = r.int()?;
        if !(0..=2).contains(&description_type) {
            return Err(protocol(format!(
                "unknown ability description type {description_type}"
            )));
        }
        let name = r.int()?;
        let association = r.int()?;
        let arguments = if description_type == 2 {
            array(r, |r| LogArgument::parse_at(r, level + 1))?
        } else {
            vec![]
        };
        let (optional, copy_index, copy_total) = if question {
            (r.boolean()?, r.int()?, r.int()?)
        } else {
            (false, 0, 1)
        };
        Ok(Self {
            description_type,
            name,
            association,
            arguments,
            optional,
            copy_index,
            copy_total,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaGameInfo {
    pub game_id: i64,
    pub rated: bool,
    pub card_pool_level: i32,
    pub levels: Vec<(i32, f64)>,
    pub likelihoods: Vec<(i32, Vec<i32>)>,
    pub timer_preset: i32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LogArgument {
    CardNames(Vec<CardFrequency>),
    Player(i32),
    Zone(i32),
    OwnZone {
        player: i32,
        zone: i32,
    },
    Number(i32),
    Counter(i32),
    Turn {
        owner: i32,
        number: i32,
        kind: i32,
        controller: i32,
    },
    Amount {
        coins: i32,
        potions: i32,
        debt: i32,
    },
    Ability(Ability),
    Event {
        kind: i32,
        ability: Ability,
        arguments: Vec<LogArgument>,
    },
    Card(i32),
    SingularCard(i32),
    Token(i32),
    DirectionalZone {
        direction: i32,
        zone: i32,
    },
    Players(Vec<i32>),
    MetaGame(MetaGameInfo),
}
impl LogArgument {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        Self::parse_at(r, 0)
    }
    fn parse_at(r: &mut Reader<'_>, level: usize) -> Result<Self> {
        depth(level)?;
        Ok(match r.int()? {
            0 => Self::CardNames(array(r, |r| {
                Ok(CardFrequency {
                    name: r.int()?,
                    frequency: r.int()?,
                })
            })?),
            1 => Self::Player(r.int()?),
            2 => Self::Zone(r.int()?),
            3 => Self::OwnZone {
                player: r.int()?,
                zone: r.int()?,
            },
            4 => Self::Number(r.int()?),
            5 => Self::Counter(r.int()?),
            6 => Self::Turn {
                owner: r.int()?,
                number: r.int()?,
                kind: ordinal(r, 10, "turn")?,
                controller: r.int()?,
            },
            7 => Self::Amount {
                coins: r.int()?,
                potions: r.int()?,
                debt: r.int()?,
            },
            8 => Self::Ability(Ability::parse(r, false, level + 1)?),
            9 => Self::Event {
                kind: ordinal(r, 3, "ability event")?,
                ability: Ability::parse(r, false, level + 1)?,
                arguments: array(r, |r| Self::parse_at(r, level + 1))?,
            },
            10 => Self::Card(r.int()?),
            11 => Self::SingularCard(r.int()?),
            12 => Self::Token(r.int()?),
            13 => {
                let direction = r.int()?;
                if !(0..=1).contains(&direction) {
                    return Err(protocol("unknown direction"));
                }
                Self::DirectionalZone {
                    direction,
                    zone: r.int()?,
                }
            }
            14 => Self::Players(r.ints()?),
            15 => Self::MetaGame(MetaGameInfo {
                game_id: r.long()?,
                rated: r.boolean()?,
                card_pool_level: r.int()?,
                levels: array(r, |r| Ok((r.int()?, r.double()?)))?,
                likelihoods: array(r, |r| Ok((r.int()?, r.ints()?)))?,
                timer_preset: r.int()?,
            }),
            tag => return Err(protocol(format!("unknown log argument tag {tag}"))),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Story {
    pub question_id: i32,
    pub arguments: Vec<LogArgument>,
    pub context: Option<LogArgument>,
}
impl Story {
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let question_id = r.int()?;
        if question_id < 0 {
            return Err(protocol("negative story question id"));
        }
        Ok(Self {
            question_id,
            arguments: array(r, LogArgument::parse)?,
            context: if r.boolean()? {
                Some(LogArgument::parse(r)?)
            } else {
                None
            },
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestionDescription {
    pub question_type: i32,
    pub association: i32,
    pub story: Story,
    pub short_story: Story,
}
impl QuestionDescription {
    fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let question_type = r.int()?;
        if question_type < 0 || question_type as usize >= TYPE_NAMES.len() {
            return Err(protocol(format!("unknown question type {question_type}")));
        }
        Ok(Self {
            question_type,
            association: r.int()?,
            story: Story::parse(r)?,
            short_story: Story::parse(r)?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtraTurn {
    pub owner: i32,
    pub number: i32,
    pub kind: i32,
    pub controller: i32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CleanupAction {
    pub names: Vec<i32>,
    pub is_optional: bool,
    pub allows_reordering: bool,
    pub is_enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum QuestionElements {
    Cards(Vec<i32>),
    Abilities(Vec<Ability>),
    ExtraTurns(Vec<ExtraTurn>),
    Zones(Vec<i32>),
    Modes(Vec<i32>),
    Names(Vec<i32>),
    Buttons(Vec<i32>),
    Cleanup(Vec<CleanupAction>),
    BuyableCards(Vec<(i32, bool)>),
    ExplainedCards(Vec<(i32, i32, i32)>),
}
impl QuestionElements {
    fn parse(r: &mut Reader<'_>, level: usize) -> Result<Self> {
        let count = r.count()?;
        if count == 0 {
            return Ok(Self::Cards(vec![]));
        }
        let tag = r.int()?;
        macro_rules! many {
            ($parser:expr) => {
                (0..count).map($parser).collect::<Result<Vec<_>>>()?
            };
        }
        Ok(match tag {
            0 => Self::Cards(many!(|_| r.int())),
            1 => Self::Abilities(many!(|_| Ability::parse(r, true, level + 1))),
            2 => Self::ExtraTurns(many!(|_| Ok(ExtraTurn {
                owner: r.int()?,
                number: r.int()?,
                kind: ordinal(r, 10, "extra turn")?,
                controller: r.int()?
            }))),
            3 => Self::Zones(many!(|_| r.int())),
            4 => Self::Modes(many!(|_| r.int())),
            5 => Self::Names(many!(|_| r.int())),
            6 => Self::Buttons(many!(|_| r.int())),
            7 => Self::Cleanup(many!(|_| Ok(CleanupAction {
                names: r.ints()?,
                is_optional: r.boolean()?,
                allows_reordering: r.boolean()?,
                is_enabled: r.boolean()?
            }))),
            8 => Self::BuyableCards(many!(|_| Ok((r.int()?, r.boolean()?)))),
            9 => Self::ExplainedCards(many!(|_| {
                let id = r.int()?;
                let value = r.int()?;
                let kind = r.int()?;
                if !(-1..=0).contains(&kind) {
                    return Err(protocol("unknown card explanation type"));
                }
                Ok((id, value, kind))
            })),
            _ => return Err(protocol(format!("unknown question element tag {tag}"))),
        })
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Cards(v)
            | Self::Zones(v)
            | Self::Modes(v)
            | Self::Names(v)
            | Self::Buttons(v) => v.len(),
            Self::Abilities(v) => v.len(),
            Self::ExtraTurns(v) => v.len(),
            Self::Cleanup(v) => v.len(),
            Self::BuyableCards(v) => v.len(),
            Self::ExplainedCards(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum QuestionKind {
    Choice {
        min: i32,
        max: i32,
        elements: QuestionElements,
        decline: i32,
        defaults: Vec<i32>,
        affected: Vec<i32>,
    },
    Number {
        min: i32,
        max: i32,
        default: i32,
        decline: i32,
    },
    Complex(Vec<Question>),
    Delayed {
        decline: i32,
    },
    Name,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub index: i32,
    pub description: QuestionDescription,
    pub kind: QuestionKind,
}
impl Question {
    /// Reads a root index and class tag, followed by its complete question tree.
    pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let index = r.int()?;
        if index < 0 {
            return Err(protocol("negative question index"));
        }
        let tag = r.int()?;
        Self::parse_at(r, index, tag, 0)
    }
    fn parse_at(r: &mut Reader<'_>, index: i32, tag: i32, level: usize) -> Result<Self> {
        depth(level)?;
        if !(0..=4).contains(&tag) {
            return Err(protocol(format!("unknown question class {tag}")));
        }
        let description = QuestionDescription::parse(r)?;
        let kind = match tag {
            0 => {
                let min = r.int()?;
                let max = r.int()?;
                if min < 0 || max < min {
                    return Err(protocol("invalid choice bounds"));
                }
                QuestionKind::Choice {
                    min,
                    max,
                    elements: QuestionElements::parse(r, level)?,
                    decline: r.int()?,
                    defaults: r.ints()?,
                    affected: r.ints()?,
                }
            }
            1 => {
                let min = r.int()?;
                let max = r.int()?;
                let default = r.int()?;
                let decline = r.int()?;
                if min > max || !(min..=max).contains(&default) {
                    return Err(protocol("invalid number bounds/default"));
                }
                QuestionKind::Number {
                    min,
                    max,
                    default,
                    decline,
                }
            }
            2 => {
                if ![18, 19].contains(&description.question_type) {
                    return Err(protocol("complex question is neither AND nor OR"));
                }
                QuestionKind::Complex(array(r, |r| {
                    let tag = r.int()?;
                    Self::parse_at(r, index, tag, level + 1)
                })?)
            }
            3 => QuestionKind::Delayed { decline: r.int()? },
            4 => QuestionKind::Name,
            _ => unreachable!(),
        };
        Ok(Self {
            index,
            description,
            kind,
        })
    }
    pub fn type_name(&self) -> &'static str {
        TYPE_NAMES
            .get(self.description.question_type as usize)
            .copied()
            .unwrap_or("UNKNOWN")
    }
    fn ordered(&self) -> bool {
        matches!(self.description.question_type, 8 | 33)
            || matches!(&self.kind, QuestionKind::Choice {elements: QuestionElements::Cleanup(items), ..} if items.iter().any(|item| item.allows_reordering))
    }
}

/// Lookups must use the observing player's legal state, never allocated card
/// identities from hidden zones. Enum labels belong to the pinned catalog.
pub trait QuestionContext {
    fn card_name(&self, instance: i32) -> Option<String>;
    fn catalog_name(&self, ordinal: i32) -> Option<String>;
    fn catalog_ordinal(&self, name: &str) -> Option<i32>;
    fn zone_name(&self, zone: i32) -> Option<String>;
    fn enum_label(&self, _catalog: &str, _ordinal: i32) -> Option<String> {
        None
    }
    fn story_text(&self, _story: &Story) -> Option<String> {
        None
    }
    fn name_choices(&self) -> Vec<(i32, String)> {
        vec![]
    }
    fn ability_text(&self, _ability: &Ability) -> Option<String> {
        None
    }
    fn card_cost(&self, _instance: i32) -> Option<dominion_protocol::Cost> {
        None
    }
    fn card_zone(&self, _instance: i32) -> Option<String> {
        None
    }
}

#[derive(Clone, Debug)]
enum Target {
    Select(i32),
    Decline,
    Number(i32),
    Name(i32),
    Accept,
    KeepOrder,
    Empty,
}
struct Indexed<'a> {
    decision: AgentDecision,
    leaves: Vec<(&'a Question, Vec<usize>)>,
    targets: Vec<(usize, Target)>,
}
fn humanize(value: &str) -> String {
    let text = value
        .strip_prefix("GAME_")
        .unwrap_or(value)
        .replace('_', " ")
        .to_lowercase();
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}
fn required(value: Option<String>, what: impl Into<String>) -> Result<String> {
    value
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| Error::Unsupported(what.into()))
}
fn prompt(q: &Question, c: &impl QuestionContext) -> String {
    c.story_text(&q.description.story)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| {
            humanize(
                &c.enum_label("QuestionIds", q.description.story.question_id)
                    .unwrap_or_else(|| q.type_name().into()),
            )
        })
}
fn option(action: &str, label: String, card: Option<String>, value: Option<i32>) -> AgentOption {
    AgentOption {
        index: 0,
        group: 0,
        action: action.into(),
        label,
        card,
        zone: None,
        cost: None,
        value,
        player: None,
    }
}
fn card_option(c: &impl QuestionContext, q: &Question, instance: i32) -> Result<AgentOption> {
    let name = required(
        c.card_name(instance),
        format!("question card {instance} has no visible identity"),
    )?;
    let action = match q.description.question_type {
        0 | 39 => "play",
        9 => "buy",
        _ => "select",
    };
    let label = match action {
        "play" => format!("Play {name}"),
        "buy" => format!("Buy {name}"),
        _ => name.clone(),
    };
    let mut result = option(action, label, Some(name), None);
    result.cost = c.card_cost(instance);
    result.zone = c.card_zone(instance);
    Ok(result)
}
fn element_options(
    q: &Question,
    c: &impl QuestionContext,
    e: &QuestionElements,
) -> Result<Vec<AgentOption>> {
    Ok(match e {
        QuestionElements::Cards(v) => v
            .iter()
            .map(|&id| card_option(c, q, id))
            .collect::<Result<_>>()?,
        QuestionElements::BuyableCards(v) => v
            .iter()
            .map(|&(id, _)| card_option(c, q, id))
            .collect::<Result<_>>()?,
        QuestionElements::ExplainedCards(v) => v
            .iter()
            .map(|&(id, value, kind)| {
                let mut option = card_option(c, q, id)?;
                if kind == 0 {
                    option.label.push_str(&format!(" ({value} VP)"));
                } else if kind != -1 {
                    return Err(protocol("unknown card explanation type"));
                }
                Ok(option)
            })
            .collect::<Result<_>>()?,
        QuestionElements::Names(v) => v
            .iter()
            .map(|&id| {
                let name = required(
                    c.catalog_name(id),
                    format!("unknown card-name ordinal {id}"),
                )?;
                Ok(option("select", name.clone(), Some(name), None))
            })
            .collect::<Result<_>>()?,
        QuestionElements::Modes(v) => v
            .iter()
            .enumerate()
            .map(|(position, &id)| {
                Ok(option(
                    "mode",
                    required(
                        c.enum_label("CardModes", id),
                        format!("unlabelled card mode {id}"),
                    )?,
                    None,
                    Some(position as i32),
                ))
            })
            .collect::<Result<_>>()?,
        QuestionElements::Buttons(v) => v
            .iter()
            .map(|&id| {
                let action = match id {
                    0 => "select",
                    1 => "spend-villager",
                    2 | 3 => "select",
                    4 => "spend-coffers",
                    5 => "pay-debt",
                    6 => "select",
                    7 => "end-phase",
                    _ => return Err(protocol(format!("unknown game button {id}"))),
                };
                Ok(option(
                    action,
                    required(
                        c.enum_label("GameButtons", id),
                        format!("unlabelled game button {id}"),
                    )?,
                    None,
                    None,
                ))
            })
            .collect::<Result<_>>()?,
        QuestionElements::Zones(v) => v
            .iter()
            .map(|&id| {
                let name = required(c.zone_name(id), format!("unknown question zone {id}"))?;
                let mut o = option("pile", name.clone(), None, None);
                o.zone = Some(name);
                Ok(o)
            })
            .collect::<Result<_>>()?,
        QuestionElements::Abilities(v) => v
            .iter()
            .map(|a| {
                let catalog = if a.description_type == 0 {
                    "MovementTypes"
                } else {
                    "AbilityNames"
                };
                let label = if let Some(text) = c.ability_text(a) {
                    text
                } else {
                    required(
                        c.enum_label(catalog, a.name),
                        format!("unlabelled ability {}", a.name),
                    )?
                };
                let card = c.card_name(a.association);
                let label = match &card {
                    Some(name) if !label.contains(name) => format!("{name}: {label}"),
                    _ => label,
                };
                Ok(option("trigger", label, card, None))
            })
            .collect::<Result<_>>()?,
        QuestionElements::ExtraTurns(v) => v
            .iter()
            .map(|t| {
                Ok(option(
                    "select",
                    required(
                        c.enum_label("ExtraTurnTypes", t.kind),
                        format!("unlabelled extra turn {}", t.kind),
                    )?,
                    None,
                    None,
                ))
            })
            .collect::<Result<_>>()?,
        QuestionElements::Cleanup(items) => items
            .iter()
            .map(|item| {
                let names = item
                    .names
                    .iter()
                    .map(|&n| required(c.catalog_name(n), format!("unknown cleanup card {n}")))
                    .collect::<Result<Vec<_>>>()?;
                let first = names
                    .first()
                    .ok_or_else(|| protocol("cleanup item has no cards"))?;
                let traveller = matches!(
                    first.as_str(),
                    "Page"
                        | "Treasure Hunter"
                        | "Warrior"
                        | "Hero"
                        | "Peasant"
                        | "Soldier"
                        | "Fugitive"
                        | "Disciple"
                );
                let label = if traveller {
                    if names.len() != 2 {
                        return Err(Error::Unsupported(
                            "Traveller exchange without a destination card".into(),
                        ));
                    }
                    format!("Exchange {} for {}", names[0], names[1])
                } else {
                    format!("Topdeck {}", names.join(", "))
                };
                Ok(option("select", label, Some(first.clone()), None))
            })
            .collect::<Result<_>>()?,
    })
}
fn bounds(q: &Question) -> (u32, u32) {
    match &q.kind {
        QuestionKind::Choice {
            min, max, elements, ..
        } => {
            let count = elements.len() as u32;
            ((*min as u32).min(count), (*max as u32).min(count))
        }
        _ => (1, 1),
    }
}
fn purpose(q: &Question) -> &'static str {
    match q.description.question_type {
        0 | 39 => "play",
        1 | 32 => "gain",
        2 => "discard",
        3 | 4 => "trash",
        7 => "trigger",
        8 | 33 => "move",
        9 => "turn",
        10 => "mode",
        13 => "top-deck",
        14 | 22 | 23 | 26 | 27 => "move",
        15 => "set-aside",
        16 | 50 => "reveal",
        17 => "pass",
        18 | 19 => "either",
        20 => "name",
        25 | 47 | 48 => "overpay",
        31 => "return",
        40 | 42 => "exile",
        _ => "other",
    }
}
impl Question {
    /// `id` must identify this opening of the decision, including reconnect/undo
    /// epoch. The caller retires it after sending; server indices may be reused.
    pub fn project(
        &self,
        context: &impl QuestionContext,
        id: &str,
        player: Option<u8>,
    ) -> Result<AgentDecision> {
        Ok(self.indexed(context, id, player)?.decision)
    }
    fn indexed<'a>(
        &'a self,
        c: &impl QuestionContext,
        id: &str,
        player: Option<u8>,
    ) -> Result<Indexed<'a>> {
        let mut out = Indexed {
            decision: AgentDecision {
                id: id.into(),
                player,
                prompt: prompt(self, c),
                kind: "choose".into(),
                purpose: purpose(self).into(),
                source: c.card_name(self.description.association),
                phase: None,
                ordered: false,
                name_any: matches!(self.kind, QuestionKind::Name),
                min: None,
                max: None,
                groups: vec![],
                options: vec![],
            },
            leaves: vec![],
            targets: vec![],
        };
        self.index_leaves(c, &mut out, &mut vec![])?;
        if matches!(self.kind, QuestionKind::Complex(_)) {
            let combinations = self.group_combinations(&out.leaves, &mut vec![])?;
            let combinations = combinations
                .iter()
                .map(|groups| {
                    format!(
                        "[{}]",
                        groups
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" or ");
            out.decision.prompt.push_str(&format!(
                " Combine groups within one alternative: {combinations}."
            ));
        }
        if out.decision.groups.len() == 1 {
            let g = &out.decision.groups[0];
            out.decision.kind = g.kind.clone();
            out.decision.min = Some(g.min);
            out.decision.max = Some(g.max);
        }
        if out.decision.options.len() > MAX_OPTIONS {
            return Err(Error::Unsupported(
                "question has too many projected options".into(),
            ));
        }
        Ok(out)
    }
    fn group_combinations(
        &self,
        leaves: &[(&Question, Vec<usize>)],
        path: &mut Vec<usize>,
    ) -> Result<Vec<Vec<usize>>> {
        let QuestionKind::Complex(children) = &self.kind else {
            return Ok(vec![vec![
                leaves
                    .iter()
                    .position(|(_, p)| p == path)
                    .ok_or_else(|| protocol("missing leaf group"))?,
            ]]);
        };
        let mut combinations = if self.description.question_type == 19 {
            vec![vec![]]
        } else {
            vec![]
        };
        for (i, child) in children.iter().enumerate() {
            path.push(i);
            let child_sets = child.group_combinations(leaves, path);
            path.pop();
            let child_sets = child_sets?;
            if self.description.question_type == 19 {
                if combinations.len().saturating_mul(child_sets.len()) > 256 {
                    return Err(Error::Unsupported("too many question alternatives".into()));
                }
                combinations = combinations
                    .iter()
                    .flat_map(|left| {
                        child_sets.iter().map(move |right| {
                            let mut combined = left.clone();
                            combined.extend(right);
                            combined
                        })
                    })
                    .collect();
            } else {
                combinations.extend(child_sets);
                if combinations.len() > 256 {
                    return Err(Error::Unsupported("too many question alternatives".into()));
                }
            }
        }
        Ok(combinations)
    }
    fn index_leaves<'a>(
        &'a self,
        c: &impl QuestionContext,
        out: &mut Indexed<'a>,
        path: &mut Vec<usize>,
    ) -> Result<()> {
        if let QuestionKind::Complex(children) = &self.kind {
            for (i, child) in children.iter().enumerate() {
                path.push(i);
                child.index_leaves(c, out, path)?;
                path.pop();
            }
            return Ok(());
        }
        if let QuestionKind::Choice { min, elements, .. } = &self.kind
            && !self.ordered()
            && *min as usize > elements.len()
        {
            return Err(Error::Unsupported(
                "choice minimum exceeds available elements".into(),
            ));
        }
        let leaf = out.leaves.len();
        let group = out.decision.groups.len() as u32;
        let (min, max) = bounds(self);
        let kind = match &self.kind {
            QuestionKind::Choice {
                elements:
                    QuestionElements::Cards(_)
                    | QuestionElements::BuyableCards(_)
                    | QuestionElements::ExplainedCards(_),
                ..
            } => "cards",
            QuestionKind::Number { .. } => "number",
            QuestionKind::Name => "name",
            _ => "choose",
        };
        let mut group_prompt = prompt(self, c);
        if !path.is_empty() {
            group_prompt.push_str(" (Combine groups under AND; choose one branch under OR.)");
        }
        out.decision.groups.push(DecisionGroup {
            group,
            kind: kind.into(),
            prompt: group_prompt,
            min,
            max,
        });
        out.leaves.push((self, path.clone()));
        out.decision.ordered |= self.ordered();
        let mut add = |mut o: AgentOption, target: Target| {
            o.index = out.decision.options.len() as u32;
            o.group = group;
            out.decision.options.push(o);
            out.targets.push((leaf, target));
        };
        let decline = match &self.kind {
            QuestionKind::Choice {
                elements,
                decline,
                defaults,
                ..
            } => {
                for (position, o) in element_options(self, c, elements)?.into_iter().enumerate() {
                    add(o, Target::Select(position as i32));
                }
                if self.ordered() && self.validate_selection(c, defaults).is_ok() {
                    add(
                        option("select", "Keep the current order".into(), None, None),
                        Target::KeepOrder,
                    );
                }
                if !path.is_empty() && self.validate_selection(c, &[]).is_ok() {
                    add(
                        option("decline", "Select none".into(), None, None),
                        Target::Empty,
                    );
                }
                *decline
            }
            QuestionKind::Number {
                min, max, decline, ..
            } => {
                if i64::from(*max) - i64::from(*min) + 1 > MAX_OPTIONS as i64 {
                    return Err(Error::Unsupported(
                        "number question range exceeds option limit".into(),
                    ));
                }
                for value in *min..=*max {
                    add(
                        option("number", value.to_string(), None, Some(value)),
                        Target::Number(value),
                    );
                }
                *decline
            }
            QuestionKind::Name => {
                for (ordinal, name) in c.name_choices() {
                    if c.catalog_name(ordinal).as_deref() != Some(&name) {
                        return Err(protocol("name choice disagrees with catalog"));
                    }
                    add(
                        option("name", name.clone(), Some(name), None),
                        Target::Name(ordinal),
                    );
                }
                -1
            }
            QuestionKind::Delayed { decline } => {
                add(
                    option(
                        "yes",
                        format!("Accept {}", prompt(self, c)),
                        c.card_name(self.description.association),
                        None,
                    ),
                    Target::Accept,
                );
                *decline
            }
            QuestionKind::Complex(_) => unreachable!(),
        };
        if decline >= 0 {
            let label = required(
                c.enum_label("DeclineButtonIds", decline),
                format!("unlabelled decline button {decline}"),
            )?;
            add(
                option(
                    if decline <= 3 { "end-phase" } else { "decline" },
                    label,
                    None,
                    None,
                ),
                Target::Decline,
            );
        } else if decline != -1 {
            return Err(protocol("invalid decline button ordinal"));
        }
        if out.decision.options.len() > MAX_OPTIONS {
            return Err(Error::Unsupported(
                "question has too many projected options".into(),
            ));
        }
        Ok(())
    }
    /// Validates contract indices/names and expectations before returning the
    /// flattened integer list. This method never transmits anything.
    pub fn encode_answer(
        &self,
        c: &impl QuestionContext,
        id: &str,
        answer: &Answer,
        expect: Option<&[String]>,
    ) -> Result<Vec<i32>> {
        if answer.decision != id {
            return Err(rejected(format!(
                "decision {} is not open; the open decision is {id}",
                answer.decision
            )));
        }
        if answer.indices.is_some() == answer.name.is_some() {
            return Err(rejected("give exactly one of indices or name"));
        }
        let indexed = self.indexed(c, id, None)?;
        let mut selected: BTreeMap<usize, Vec<&Target>> = BTreeMap::new();
        if let Some(name) = &answer.name {
            if !indexed.decision.name_any {
                return Err(rejected("this decision does not allow naming any card"));
            }
            let ordinal = c
                .catalog_ordinal(name)
                .ok_or_else(|| rejected(format!("unknown card name {name}")))?;
            return Ok(vec![ordinal]);
        }
        let indices = answer.indices.as_deref().unwrap_or_default();
        if let Some(expect) = expect
            && expect.len() != indices.len()
        {
            return Err(rejected("expect must have one entry per index"));
        }
        let mut seen = BTreeSet::new();
        for (position, &index) in indices.iter().enumerate() {
            if !seen.insert(index) {
                return Err(rejected("indices must not repeat"));
            }
            let o = indexed
                .decision
                .options
                .get(index as usize)
                .ok_or_else(|| {
                    rejected(format!(
                        "option {index} does not exist (decision has {} options)",
                        indexed.decision.options.len()
                    ))
                })?;
            if let Some(expect) = expect
                && !matches_expect(o, &expect[position])
            {
                return Err(rejected(format!(
                    "option {index} is {:?}, not {:?}",
                    o.label, expect[position]
                )));
            }
            let (leaf, target) = &indexed.targets[index as usize];
            selected.entry(*leaf).or_default().push(target);
        }
        if indices.is_empty() {
            if indexed.leaves.len() != 1
                || !matches!(indexed.leaves[0].0.kind, QuestionKind::Choice { .. })
                || bounds(indexed.leaves[0].0).0 != 0
            {
                return Err(rejected(
                    "empty answer requires a single optional selection",
                ));
            }
            selected.insert(0, vec![]);
        }
        let mut answers = BTreeMap::new();
        for (leaf, targets) in selected {
            let (q, path) = &indexed.leaves[leaf];
            let values = if targets.iter().all(|t| matches!(t, Target::Select(_))) {
                let (min, max) = bounds(q);
                if targets.len() < min as usize || targets.len() > max as usize {
                    return Err(rejected(format!(
                        "group {leaf} takes {min}-{max} options, got {}",
                        targets.len()
                    )));
                }
                let mut values: Vec<i32> = targets
                    .iter()
                    .map(|t| match t {
                        Target::Select(v) => *v,
                        _ => unreachable!(),
                    })
                    .collect();
                q.validate_selection(c, &values)?;
                if matches!(q.description.question_type, 8 | 33) {
                    values.reverse();
                }
                values
            } else {
                if targets.len() != 1 {
                    return Err(rejected(
                        "decline, number, name, accept and keep-order must be chosen alone within their group",
                    ));
                }
                match targets[0] {
                    Target::Decline => {
                        if matches!(q.kind, QuestionKind::Delayed { .. }) {
                            vec![0]
                        } else {
                            vec![]
                        }
                    }
                    Target::Number(v) | Target::Name(v) => vec![*v],
                    Target::Accept => vec![1],
                    Target::Empty => {
                        q.validate_selection(c, &[])?;
                        vec![]
                    }
                    Target::KeepOrder => match &q.kind {
                        QuestionKind::Choice { defaults, .. } => defaults.clone(),
                        _ => unreachable!(),
                    },
                    Target::Select(_) => unreachable!(),
                }
            };
            answers.insert(path.clone(), values);
        }
        let (values, active) = self.compose(c, &answers, &mut vec![])?;
        if !active {
            return Err(rejected("answer does not select a question"));
        }
        Ok(values)
    }
    fn compose(
        &self,
        c: &impl QuestionContext,
        answers: &BTreeMap<Vec<usize>, Vec<i32>>,
        path: &mut Vec<usize>,
    ) -> Result<(Vec<i32>, bool)> {
        if let Some(values) = answers.get(path) {
            return Ok((values.clone(), true));
        }
        if let QuestionKind::Complex(children) = &self.kind {
            if self.description.question_type == 18 {
                let active: Vec<_> = (0..children.len())
                    .filter(|&i| {
                        let mut branch = path.clone();
                        branch.push(i);
                        answers.keys().any(|key| key.starts_with(&branch))
                    })
                    .collect();
                if active.len() != 1 {
                    return Err(rejected("choose exactly one branch of an OR question"));
                }
                let i = active[0];
                path.push(i);
                let result = children[i].compose(c, answers, path);
                path.pop();
                let (values, _) = result?;
                let mut out = vec![i as i32];
                out.extend(values);
                return Ok((out, true));
            }
            let mut results = vec![];
            for (i, child) in children.iter().enumerate() {
                path.push(i);
                results.push(child.compose(c, answers, path));
                path.pop();
            }
            let mut out = vec![];
            let mut active = false;
            for result in results {
                let (values, used) = result?;
                active |= used;
                out.push(values.len() as i32);
                out.extend(values);
            }
            return Ok((out, active));
        }
        let values = match &self.kind {
            QuestionKind::Choice { defaults, min, .. } => {
                // Upstream getAnswer leaves an untouched AND sibling empty even
                // when its minimum applies to an explicit selection.
                if defaults.len() < *min as usize {
                    vec![]
                } else {
                    self.validate_selection(c, defaults)?;
                    defaults.clone()
                }
            }
            QuestionKind::Number { default, .. } => vec![*default],
            QuestionKind::Name | QuestionKind::Delayed { .. } => vec![],
            QuestionKind::Complex(_) => unreachable!(),
        };
        Ok((values, false))
    }

    fn validate_selection(&self, c: &impl QuestionContext, values: &[i32]) -> Result<()> {
        let QuestionKind::Choice { elements, .. } = &self.kind else {
            return Err(protocol("selection for non-choice question"));
        };
        let (min, max) = bounds(self);
        if values.len() < min as usize
            || values.len() > max as usize
            || values
                .iter()
                .any(|v| *v < 0 || *v as usize >= elements.len())
            || values.iter().collect::<BTreeSet<_>>().len() != values.len()
        {
            return Err(rejected(format!(
                "{} requires a valid selection",
                self.type_name()
            )));
        }
        if let QuestionElements::Cleanup(items) = elements
            && items
                .iter()
                .enumerate()
                .any(|(i, item)| !item.is_optional && !values.contains(&(i as i32)))
        {
            return Err(rejected("mandatory cleanup items must be selected"));
        }
        if matches!(self.description.question_type, 4 | 42) {
            let QuestionElements::Cards(cards) = elements else {
                return Err(protocol("name-constrained selection without card elements"));
            };
            let names = values
                .iter()
                .map(|v| {
                    required(
                        c.card_name(cards[*v as usize]),
                        "name-constrained card is not visible",
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let distinct = names.iter().collect::<BTreeSet<_>>().len();
            if (self.description.question_type == 4 && distinct != names.len())
                || (self.description.question_type == 42 && distinct > 1)
            {
                return Err(rejected(
                    "selection violates the question's card-name constraint",
                ));
            }
        }
        Ok(())
    }
}
fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn matches_expect(option: &AgentOption, expect: &str) -> bool {
    let expected = normalized(expect);
    if expected.is_empty() {
        return false;
    }
    let without_parenthesis = if option.label.ends_with(')') {
        option
            .label
            .rsplit_once('(')
            .map(|(prefix, _)| prefix)
            .unwrap_or(&option.label)
    } else {
        &option.label
    };
    normalized(&option.label) == expected
        || normalized(without_parenthesis) == expected
        || option
            .card
            .as_deref()
            .is_some_and(|name| normalized(name) == expected)
}

#[cfg(test)]
mod campaign;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;
    pub(super) struct Context;
    impl QuestionContext for Context {
        fn card_name(&self, id: i32) -> Option<String> {
            match id {
                7 | 11 => Some("Copper".into()),
                9 => Some("Village".into()),
                _ => None,
            }
        }
        fn catalog_name(&self, id: i32) -> Option<String> {
            match id {
                2 => Some("Copper".into()),
                31 => Some("Village".into()),
                _ => None,
            }
        }
        fn catalog_ordinal(&self, name: &str) -> Option<i32> {
            match normalized(name).as_str() {
                "copper" => Some(2),
                "village" => Some(31),
                _ => None,
            }
        }
        fn zone_name(&self, id: i32) -> Option<String> {
            Some(format!("zone {id}"))
        }
        fn enum_label(&self, catalog: &str, id: i32) -> Option<String> {
            Some(format!("{catalog} {id}"))
        }
        fn ability_text(&self, a: &Ability) -> Option<String> {
            (!a.arguments.is_empty()).then(|| "Draw 2 cards".into())
        }
        fn name_choices(&self) -> Vec<(i32, String)> {
            vec![(31, "Village".into()), (2, "Copper".into())]
        }
    }
    fn story(w: &mut Writer) {
        w.int(0);
        w.int(0);
        w.boolean(false);
    }
    fn description(w: &mut Writer, kind: i32) {
        w.int(kind);
        w.int(-1);
        story(w);
        story(w);
    }
    fn choice(w: &mut Writer, kind: i32, min: i32, max: i32, defaults: &[i32]) {
        description(w, kind);
        w.int(min);
        w.int(max);
        w.int(2);
        w.int(0);
        w.int(7);
        w.int(9);
        w.int(-1);
        w.ints(defaults);
        w.ints(&[]);
    }
    fn root(tag: i32, body: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::new();
        w.int(42);
        w.int(tag);
        body(&mut w);
        w.finish()
    }
    fn parse(bytes: &[u8]) -> Question {
        let mut r = Reader::new(bytes);
        let q = Question::parse(&mut r).unwrap();
        r.finish().unwrap();
        q
    }
    fn encode(q: &Question, indices: Vec<u32>) -> Vec<i32> {
        q.encode_answer(
            &Context,
            "epoch1-d42",
            &Answer::indices("epoch1-d42", indices),
            None,
        )
        .unwrap()
    }
    #[test]
    fn upstream_choice_fixture_positions_and_exact_consumption() {
        let bytes = root(0, |w| choice(w, 0, 1, 2, &[]));
        let q = parse(&bytes);
        assert_eq!(q.index, 42);
        assert_eq!(encode(&q, vec![1]), [1]); // Neither instance 9 nor catalog ordinal 31.
        for end in 0..bytes.len() {
            assert!(
                Question::parse(&mut Reader::new(&bytes[..end])).is_err(),
                "prefix {end}"
            );
        }
        let d = q.project(&Context, "epoch1-d42", Some(0)).unwrap();
        assert_eq!(d.options[1].label, "Play Village");
    }
    #[test]
    fn number_name_and_delayed_encodings() {
        let number = parse(&root(1, |w| {
            description(w, 11);
            w.int(3);
            w.int(5);
            w.int(3);
            w.int(4);
        }));
        assert_eq!(encode(&number, vec![2]), [5]);
        assert!(encode(&number, vec![3]).is_empty());
        let name = parse(&root(4, |w| description(w, 20)));
        assert_eq!(encode(&name, vec![0]), [31]);
        assert_eq!(
            name.encode_answer(&Context, "n", &Answer::name("n", " VIL-lage "), None)
                .unwrap(),
            [31]
        );
        let delayed = parse(&root(3, |w| {
            description(w, 21);
            w.int(4);
        }));
        assert_eq!(encode(&delayed, vec![0]), [1]);
        assert_eq!(encode(&delayed, vec![1]), [0]);
    }
    #[test]
    fn nested_and_or_lengths_and_unselected_defaults() {
        let q = parse(&root(2, |w| {
            description(w, 19);
            w.int(2);
            w.int(0);
            choice(w, 2, 1, 2, &[]);
            w.int(2);
            description(w, 18);
            w.int(2);
            w.int(4);
            description(w, 20);
            w.int(1);
            description(w, 11);
            w.int(8);
            w.int(10);
            w.int(9);
            w.int(-1);
        }));
        assert_eq!(encode(&q, vec![1, 5]), [1, 1, 2, 1, 9]);
        assert!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![1, 2, 4]), None)
                .is_err()
        );
        assert_eq!(encode(&q, vec![5]), [0, 2, 1, 9]);
        let defaulted = parse(&root(2, |w| {
            description(w, 19);
            w.int(2);
            w.int(0);
            choice(w, 2, 1, 1, &[]);
            w.int(1);
            description(w, 11);
            w.int(8);
            w.int(10);
            w.int(9);
            w.int(-1);
        }));
        assert_eq!(encode(&defaulted, vec![0]), [1, 0, 1, 9]);
    }
    #[test]
    fn ordered_contract_is_top_first_keep_order_stays_bottom_first() {
        let q = parse(&root(0, |w| choice(w, 8, 2, 2, &[0, 1])));
        assert!(q.project(&Context, "d", None).unwrap().ordered);
        assert_eq!(encode(&q, vec![0, 1]), [1, 0]);
        assert_eq!(encode(&q, vec![2]), [0, 1]);
        assert!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![0]), None)
                .is_err()
        );
    }
    #[test]
    fn validates_staleness_expectations_duplicates_and_empty() {
        let q = parse(&root(0, |w| choice(w, 9, 1, 1, &[])));
        let answer = Answer::indices("d", vec![1]);
        assert!(q.encode_answer(&Context, "new", &answer, None).is_err());
        assert!(
            q.encode_answer(&Context, "d", &answer, Some(&["Buy Copper".into()]))
                .is_err()
        );
        assert_eq!(
            q.encode_answer(&Context, "d", &answer, Some(&["buy VILLAGE!".into()]))
                .unwrap(),
            [1]
        );
        for indices in [vec![], vec![0, 0], vec![4], vec![0, 1]] {
            assert!(
                q.encode_answer(&Context, "d", &Answer::indices("d", indices), None)
                    .is_err()
            );
        }
        let optional = parse(&root(0, |w| choice(w, 2, 0, 2, &[])));
        assert!(encode(&optional, vec![]).is_empty());
    }
    #[test]
    fn rejects_unknown_tags_bounds_and_excessive_nesting() {
        assert!(Question::parse(&mut Reader::new(&root(5, |_| {}))).is_err());
        assert!(
            Question::parse(&mut Reader::new(&root(1, |w| {
                description(w, 11);
                w.int(5);
                w.int(3);
                w.int(4);
                w.int(-1);
            })))
            .is_err()
        );
        let unknown_elements = root(0, |w| {
            description(w, 0);
            w.int(0);
            w.int(1);
            w.int(1);
            w.int(10);
            w.int(0);
        });
        assert!(Question::parse(&mut Reader::new(&unknown_elements)).is_err());
        let nested = root(2, |w| {
            for _ in 0..50 {
                description(w, 19);
                w.int(1);
                w.int(2);
            }
            description(w, 19);
            w.int(0);
        });
        assert!(Question::parse(&mut Reader::new(&nested)).is_err());
        assert!(LogArgument::parse(&mut Reader::new(&16_i32.to_be_bytes())).is_err());
    }
    #[test]
    fn parses_all_element_shapes_and_contextual_log_arguments() {
        for tag in 0..10 {
            let bytes = root(0, |w| {
                description(w, 0);
                w.int(0);
                w.int(1);
                w.int(1);
                w.int(tag);
                match tag {
                    0 | 3 | 4 | 5 | 6 => w.int(7),
                    1 => {
                        w.int(2);
                        w.int(3);
                        w.int(7);
                        w.int(1);
                        w.int(4);
                        w.int(9);
                        w.boolean(true);
                        w.int(0);
                        w.int(1);
                    }
                    2 => {
                        w.int(0);
                        w.int(1);
                        w.int(2);
                        w.int(0);
                    }
                    7 => {
                        w.ints(&[2, 31]);
                        w.boolean(true);
                        w.boolean(false);
                        w.boolean(true);
                    }
                    8 => {
                        w.int(7);
                        w.boolean(true);
                    }
                    9 => {
                        w.int(7);
                        w.int(5);
                        w.int(-1);
                    }
                    _ => unreachable!(),
                }
                w.int(-1);
                w.ints(&[]);
                w.ints(&[]);
            });
            assert!(
                matches!(parse(&bytes).kind,QuestionKind::Choice{elements,..} if elements.len()==1)
            );
        }
        let mut w = Writer::new();
        w.int(3);
        w.int(3);
        w.int(3);
        w.int(0);
        w.int(1);
        w.int(0);
        w.int(1);
        w.int(31);
        w.int(2);
        w.int(4);
        w.int(9);
        w.boolean(true);
        w.int(10);
        w.int(7);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let s = Story::parse(&mut r).unwrap();
        r.finish().unwrap();
        assert_eq!(s.context, Some(LogArgument::Card(7)));
        assert_eq!(
            s.arguments,
            vec![
                LogArgument::OwnZone { player: 0, zone: 1 },
                LogArgument::CardNames(vec![CardFrequency {
                    name: 31,
                    frequency: 2
                }]),
                LogArgument::Number(9)
            ]
        );
    }
    #[test]
    fn contextual_abilities_and_mandatory_cleanup_project_and_validate() {
        let mut q = parse(&root(0, |w| choice(w, 7, 1, 1, &[])));
        let QuestionKind::Choice { elements, .. } = &mut q.kind else {
            unreachable!()
        };
        *elements = QuestionElements::Abilities(vec![Ability {
            description_type: 2,
            name: 2,
            association: 7,
            arguments: vec![LogArgument::Number(2)],
            optional: true,
            copy_index: 0,
            copy_total: 1,
        }]);
        let d = q.project(&Context, "d", None).unwrap();
        assert_eq!(d.options[0].label, "Copper: Draw 2 cards");
        assert_eq!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![0]), None)
                .unwrap(),
            [0]
        );
        q.kind = QuestionKind::Choice {
            min: 0,
            max: 2,
            decline: -1,
            defaults: vec![],
            affected: vec![],
            elements: QuestionElements::Cleanup(vec![
                CleanupAction {
                    names: vec![2],
                    is_optional: false,
                    allows_reordering: false,
                    is_enabled: true,
                },
                CleanupAction {
                    names: vec![31],
                    is_optional: true,
                    allows_reordering: false,
                    is_enabled: false,
                },
            ]),
        };
        assert_eq!(
            q.project(&Context, "d", None).unwrap().options[0].label,
            "Topdeck Copper"
        );
        assert!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![]), None)
                .is_err()
        );
        assert!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![1]), None)
                .is_err()
        );
        assert_eq!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![0]), None)
                .unwrap(),
            [0]
        );
    }
    #[test]
    fn or_cannot_ignore_an_invalid_selected_branch() {
        let q = parse(&root(2, |w| {
            description(w, 18);
            w.int(2);
            w.int(2);
            description(w, 19);
            w.int(2);
            w.int(0);
            choice(w, 2, 1, 1, &[]);
            w.int(0);
            choice(w, 3, 1, 1, &[]);
            w.int(4);
            description(w, 20);
        }));
        assert!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![0, 4]), None)
                .is_err()
        );
        assert_eq!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![4]), None)
                .unwrap(),
            [1, 31]
        );
    }
    #[test]
    fn cleanup_order_follows_the_upstream_cleanup_window() {
        let mut q = parse(&root(0, |w| choice(w, 0, 0, 2, &[])));
        q.description.question_type = 35;
        q.kind = QuestionKind::Choice {
            min: 0,
            max: 2,
            decline: -1,
            defaults: vec![],
            affected: vec![],
            elements: QuestionElements::Cleanup(vec![
                CleanupAction {
                    names: vec![2],
                    is_optional: false,
                    allows_reordering: true,
                    is_enabled: true,
                },
                CleanupAction {
                    names: vec![31],
                    is_optional: true,
                    allows_reordering: true,
                    is_enabled: true,
                },
            ]),
        };
        assert!(q.project(&Context, "d", None).unwrap().ordered);
        assert_eq!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![1, 0]), None)
                .unwrap(),
            [1, 0]
        );
        assert!(
            q.encode_answer(&Context, "d", &Answer::indices("d", vec![1]), None)
                .is_err()
        );
    }
}

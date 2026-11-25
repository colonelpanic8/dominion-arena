use super::{catalog::WireCatalog, projection::ProjectionError};
use dominion_engine::{
    Card, CardId, Decision, DecisionKind, Move, Opt, Phase, Purpose, Response, View,
};
use dominion_online::questions::{
    Ability, Question, QuestionDescription, QuestionElements, QuestionKind, Story,
};
use std::collections::{BTreeMap, BTreeSet};
pub(super) mod semantics;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum AnswerRejection {
    #[error("stale wire question index or projection epoch")]
    StaleQuestion,
    #[error("malformed answer: {0}")]
    Malformed(&'static str),
}
pub(crate) struct PreparedQuestion {
    pub wire: Question,
    pub decision_id: String,
    pub epoch: u64,
    plan: AnswerPlan,
}
impl PreparedQuestion {
    pub(crate) fn resolve_at(
        &self,
        epoch: u64,
        question_index: i32,
        values: &[i32],
    ) -> Result<Response, AnswerRejection> {
        if epoch != self.epoch {
            return Err(AnswerRejection::StaleQuestion);
        }
        self.resolve(question_index, values)
    }
    pub(crate) fn resolve(
        &self,
        question_index: i32,
        values: &[i32],
    ) -> Result<Response, AnswerRejection> {
        if question_index != self.wire.index {
            return Err(AnswerRejection::StaleQuestion);
        }
        self.plan.resolve(values)
    }
}

enum AnswerPlan {
    Moves {
        targets: Vec<Move>,
        decline: bool,
    },
    Cards {
        targets: Vec<CardId>,
        min: usize,
        max: usize,
        reverse: bool,
    },
    Piles {
        targets: Vec<dominion_engine::PileId>,
        optional: bool,
    },
    Options {
        targets: Vec<Opt>,
        min: usize,
        max: usize,
        decline: Option<Opt>,
    },
    Names(BTreeMap<i32, Card>),
    Number {
        min: i32,
        max: i32,
        reverse: bool,
    },
    Constant(Response),
    Or(Vec<AnswerPlan>),
}
impl AnswerPlan {
    fn resolve(&self, values: &[i32]) -> Result<Response, AnswerRejection> {
        use AnswerRejection::Malformed as M;
        Ok(match self {
            Self::Or(children) => {
                let (branch, rest) = values.split_first().ok_or(M("missing OR branch"))?;
                return position(children, *branch)?.resolve(rest);
            }
            Self::Moves { targets, decline } => {
                if values.is_empty() && *decline {
                    Response::Move(Move::EndPhase)
                } else {
                    if values.len() != 1 {
                        return Err(M("select exactly one move"));
                    }
                    Response::Move(*position(targets, values[0])?)
                }
            }
            Self::Cards {
                targets,
                min,
                max,
                reverse,
            } => {
                let mut selected = selected(targets, values, *min, *max)?;
                if *reverse {
                    selected.reverse();
                }
                Response::Cards(selected)
            }
            Self::Piles { targets, optional } => {
                if values.is_empty() && *optional {
                    Response::Pile(None)
                } else {
                    if values.len() != 1 {
                        return Err(M("select exactly one pile"));
                    }
                    Response::Pile(Some(*position(targets, values[0])?))
                }
            }
            Self::Options {
                targets,
                min,
                max,
                decline,
            } => {
                if values.is_empty() && decline.is_some() {
                    Response::option(decline.unwrap())
                } else {
                    Response::Options(selected(targets, values, *min, *max)?)
                }
            }
            Self::Names(names) => {
                if values.len() != 1 {
                    return Err(M("name exactly one card"));
                }
                Response::Name(
                    *names
                        .get(&values[0])
                        .ok_or(M("name outside physical catalog cards"))?,
                )
            }
            Self::Number { min, max, reverse } => {
                if values.len() != 1 || !(*min..=*max).contains(&values[0]) {
                    return Err(M("number outside range"));
                }
                Response::Number(if *reverse {
                    max - (values[0] - min)
                } else {
                    values[0]
                })
            }
            Self::Constant(response) => {
                if !values.is_empty() {
                    return Err(M("unexpected branch arguments"));
                }
                response.clone()
            }
        })
    }
}
fn selected<T: Copy>(
    targets: &[T],
    values: &[i32],
    min: usize,
    max: usize,
) -> Result<Vec<T>, AnswerRejection> {
    if !(min..=max).contains(&values.len()) {
        return Err(AnswerRejection::Malformed("invalid selection count"));
    }
    let mut seen = BTreeSet::new();
    values
        .iter()
        .map(|&i| {
            if !seen.insert(i) {
                return Err(AnswerRejection::Malformed("duplicate selection"));
            }
            position(targets, i).copied()
        })
        .collect()
}
fn position<T>(targets: &[T], value: i32) -> Result<&T, AnswerRejection> {
    usize::try_from(value)
        .ok()
        .and_then(|i| targets.get(i))
        .ok_or(AnswerRejection::Malformed(
            "position outside offered elements",
        ))
}
fn description(
    c: &WireCatalog,
    kind: &str,
    story: &str,
    association: i32,
) -> Result<QuestionDescription, ProjectionError> {
    described(c, kind, story, association, vec![])
}
fn described(
    c: &WireCatalog,
    kind: &str,
    story: &str,
    association: i32,
    arguments: Vec<dominion_online::questions::LogArgument>,
) -> Result<QuestionDescription, ProjectionError> {
    let story = Story {
        question_id: c.symbol("QuestionIds", story)?,
        arguments,
        context: None,
    };
    Ok(QuestionDescription {
        question_type: c.symbol("QuestionTypes", kind)?,
        association,
        short_story: story.clone(),
        story,
    })
}
fn choice(
    description: QuestionDescription,
    elements: QuestionElements,
    min: usize,
    max: usize,
    decline: i32,
) -> Result<Question, ProjectionError> {
    if min > max || max > elements.len() {
        return Err(ProjectionError::Inconsistent("question bounds"));
    }
    Ok(Question {
        index: 0,
        description,
        kind: QuestionKind::Choice {
            min: super::projection::integer(min, "question minimum")?,
            max: super::projection::integer(max, "question maximum")?,
            elements,
            decline,
            defaults: vec![],
            affected: vec![],
        },
    })
}
/// A set's own decline button for `d`, else the one its purpose names.
fn decline_for(c: &WireCatalog, d: &Decision, min: usize) -> Result<i32, ProjectionError> {
    match semantics::decline_button(d) {
        Some(button) if min == 0 => c.symbol("DeclineButtonIds", button),
        _ => decline(c, d.purpose, min),
    }
}
fn decline(c: &WireCatalog, purpose: Purpose, min: usize) -> Result<i32, ProjectionError> {
    if min != 0 {
        return Ok(-1);
    }
    c.symbol(
        "DeclineButtonIds",
        match purpose {
            Purpose::Trash => "DONT_TRASH",
            Purpose::Discard => "DONT_DISCARD",
            Purpose::Gain => "DONT_GAIN",
            Purpose::Play => "DONT_PLAY",
            Purpose::TopDeck => "DONT_TOPDECK",
            Purpose::Reveal => "DONT_REVEAL",
            Purpose::SetAside => "DONE_SETTING_ASIDE",
            _ => "DONT",
        },
    )
}

pub(super) fn prepare(
    view: &View<'_>,
    d: &Decision,
    decision_id: &str,
    question_index: i32,
    ids: &BTreeMap<CardId, i32>,
    landscapes: &BTreeMap<Card, i32>,
    c: &WireCatalog,
) -> Result<PreparedQuestion, ProjectionError> {
    if question_index <= 0 {
        return Err(ProjectionError::Size("positive question index required"));
    }
    if decision_id.is_empty() {
        return Err(ProjectionError::Inconsistent("empty decision identity"));
    }
    let id = |card: &CardId| {
        ids.get(card).copied().ok_or(ProjectionError::Inconsistent(
            "question card is not disclosed",
        ))
    };
    let association = view
        .pending_source()
        .and_then(|card| ids.get(&card).copied())
        .or_else(|| d.source.and_then(|s| landscapes.get(&s).copied()))
        .unwrap_or(-1);
    let (mut wire, plan) = match &d.kind {
        DecisionKind::Turn { phase, moves } if d.purpose == Purpose::Turn => {
            let mut plays = vec![];
            let mut buys = vec![];
            let mut landscapes = vec![];
            let mut resources = vec![];
            let mut seen = std::collections::HashSet::new();
            for &m in moves {
                if !seen.insert(m) {
                    return Err(ProjectionError::Inconsistent("duplicate move"));
                }
                match m {
                    Move::Play(card) => plays.push((id(&card)?, m)),
                    Move::Buy(pile) => buys.push((
                        id(&view
                            .pile(pile)
                            .top()
                            .ok_or(ProjectionError::Inconsistent("buy empty pile"))?)?,
                        m,
                    )),
                    Move::BuyLandscape(card) => landscapes.push((c.card(card)?, m)),
                    Move::Coffers | Move::Villager | Move::PayDebt => resources.push(m),
                    Move::EndPhase => {}
                }
            }
            if !moves.contains(&Move::EndPhase) {
                return Err(semantics::gap(d, "turn without end move"));
            }
            plays.sort_by_key(|x| x.0);
            buys.sort_by_key(|x| x.0);
            landscapes.sort_by_key(|x| x.0);
            let mut children = vec![];
            let mut plans = vec![];
            let button = match phase {
                Phase::Action => "END_ACTION_PHASE",
                Phase::Buy => "END_BUY_PHASE",
                Phase::Night => "END_TURN",
                _ => return Err(semantics::gap(d, "unsupported turn phase")),
            };
            let phase_story = match phase {
                Phase::Action => "GAME_ACTION_PHASE",
                Phase::Buy => "GAME_BUY_PHASE",
                _ => "WHAT_NIGHT_TO_PLAY",
            };
            let play_story = match phase {
                Phase::Action => "GAME_ACTION_PHASE",
                Phase::Buy => "GAME_MAY_PLAY_TREASURES",
                _ => "WHAT_NIGHT_TO_PLAY",
            };
            if *phase != Phase::Buy || !plays.is_empty() {
                let end = *phase != Phase::Buy;
                children.push(choice(
                    description(c, "PLAY", play_story, -1)?,
                    QuestionElements::Cards(plays.iter().map(|x| x.0).collect()),
                    usize::from(!end),
                    usize::from(!plays.is_empty()),
                    if end {
                        c.symbol("DeclineButtonIds", button)?
                    } else {
                        -1
                    },
                )?);
                plans.push(AnswerPlan::Moves {
                    targets: plays.iter().map(|x| x.1).collect(),
                    decline: end,
                });
            }
            if *phase == Phase::Buy {
                let story = if plays.is_empty() {
                    "GAME_MAY_BUY"
                } else {
                    "GAME_MAY_START_BUYING"
                };
                children.push(choice(
                    description(c, "BUY", story, -1)?,
                    QuestionElements::BuyableCards(buys.iter().map(|x| (x.0, false)).collect()),
                    0,
                    usize::from(!buys.is_empty()),
                    c.symbol("DeclineButtonIds", button)?,
                )?);
                plans.push(AnswerPlan::Moves {
                    targets: buys.iter().map(|x| x.1).collect(),
                    decline: true,
                });
                if !landscapes.is_empty() {
                    children.push(choice(
                        description(c, "BUY", "GAME_MAY_BUY_NONCARDS", -1)?,
                        QuestionElements::Names(landscapes.iter().map(|x| x.0).collect()),
                        1,
                        1,
                        -1,
                    )?);
                    plans.push(AnswerPlan::Moves {
                        targets: landscapes.iter().map(|x| x.1).collect(),
                        decline: false,
                    });
                }
            }
            for m in resources {
                let (story, button) = match m {
                    Move::Coffers => ("GAME_USE_COFFERS", "USE_COFFERS"),
                    Move::Villager => ("GAME_USE_VILLAGER", "USE_VILLAGER"),
                    _ => ("GAME_PAY_OFF_DEBT", "PAY_OFF_DEBT"),
                };
                children.push(choice(
                    description(c, "BUTTON", story, -1)?,
                    QuestionElements::Buttons(vec![c.symbol("GameButtons", button)?]),
                    1,
                    1,
                    -1,
                )?);
                plans.push(AnswerPlan::Moves {
                    targets: vec![m],
                    decline: false,
                });
            }
            if children.len() == 1 {
                (children.remove(0), plans.remove(0))
            } else {
                (
                    Question {
                        index: 0,
                        description: description(c, "COMPLEX_OR", phase_story, -1)?,
                        kind: QuestionKind::Complex(children),
                    },
                    AnswerPlan::Or(plans),
                )
            }
        }
        DecisionKind::Choose {
            options, min, max, ..
        } if d.purpose == Purpose::Trigger => {
            let triggers = view.pending_triggers();
            let occasions = view.pending_occasions();
            let mut abilities = vec![];
            let mut targets = vec![];
            let mut optional = false;
            for &o in options {
                match o {
                    Opt::Trigger(i) => {
                        let t = triggers
                            .get(usize::from(i))
                            .ok_or(ProjectionError::Inconsistent("trigger index"))?;
                        let symbol = semantics::ability(t.card, t.ability).ok_or_else(|| {
                            semantics::gap(d, &format!("ability {}:{}", t.card.name(), t.ability))
                        })?;
                        let arguments = if t.card == Card::Haven {
                            let lasting = view
                                .lasting()
                                .into_iter()
                                .find(|l| Some(l.id) == t.lasting)
                                .ok_or_else(|| semantics::gap(d, "missing Haven lasting"))?;
                            let card =
                                lasting.data.pick.and_then(|id| view.card(id)).ok_or_else(
                                    || semantics::gap(d, "missing entitled Haven card"),
                                )?;
                            vec![dominion_online::questions::LogArgument::CardNames(vec![
                                dominion_online::questions::CardFrequency {
                                    name: c.card(card)?,
                                    frequency: 1,
                                },
                            ])]
                        } else {
                            semantics::ability_arguments(
                                view,
                                t,
                                occasions.get(usize::from(i)).copied(),
                                c,
                            )
                            .transpose()?
                            .unwrap_or_default()
                        };
                        abilities.push(Ability {
                            description_type: if arguments.is_empty() { 1 } else { 2 },
                            name: c.symbol("AbilityNames", symbol)?,
                            association: t
                                .source
                                .and_then(|s| ids.get(&s).copied())
                                .or_else(|| landscapes.get(&t.card).copied())
                                .ok_or_else(|| {
                                    semantics::gap(d, "ability association unavailable")
                                })?,
                            arguments,
                            optional: t.optional,
                            copy_index: 0,
                            copy_total: 1,
                        });
                        targets.push(o);
                    }
                    Opt::No => optional = true,
                    _ => return Err(semantics::gap(d, "non-trigger option in trigger window")),
                }
            }
            let story = if optional {
                "GAME_MAY_REACT_WITH"
            } else {
                "GAME_RESOLVE_ABILITY"
            };
            (
                choice(
                    description(c, "RESOLVE_ABILITY", story, -1)?,
                    QuestionElements::Abilities(abilities),
                    if optional { 0 } else { *min },
                    *max,
                    if optional {
                        c.symbol("DeclineButtonIds", "DONE_REACTING")?
                    } else {
                        -1
                    },
                )?,
                AnswerPlan::Options {
                    targets,
                    min: *min,
                    max: *max,
                    decline: optional.then_some(Opt::No),
                },
            )
        }
        _ => {
            let (kind, story, args) = semantics::binding(view, d, c)?;
            let desc = described(c, kind, story, association, args)?;
            match &d.kind {
                DecisionKind::Cards {
                    candidates,
                    min,
                    max,
                    ordered,
                } => {
                    let mut pairs = candidates
                        .iter()
                        .map(|x| Ok((id(x)?, *x)))
                        .collect::<Result<Vec<_>, ProjectionError>>()?;
                    if *ordered {
                        pairs.reverse();
                    } else {
                        pairs.sort_by_key(|x| x.0);
                    }
                    (
                        choice(
                            desc,
                            QuestionElements::Cards(pairs.iter().map(|x| x.0).collect()),
                            *min,
                            *max,
                            decline_for(c, d, *min)?,
                        )?,
                        AnswerPlan::Cards {
                            targets: pairs.iter().map(|x| x.1).collect(),
                            min: *min,
                            max: *max,
                            reverse: *ordered,
                        },
                    )
                }
                DecisionKind::Pile { piles, optional } => {
                    let pairs = piles
                        .iter()
                        .map(|&p| {
                            Ok((
                                id(&view
                                    .pile(p)
                                    .top()
                                    .ok_or(ProjectionError::Inconsistent("select empty pile"))?)?,
                                p,
                            ))
                        })
                        .collect::<Result<Vec<_>, ProjectionError>>()?;
                    (
                        choice(
                            desc,
                            QuestionElements::Cards(pairs.iter().map(|x| x.0).collect()),
                            usize::from(!optional),
                            1,
                            decline_for(c, d, usize::from(!optional))?,
                        )?,
                        AnswerPlan::Piles {
                            targets: pairs.iter().map(|x| x.1).collect(),
                            optional: *optional,
                        },
                    )
                }
                DecisionKind::NameCard => {
                    let names = Card::ALL
                        .iter()
                        .filter(|card| card.kind() == dominion_engine::Kind::Card)
                        .filter_map(|&card| c.card(card).ok().map(|name| (name, card)))
                        .collect();
                    (
                        Question {
                            index: 0,
                            description: desc,
                            kind: QuestionKind::Name,
                        },
                        AnswerPlan::Names(names),
                    )
                }
                DecisionKind::Number { min, max } => {
                    // A location counts from the bottom; an amount is itself, none by default.
                    let reverse = match kind {
                        "LOCATION" => true,
                        "OVERPAY" | "OVERPAY_COINS" | "OVERPAY_POTIONS" => false,
                        _ => return Err(semantics::gap(d, "number meaning")),
                    };
                    (
                        Question {
                            index: 0,
                            description: desc,
                            kind: QuestionKind::Number {
                                min: *min,
                                max: *max,
                                default: if reverse { *max } else { *min },
                                decline: -1,
                            },
                        },
                        AnswerPlan::Number {
                            min: *min,
                            max: *max,
                            reverse,
                        },
                    )
                }
                DecisionKind::Choose {
                    options, min, max, ..
                } => {
                    if let Some(shown) = semantics::choice(view, d) {
                        set_choice(c, d, desc, association, shown, options, *min, *max, &id)?
                    } else if options.iter().all(|o| matches!(o, Opt::Mode(_)))
                        && d.source != Some(Card::Torturer)
                    {
                        let source = d
                            .source
                            .ok_or_else(|| semantics::gap(d, "mode without source"))?;
                        let modes = options
                            .iter()
                            .map(|o| {
                                let Opt::Mode(m) = o else { unreachable!() };
                                c.symbol(
                                    "CardModes",
                                    semantics::mode(source, *m)
                                        .ok_or_else(|| semantics::gap(d, "unmapped mode"))?,
                                )
                            })
                            .collect::<Result<Vec<_>, ProjectionError>>()?;
                        (
                            choice(desc, QuestionElements::Modes(modes), *min, *max, -1)?,
                            AnswerPlan::Options {
                                targets: options.clone(),
                                min: *min,
                                max: *max,
                                decline: None,
                            },
                        )
                    } else if d.source == Some(Card::Library) {
                        let modes = options
                            .iter()
                            .map(|o| {
                                c.symbol(
                                    "CardModes",
                                    if *o == Opt::Yes {
                                        "LIBRARY_SKIP_IT"
                                    } else {
                                        "LIBRARY_DRAW_IT"
                                    },
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        (
                            choice(desc, QuestionElements::Modes(modes), *min, *max, -1)?,
                            AnswerPlan::Options {
                                targets: options.clone(),
                                min: *min,
                                max: *max,
                                decline: None,
                            },
                        )
                    } else if options == &[Opt::Yes, Opt::No]
                        && matches!(
                            d.source,
                            Some(Card::Vassal | Card::MiningVillage | Card::Engineer)
                        )
                    {
                        let target = if d.source == Some(Card::Vassal) {
                            view.pending_reference()
                        } else {
                            view.pending_source()
                        }
                        .ok_or_else(|| {
                            semantics::gap(d, "optional effect missing visible source")
                        })?;
                        (
                            choice(
                                desc,
                                QuestionElements::Cards(vec![id(&target)?]),
                                0,
                                1,
                                decline(c, d.purpose, 0)?,
                            )?,
                            AnswerPlan::Options {
                                targets: vec![Opt::Yes],
                                min: 1,
                                max: 1,
                                decline: Some(Opt::No),
                            },
                        )
                    } else if matches!(d.source, Some(Card::Torturer | Card::Mill)) {
                        let mut children = vec![];
                        let mut plans = vec![];
                        for &o in options {
                            let (story, button) = match (d.source, o) {
                                (Some(Card::Torturer), Opt::Mode(0)) => {
                                    ("TORTURER_MAY_DISCARD", "CONTINUE")
                                }
                                (Some(Card::Torturer), Opt::Mode(1)) => {
                                    ("TORTURER_MAY_CURSE", "TORTURER_GAIN_CURSE")
                                }
                                (Some(Card::Mill), Opt::Yes) => ("MILL", "CONTINUE"),
                                (Some(Card::Mill), Opt::No) => ("MILL", "DONT_DISCARD"),
                                _ => return Err(semantics::gap(d, "alternative binding")),
                            };
                            children.push(choice(
                                description(c, "BUTTON", story, association)?,
                                QuestionElements::Cards(vec![]),
                                0,
                                0,
                                c.symbol("DeclineButtonIds", button)?,
                            )?);
                            plans.push(AnswerPlan::Constant(Response::option(o)));
                        }
                        let mut desc = desc;
                        desc.question_type = c.symbol("QuestionTypes", "COMPLEX_OR")?;
                        (
                            Question {
                                index: 0,
                                description: desc,
                                kind: QuestionKind::Complex(children),
                            },
                            AnswerPlan::Or(plans),
                        )
                    } else {
                        return Err(semantics::gap(d, "unmapped option shape"));
                    }
                }
                _ => return Err(semantics::gap(d, "unmapped decision shape")),
            }
        }
    };
    wire.index = question_index;
    Ok(PreparedQuestion {
        wire,
        decision_id: decision_id.into(),
        epoch: 0,
        plan,
    })
}

/// A `Choose` a set's module says how to show (`semantics::Choice`).
#[allow(clippy::too_many_arguments)]
fn set_choice(
    c: &WireCatalog,
    d: &Decision,
    desc: QuestionDescription,
    association: i32,
    shown: semantics::Choice,
    options: &[Opt],
    min: usize,
    max: usize,
    id: &dyn Fn(&CardId) -> Result<i32, ProjectionError>,
) -> Result<(Question, AnswerPlan), ProjectionError> {
    use semantics::Choice;
    Ok(match shown {
        Choice::Modes(modes) => {
            if modes.len() != options.len() {
                return Err(ProjectionError::Inconsistent("one mode per option"));
            }
            let modes = modes
                .iter()
                .map(|m| c.symbol("CardModes", m))
                .collect::<Result<Vec<_>, _>>()?;
            (
                choice(desc, QuestionElements::Modes(modes), min, max, -1)?,
                AnswerPlan::Options {
                    targets: options.to_vec(),
                    min,
                    max,
                    decline: None,
                },
            )
        }
        Choice::Card(card) => {
            if options != [Opt::Yes, Opt::No] {
                return Err(semantics::gap(d, "card choice without yes and no"));
            }
            (
                choice(
                    desc,
                    QuestionElements::Cards(vec![id(&card)?]),
                    0,
                    1,
                    decline_for(c, d, 0)?,
                )?,
                AnswerPlan::Options {
                    targets: vec![Opt::Yes],
                    min: 1,
                    max: 1,
                    decline: Some(Opt::No),
                },
            )
        }
        Choice::Buttons(buttons) => {
            if buttons.len() != options.len() {
                return Err(ProjectionError::Inconsistent("one button per option"));
            }
            let mut children = vec![];
            let mut plans = vec![];
            for (&o, button) in options.iter().zip(buttons) {
                let mut child = desc.clone();
                child.question_type = c.symbol("QuestionTypes", "BUTTON")?;
                child.association = association;
                children.push(choice(
                    child,
                    QuestionElements::Cards(vec![]),
                    0,
                    0,
                    c.symbol("DeclineButtonIds", button)?,
                )?);
                plans.push(AnswerPlan::Constant(Response::option(o)));
            }
            let mut desc = desc;
            desc.question_type = c.symbol("QuestionTypes", "COMPLEX_OR")?;
            (
                Question {
                    index: 0,
                    description: desc,
                    kind: QuestionKind::Complex(children),
                },
                AnswerPlan::Or(plans),
            )
        }
    })
}

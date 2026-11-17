//! A game's `dominion/events@1` stream, written from the engine's journal.

use dominion_engine::{
    Card, CardId, EndReason, Game, Gauge, Kind, Mat, Phase, PileId, PileKind, PileProps, PlayerId,
    SetupRole, Spot, Step, Token, TokenAt, TokenKind, Trace, Verb, Zone,
};
use dominion_protocol::events::{
    self, At, CardRef, Choice, CounterChange, CounterKey, CounterName, CounterOf, Event, FoldError,
    GameEnd, Move, Pack, PileChange, PileRole, PileState, Play, Recipe, RecipeGroup, RecipeKind,
    Role, Rules, SeatResult, SeatZone, Setup, Table, TurnStart, What,
};

/// Writes the full stream of a game started with [`Game::new_journaled`]: every card's
/// face, every shuffle's order.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    /// The table after the events written so far.
    table: Table,
    /// What each card is called now.
    names: Vec<String>,
    /// Each pile's zone, once it has been seen.
    piles: Vec<Option<events::Zone>>,
    seq: u64,
    set_up: bool,
    ended: bool,
    /// Whose turn it is: who moves a card that goes between zones nobody owns.
    turn: u8,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// The table after everything recorded so far.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// Takes what `game` has journaled since the last call and returns it as events.
    /// `seats` names the players. An error means the journal doesn't describe one game:
    /// a bug in the engine or here.
    pub fn record(&mut self, game: &mut Game, seats: &[String]) -> Result<Vec<Event>, FoldError> {
        let mut steps = game.take_journal().into_iter().peekable();
        let mut out: Vec<Event> = vec![];
        if !self.set_up {
            self.set_up = true;
            let mut created = vec![];
            while let Some(Step::Trace {
                entry: Trace::Created { .. },
                ..
            }) = steps.peek()
            {
                if let Some(Step::Trace {
                    entry: Trace::Created { card, is, zone },
                    ..
                }) = steps.next()
                {
                    created.push((card, is, zone));
                }
            }
            // What setup made besides cards: Boons and Hexes, counters, the rest of the table.
            let mut made = vec![];
            while let Some(
                Step::FateAdded { .. }
                | Step::Gauge { .. }
                | Step::Token { .. }
                | Step::Holding { .. }
                | Step::Pile { .. }
                | Step::Landscape { .. }
                | Step::Notes(_)
                | Step::Derived { .. },
            ) = steps.peek()
            {
                let step = steps.next().expect("peeked");
                if let Some(what) = self.state_change(game, step) {
                    made.push(what);
                }
            }
            for what in self.set_up(game, seats, created, made) {
                self.push(&mut out, what)?;
            }
        }
        while let Some(step) = steps.next() {
            let what = match step {
                Step::Trace {
                    entry: Trace::Created { card, is, zone },
                    ..
                } => {
                    self.name(card, is);
                    What::Add {
                        cards: vec![self.card(card)],
                        to: self.zone(game, zone),
                    }
                }
                Step::Trace {
                    entry: Trace::Moved { from, to, .. },
                    verb: Some(Verb::Shuffle),
                } => {
                    // Its cards are journaled bottom first.
                    let mut cards = vec![];
                    let mut step = Some(step);
                    while let Some(Step::Trace {
                        entry: Trace::Moved { card, .. },
                        verb: Some(Verb::Shuffle),
                    }) = step
                    {
                        cards.push(self.card(card));
                        step = steps.next_if(|next| {
                            matches!(next, Step::Trace { entry: Trace::Moved { from: f, to: t, .. }, verb: Some(Verb::Shuffle) } if *f == from && *t == to)
                        });
                    }
                    cards.reverse();
                    let (from, to) = (self.zone(game, from), self.zone(game, to));
                    let at = if from == to { At::Top } else { At::Bottom };
                    let pos = self.depths(&from, &cards);
                    What::Shuffle(Move {
                        player: self.mover(&from, &to),
                        cards,
                        from,
                        to,
                        at,
                        pos,
                    })
                }
                Step::Trace {
                    entry:
                        Trace::Moved {
                            card,
                            from,
                            to,
                            bottom,
                            under,
                            ..
                        },
                    verb,
                } => {
                    let (from, to) = (self.zone(game, from), self.zone(game, to));
                    let cards = vec![self.card(card)];
                    // The engine puts a card that should go under more cards than there
                    // are at the bottom.
                    let there = self.table.zone(&to).len() - usize::from(from == to);
                    let at = match (to.ordered(), bottom, under.min(there)) {
                        (false, ..) | (true, false, 0) => At::Top,
                        (true, true, _) => At::Bottom,
                        (true, false, n) => At::Under(n as u32),
                    };
                    let pos = self.depths(&from, &cards);
                    let played = (verb == Some(Verb::Play))
                        .then(|| {
                            steps.next_if(
                                |next| matches!(next, Step::Played { card: c, .. } if *c == card),
                            )
                        })
                        .flatten();
                    if let Some(Step::Played { player, .. }) = played {
                        let what = What::Play(Play {
                            player: player.0,
                            card: self.card(card),
                            from: Some(from),
                            to: Some(to),
                            pos: pos.first().copied(),
                        });
                        self.push(&mut out, what)?;
                        continue;
                    }
                    What::moved(
                        verb_of(verb.unwrap_or(Verb::Put)),
                        Move {
                            player: self.mover(&from, &to),
                            cards,
                            from,
                            to,
                            at,
                            pos,
                        },
                    )
                }
                Step::Trace {
                    entry: Trace::Changed { card, to, .. },
                    ..
                } => {
                    let was = self.card(card);
                    self.name(card, to);
                    What::Rename {
                        card: was,
                        name: to.name().to_string(),
                    }
                }
                Step::Turn {
                    player,
                    number,
                    extra,
                } => {
                    self.turn = player.0;
                    What::Turn(TurnStart {
                        player: player.0,
                        number,
                        extra: extra.map(|c| c.name().to_string()),
                    })
                }
                Step::Phase(phase) => What::Phase {
                    phase: phase_of(phase),
                },
                step @ (Step::Gauge { .. }
                | Step::Fate { .. }
                | Step::FateShuffled { .. }
                | Step::FateAdded { .. }
                | Step::Token { .. }
                | Step::Holding { .. }
                | Step::Pile { .. }
                | Step::Landscape { .. }
                | Step::Notes(_)
                | Step::Derived { .. }) => match self.state_change(game, step) {
                    Some(what) => what,
                    None => continue,
                },
                // Played where it lies: again, or from where it isn't put into play.
                Step::Played { player, card } => What::Play(Play {
                    player: player.0,
                    card: self.card(card),
                    from: None,
                    to: None,
                    pos: None,
                }),
                Step::Logged { index } => match occurrence(&game.log()[index]) {
                    Some(what) => what,
                    None => continue,
                },
                // Who saw what is not in the full stream.
                Step::Trace { .. } => continue,
            };
            self.push(&mut out, what)?;
        }
        if let (false, Some(outcome)) = (self.ended, game.outcome()) {
            self.ended = true;
            let result = (0..game.num_players())
                .map(|i| SeatResult {
                    seat: i as u8,
                    vp: outcome.scores[i],
                    turns: outcome.turns[i],
                    rank: 1
                        + (0..game.num_players())
                            .filter(|&j| ahead(outcome, j, i))
                            .count() as u32,
                    resigned: outcome.resigned.contains(&PlayerId(i as u8)),
                })
                .collect();
            let reason = match outcome.reason {
                EndReason::ProvincesGone => "provinces",
                EndReason::ColoniesGone => "colonies",
                EndReason::PilesEmpty => "piles",
                EndReason::TurnLimit => "turn-limit",
                EndReason::Resigned => "resigned",
            };
            self.push(
                &mut out,
                What::GameEnd(GameEnd {
                    reason: reason.into(),
                    result,
                }),
            )?;
        }
        Ok(out)
    }

    /// The setup event, then each player's opening shuffle and draw: the engine deals
    /// the opening hands before it journals anything.
    fn set_up(
        &mut self,
        game: &Game,
        seats: &[String],
        created: Vec<(CardId, Card, Zone)>,
        made: Vec<What>,
    ) -> Vec<What> {
        let mut table = Table::default();
        for i in 0..game.piles().len() {
            let zone = self.zone(game, Zone::Pile(PileId(i as u8)));
            table.zones.insert(zone, vec![]);
        }
        for what in made {
            let event = Event {
                seq: 0,
                by: None,
                what,
            };
            table.apply(&event).expect("setup's own state folds");
        }
        let n = game.num_players();
        // Per seat: the deck as shuffled, top first, and how many of it were drawn.
        let mut opening: Vec<(Vec<CardRef>, usize)> = vec![(vec![], 0); n];
        for (id, is, zone) in created {
            self.name(id, is);
            let card = self.card(id);
            match zone {
                // Journaled bottom first; the hand in the order drawn.
                Zone::Deck(p) => opening[p.index()].0.insert(0, card),
                Zone::Hand(p) => {
                    let (deck, drawn) = &mut opening[p.index()];
                    deck.insert(*drawn, card);
                    *drawn += 1;
                }
                zone => {
                    let zone = self.zone(game, zone);
                    let cards = table.zones.entry(zone.clone()).or_default();
                    cards.insert(if zone.ordered() { 0 } else { cards.len() }, card);
                }
            }
        }
        let mut after = vec![];
        for (seat, (deck, drawn)) in opening.into_iter().enumerate() {
            let seat = seat as u8;
            let zone = events::Zone::Seat(seat, SeatZone::Deck);
            // Before the shuffle the deck is in no order anyone chose: by handle.
            let mut unshuffled = deck.clone();
            unshuffled.sort_by_key(CardRef::handle);
            let pos = sequential_depths(&unshuffled, &deck);
            table.zones.insert(zone.clone(), unshuffled);
            let hand = deck[..drawn].to_vec();
            after.push(What::Shuffle(Move {
                player: seat,
                cards: deck,
                from: zone.clone(),
                to: zone.clone(),
                at: At::Top,
                pos,
            }));
            if !hand.is_empty() {
                after.push(What::Draw(Move {
                    player: seat,
                    cards: hand,
                    from: zone,
                    to: events::Zone::Seat(seat, SeatZone::Hand),
                    at: At::Top,
                    pos: vec![],
                }));
            }
        }
        let names = |cards: &[Card]| cards.iter().map(|c| c.name().to_string()).collect();
        let setup = Setup {
            rules: Rules {
                implementation: "dominion-engine".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            seats: (0..n)
                .map(|i| (seats.get(i).cloned()).unwrap_or_else(|| PlayerId(i as u8).to_string()))
                .collect(),
            kingdom: names(&game.config().kingdom),
            landscapes: names(game.landscapes()),
            table,
        };
        let mut out = vec![What::Setup(Box::new(setup))];
        out.extend(after);
        out
    }

    fn push(&mut self, out: &mut Vec<Event>, what: What) -> Result<(), FoldError> {
        let event = Event {
            seq: self.seq,
            by: None,
            what,
        };
        self.table.apply(&event)?;
        // Cards moving the same way one after the other are one event.
        if let (Some(last), Some((verb, m))) = (out.last_mut(), event.what.as_move())
            && let Some((v, _)) = last.what.as_move()
            && v == verb
            && verb != events::Verb::Shuffle
            && let Some(l) = last.what.as_move_mut()
            && (l.player, &l.from, &l.to, l.at) == (m.player, &m.from, &m.to, m.at)
        {
            if !(l.pos.is_empty() && m.pos.is_empty()) {
                l.pos.resize(l.cards.len(), 0);
                l.pos.extend(if m.pos.is_empty() {
                    vec![0; m.cards.len()]
                } else {
                    m.pos.clone()
                });
            }
            l.cards.extend(m.cards.iter().cloned());
            return Ok(());
        }
        self.seq += 1;
        out.push(event);
        Ok(())
    }

    fn name(&mut self, id: CardId, is: Card) {
        if self.names.len() <= id.index() {
            self.names.resize(id.index() + 1, String::new());
        }
        self.names[id.index()] = is.name().to_string();
    }

    fn card(&self, id: CardId) -> CardRef {
        CardRef::seen(self.names[id.index()].clone(), id.0)
    }

    /// How many cards lie over each of `cards` in `zone` as they leave it one after the
    /// other; nothing when each comes off the top.
    fn depths(&self, zone: &events::Zone, cards: &[CardRef]) -> Vec<u32> {
        if !zone.ordered() {
            return vec![];
        }
        sequential_depths(self.table.zone(zone), cards)
    }

    /// Whose action a move between `from` and `to` is: the owner of `from`, else of `to`,
    /// else the turn's player.
    fn mover(&self, from: &events::Zone, to: &events::Zone) -> u8 {
        match (from, to) {
            (events::Zone::Seat(p, _), _) | (_, events::Zone::Seat(p, _)) => *p,
            _ => self.turn,
        }
    }

    fn zone(&mut self, game: &Game, zone: Zone) -> events::Zone {
        let seat = |p: PlayerId, z| events::Zone::Seat(p.0, z);
        match zone {
            Zone::Deck(p) => seat(p, SeatZone::Deck),
            Zone::Hand(p) => seat(p, SeatZone::Hand),
            Zone::Discard(p) => seat(p, SeatZone::Discard),
            Zone::InPlay(p) => seat(p, SeatZone::Play),
            Zone::SetAside(p) => seat(p, SeatZone::Aside),
            Zone::Looking(p) => seat(p, SeatZone::Looking),
            Zone::Mat(p, Mat::Tavern) => seat(p, SeatZone::Tavern),
            Zone::Mat(p, Mat::Island) => seat(p, SeatZone::Island),
            Zone::Mat(p, Mat::NativeVillage) => seat(p, SeatZone::NativeVillage),
            Zone::Mat(p, Mat::Exile) => seat(p, SeatZone::Exile),
            Zone::Trash => events::Zone::Trash,
            Zone::Pile(id) => self.pile(game, id),
        }
    }

    /// A pile's zone: named after its card, and told apart from an earlier pile of the
    /// same name (one Divine Wind removed) by a number.
    fn pile(&mut self, game: &Game, id: PileId) -> events::Zone {
        if self.piles.len() <= id.index() {
            self.piles.resize(id.index() + 1, None);
        }
        if let Some(zone) = &self.piles[id.index()] {
            return zone.clone();
        }
        let pile = game.pile(id);
        let named = |name: String| match pile.holder {
            true => events::Zone::Aside(name),
            false => events::Zone::Pile(name),
        };
        let base = pile.card.name().to_string();
        let taken = |zone: &events::Zone| self.piles.iter().flatten().any(|z| z == zone);
        let zone = (1..)
            .map(|n| match n {
                1 => named(base.clone()),
                n => named(format!("{base} ({n})")),
            })
            .find(|zone| !taken(zone))
            .expect("some number is free");
        self.piles[id.index()] = Some(zone.clone());
        zone
    }

    fn counter(&mut self, game: &Game, gauge: Gauge) -> CounterKey {
        use CounterName as N;
        let seat = |p: PlayerId, name| CounterKey::seat(p.0, name);
        let mut pile = |id, name| CounterKey::new(CounterOf::Pile(self.pile(game, id)), name);
        match gauge {
            Gauge::Actions => CounterKey::global(N::Actions),
            Gauge::Buys => CounterKey::global(N::Buys),
            Gauge::Coins => CounterKey::global(N::Coins),
            Gauge::Potions => CounterKey::global(N::Potions),
            Gauge::TradeRouteMat => CounterKey::global(N::TradeRouteMat),
            Gauge::Sun => CounterKey::global(N::Sun),
            Gauge::Coffers(p) => seat(p, N::Coffers),
            Gauge::Villagers(p) => seat(p, N::Villagers),
            Gauge::Favors(p) => seat(p, N::Favors),
            Gauge::Debt(p) => seat(p, N::Debt),
            Gauge::VpTokens(p) => seat(p, N::VpTokens),
            Gauge::PirateShip(p) => seat(p, N::PirateShip),
            Gauge::SinisterPlot(p) => seat(p, N::SinisterPlot),
            Gauge::PileVp(id) => pile(id, N::Vp),
            Gauge::PileDebt(id) => pile(id, N::Debt),
            Gauge::Embargo(id) => pile(id, N::Embargo),
            Gauge::TradeRoute(id) => pile(id, N::TradeRoute),
            Gauge::PileFavors(id) => pile(id, N::Favors),
            Gauge::LandscapeVp(card) => {
                CounterKey::new(CounterOf::Landscape(card.name().to_string()), N::Vp)
            }
        }
    }

    /// The event a step that changes the table besides cards says, if any.
    fn state_change(&mut self, game: &Game, step: Step) -> Option<What> {
        let name = |c: Card| c.name().to_string();
        Some(match step {
            Step::Gauge { gauge, was, now } => What::Counter(CounterChange {
                counter: self.counter(game, gauge),
                delta: now - was,
                value: now,
            }),
            Step::Fate { card, to } => {
                let card = CardRef::Named(name(card));
                let to = fate_zone(&card, to);
                // Not found, the fold refuses the move: the journal is not one game's.
                let from = (self.table.zones.iter())
                    .find(|(z, cards)| is_fate_zone(z) && cards.contains(&card))
                    .map_or_else(|| fate_zone(&card, Spot::Deck), |(z, _)| z.clone());
                let verb = match to {
                    events::Zone::Boons(Pack::Discard) | events::Zone::Hexes(Pack::Discard) => {
                        events::Verb::Discard
                    }
                    events::Zone::Boons(Pack::Druid) => events::Verb::SetAside,
                    _ => events::Verb::Take,
                };
                let cards = vec![card];
                let pos = self.depths(&from, &cards);
                What::moved(
                    verb,
                    Move {
                        player: self.mover(&from, &to),
                        cards,
                        from,
                        to,
                        at: At::Top,
                        pos,
                    },
                )
            }
            Step::FateShuffled { hexes, cards } => {
                let zone = |pack| match hexes {
                    true => events::Zone::Hexes(pack),
                    false => events::Zone::Boons(pack),
                };
                let cards: Vec<CardRef> =
                    cards.into_iter().map(|c| CardRef::Named(name(c))).collect();
                let from = zone(Pack::Discard);
                let pos = self.depths(&from, &cards);
                What::Shuffle(Move {
                    player: self.turn,
                    cards,
                    from,
                    to: zone(Pack::Deck),
                    at: At::Top,
                    pos,
                })
            }
            Step::FateAdded { cards, to } => {
                let cards: Vec<CardRef> =
                    cards.into_iter().map(|c| CardRef::Named(name(c))).collect();
                let to = fate_zone(cards.first()?, to);
                What::Add { cards, to }
            }
            Step::Token { player, token, at } => What::Token {
                player: player.0,
                token: token_kind(token),
                at: at.map(|at| match at {
                    TokenAt::Pile(id) => events::TokenAt::Pile(self.pile(game, id)),
                    TokenAt::Player => events::TokenAt::Player,
                    TokenAt::FaceDown => events::TokenAt::FaceDown,
                }),
            },
            Step::Holding { card, from, to } => What::Holding {
                card: name(card),
                from: from.map(|p| p.0),
                to: to.map(|p| p.0),
            },
            Step::Pile { pile, props } => {
                let zone = self.pile(game, pile);
                What::Pile(PileChange {
                    pile: zone,
                    state: pile_state(game.pile(pile).card, props),
                })
            }
            Step::Landscape { card, prophecy } => What::Landscape {
                card: name(card),
                prophecy,
            },
            Step::Notes(notes) => What::Notes {
                notes: (notes.into_iter())
                    .map(|n| events::Note {
                        source: name(n.source),
                        player: n.player.map(|p| p.0),
                        pile: n.pile.map(|id| self.pile(game, id)),
                        card: n.card.map(name),
                        count: n.count,
                    })
                    .collect(),
            },
            Step::Derived { costs, types } => What::Derived {
                costs: (costs.into_iter())
                    .map(|(c, cost)| (name(c), cost_of(cost)))
                    .collect(),
                types: (types.into_iter())
                    .map(|(c, t)| (name(c), t.map(type_names)))
                    .collect(),
            },
            _ => return None,
        })
    }
}

/// What a log entry says that no card move or counter does.
fn occurrence(event: &dominion_engine::Event) -> Option<What> {
    use dominion_engine::Event as E;
    let name = |c: &Card| c.name().to_string();
    Some(match event {
        E::Received { player, card } => What::Receive {
            player: player.0,
            card: name(card),
        },
        E::Bought { player, card } | E::BoughtLandscape { player, card } => What::Buy {
            player: player.0,
            card: name(card),
        },
        E::Resolved { player, card } | E::ResolvedFor { player, card } => What::Resolve {
            player: player.0,
            card: name(card),
        },
        E::Named { player, card } => What::Name {
            player: player.0,
            card: name(card),
        },
        E::UsedWay { player, card, way } => What::Way {
            player: player.0,
            card: name(card),
            way: name(way),
        },
        E::Unaffected { player, attack, by } => What::Unaffected {
            player: player.0,
            attack: name(attack),
            card: name(by),
        },
        E::Called { player, card } => What::Call {
            player: player.0,
            card: name(card),
        },
        E::Chosen {
            player,
            source,
            choices,
        } => What::Choose(Choice {
            player: player.0,
            source: source.as_ref().map(name),
            choices: choices.clone(),
        }),
        E::TurnSkipped { player, extra } => What::TurnSkipped {
            player: player.0,
            extra: extra.as_ref().map(name),
            lost: false,
        },
        E::ExtraTurnLost { player, source } => What::TurnSkipped {
            player: player.0,
            extra: Some(name(source)),
            lost: true,
        },
        _ => return None,
    })
}

/// Whether seat `a` finished ahead of seat `b`.
fn ahead(outcome: &dominion_engine::Outcome, a: usize, b: usize) -> bool {
    let key = |i: usize| {
        (
            !outcome.resigned.contains(&PlayerId(i as u8)),
            outcome.scores[i],
            std::cmp::Reverse(outcome.turns[i]),
        )
    };
    key(a) > key(b)
}

/// How many cards lie over each of `cards` in `zone` (top first) as they leave it one
/// after the other; nothing when each comes off the top.
fn sequential_depths(zone: &[CardRef], cards: &[CardRef]) -> Vec<u32> {
    let mut left = zone.to_vec();
    let depths: Vec<u32> = cards
        .iter()
        .map(|card| {
            let i = left.iter().position(|c| c == card).unwrap_or(0);
            if i < left.len() {
                left.remove(i);
            }
            i as u32
        })
        .collect();
    if depths.iter().all(|&d| d == 0) {
        vec![]
    } else {
        depths
    }
}

fn verb_of(verb: Verb) -> events::Verb {
    match verb {
        // Without a play: nothing the engine does.
        Verb::Play => events::Verb::Put,
        Verb::Draw => events::Verb::Draw,
        Verb::Discard => events::Verb::Discard,
        Verb::Gain => events::Verb::Gain,
        Verb::Trash => events::Verb::Trash,
        Verb::Topdeck => events::Verb::Topdeck,
        Verb::Bottomdeck => events::Verb::Bottomdeck,
        Verb::Insert => events::Verb::Insert,
        Verb::Reveal => events::Verb::Reveal,
        Verb::Look => events::Verb::Look,
        Verb::SetAside => events::Verb::SetAside,
        Verb::Exile => events::Verb::Exile,
        Verb::Put => events::Verb::Put,
        Verb::Return => events::Verb::Return,
        Verb::Exchange => events::Verb::Exchange,
        Verb::Shuffle => events::Verb::Shuffle,
        Verb::Rotate => events::Verb::Rotate,
    }
}

fn phase_of(phase: Phase) -> events::Phase {
    match phase {
        Phase::Start => events::Phase::Start,
        Phase::Action => events::Phase::Action,
        Phase::Buy => events::Phase::Buy,
        Phase::Night => events::Phase::Night,
        Phase::Cleanup => events::Phase::Cleanup,
        Phase::End => events::Phase::End,
    }
}

fn is_fate_zone(zone: &events::Zone) -> bool {
    matches!(
        zone,
        events::Zone::Boons(_)
            | events::Zone::Hexes(_)
            | events::Zone::Seat(_, SeatZone::Boons | SeatZone::Hexes)
    )
}

/// The zone a Boon or Hex at `spot` is in.
fn fate_zone(card: &CardRef, spot: Spot) -> events::Zone {
    let hex = card
        .name()
        .and_then(|n| Card::ALL.iter().find(|c| c.name() == n))
        .is_some_and(|c| c.kind() == Kind::Hex);
    match (spot, hex) {
        (Spot::Deck, false) => events::Zone::Boons(Pack::Deck),
        (Spot::Deck, true) => events::Zone::Hexes(Pack::Deck),
        (Spot::Discard, false) => events::Zone::Boons(Pack::Discard),
        (Spot::Discard, true) => events::Zone::Hexes(Pack::Discard),
        (Spot::Druid, _) => events::Zone::Boons(Pack::Druid),
        (Spot::Player(p), false) => events::Zone::Seat(p.0, SeatZone::Boons),
        (Spot::Player(p), true) => events::Zone::Seat(p.0, SeatZone::Hexes),
    }
}

fn token_kind(token: TokenKind) -> events::TokenKind {
    use events::TokenKind as T;
    match token {
        TokenKind::Adventures(Token::PlusCard) => T::PlusCard,
        TokenKind::Adventures(Token::PlusAction) => T::PlusAction,
        TokenKind::Adventures(Token::PlusBuy) => T::PlusBuy,
        TokenKind::Adventures(Token::PlusCoin) => T::PlusCoin,
        TokenKind::Adventures(Token::MinusCost) => T::MinusCost,
        TokenKind::Adventures(Token::Trashing) => T::Trashing,
        TokenKind::Adventures(Token::Estate) => T::Estate,
        TokenKind::MinusCard => T::MinusCard,
        TokenKind::MinusCoin => T::MinusCoin,
        TokenKind::Journey => T::Journey,
    }
}

fn pile_state(card: Card, props: PileProps) -> PileState {
    let name = |c: Card| c.name().to_string();
    PileState {
        card: name(card),
        supply: props.in_supply,
        hidden: props.hidden,
        removed: props.removed,
        roles: (props.roles.into_iter())
            .map(|(source, role)| PileRole {
                card: name(source),
                role: match role {
                    SetupRole::SetAside => Role::SetAside,
                    SetupRole::Bane => Role::Bane,
                    SetupRole::ExtraPile => Role::ExtraPile,
                    SetupRole::ChosenPile => Role::ChosenPile,
                    SetupRole::Deck => Role::Deck,
                },
            })
            .collect(),
        traits: props.traits.into_iter().map(name).collect(),
        recipe: props.structure.map(|s| Recipe {
            kind: match s.kind {
                PileKind::Stacked => RecipeKind::Stacked,
                PileKind::Rotating => RecipeKind::Rotating,
                PileKind::Shuffled => RecipeKind::Shuffled,
            },
            cards: (s.cards.into_iter())
                .map(|g| RecipeGroup {
                    card: name(g.card),
                    count: g.count,
                })
                .collect(),
            sample: s.sample_size,
        }),
    }
}

fn cost_of(cost: dominion_engine::Cost) -> events::Cost {
    events::Cost {
        coins: u32::from(cost.coins),
        potions: u32::from(cost.potions),
        debt: u32::from(cost.debt),
    }
}

fn type_names(types: dominion_engine::Types) -> Vec<String> {
    types.names().map(str::to_string).collect()
}

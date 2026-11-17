//! A game's `dominion/events@1` stream, written from the engine's journal.

use dominion_engine::{
    Card, CardId, EffectEnd, EffectState, EndReason, ExtraLimit, Game, Gauge, Mat, Phase, PileId,
    PlayerId, Step, Trace, TurnKind, Until, Verb, Zone,
};
use dominion_protocol::events::{
    self, At, CardRef, Choice, CounterChange, Effect, Ending, Event, ExtraTurnQueued, Face,
    FoldError, GameEnd, Move, Play, Rules, SeatResult, SeatZone, Setup, Table, TurnLimits,
    TurnStart, What,
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
            let mut counters = vec![];
            while let Some(&Step::Gauge { gauge, now, .. }) = steps.peek() {
                steps.next();
                counters.push((self.counter(game, gauge), now));
            }
            for what in self.set_up(game, seats, created, counters) {
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
                        face: None,
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
                    let face = matches!(
                        to,
                        events::Zone::Seat(_, SeatZone::Aside | SeatZone::Looking)
                    )
                    .then(|| {
                        match steps.next_if(|next| matches!(next, Step::FaceUp(c) if *c == card)) {
                            Some(_) => Face::Up,
                            None => Face::Down,
                        }
                    });
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
                            face,
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
                    kind,
                    limits,
                } => {
                    self.turn = player.0;
                    let (kind, extra, extra_turn) = turn_kind(kind);
                    What::Turn(TurnStart {
                        player: player.0,
                        number,
                        extra,
                        kind,
                        extra_turn,
                        limits: TurnLimits {
                            no_card_buys: limits.no_card_buys,
                            hand_plays: limits.hand_plays,
                        },
                    })
                }
                Step::TurnSkipped { player, kind, lost } => {
                    let (kind, extra, extra_turn) = turn_kind(kind);
                    What::TurnSkipped {
                        player: player.0,
                        extra,
                        lost,
                        kind,
                        extra_turn,
                    }
                }
                Step::TurnEnded { player } => What::TurnEnd { player: player.0 },
                Step::EffectBegan(e) => What::Effect(self.effect(e)),
                Step::EffectChanged(e) => What::EffectChange(self.effect(e)),
                Step::EffectEnded { id, why } => What::EffectEnd {
                    id,
                    why: match why {
                        EffectEnd::Resolved => events::EffectEnd::Resolved,
                        EffectEnd::Expired => events::EffectEnd::Expired,
                        EffectEnd::LeftPlay => events::EffectEnd::LeftPlay,
                        EffectEnd::ExtraTurn => events::EffectEnd::ExtraTurn,
                    },
                },
                Step::ExtraTurnQueued { turn, front } => What::ExtraTurn(ExtraTurnQueued {
                    id: turn.id,
                    player: turn.player.0,
                    card: turn.source.name().to_string(),
                    limit: match turn.limit {
                        ExtraLimit::None => events::ExtraLimit::None,
                        ExtraLimit::ThirdInARow => events::ExtraLimit::ThirdInARow,
                    },
                    effect: turn.lasting,
                    front,
                }),
                Step::ExtraTurnRemoved { id } => What::ExtraTurnRemoved { id },
                Step::GameEnding {
                    reason,
                    final_round,
                } => What::GameEnding(Ending {
                    reason: reason_of(reason).into(),
                    final_round: final_round.iter().map(|p| p.0).collect(),
                }),
                // Read with the move it follows; alone, the move was no placement.
                Step::FaceUp(_) => continue,
                Step::Phase(phase) => What::Phase {
                    phase: phase_of(phase),
                },
                Step::Gauge { gauge, was, now } => What::Counter(CounterChange {
                    counter: self.counter(game, gauge),
                    delta: now - was,
                    value: now,
                }),
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
            let reason = reason_of(outcome.reason);
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
        counters: Vec<(String, i64)>,
    ) -> Vec<What> {
        let mut table = Table {
            counters: counters.into_iter().collect(),
            ..Table::default()
        };
        for i in 0..game.piles().len() {
            let zone = self.zone(game, Zone::Pile(PileId(i as u8)));
            table.zones.insert(zone, vec![]);
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
                face: None,
            }));
            if !hand.is_empty() {
                after.push(What::Draw(Move {
                    player: seat,
                    cards: hand,
                    from: zone,
                    to: events::Zone::Seat(seat, SeatZone::Hand),
                    at: At::Top,
                    pos: vec![],
                    face: None,
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
            && (l.player, &l.from, &l.to, l.at, l.face) == (m.player, &m.from, &m.to, m.at, m.face)
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

    fn counter(&mut self, game: &Game, gauge: Gauge) -> String {
        let of_pile = |r: &mut Recorder, id, name: &str| format!("{}/{name}", r.pile(game, id));
        match gauge {
            Gauge::Actions => "actions".into(),
            Gauge::Buys => "buys".into(),
            Gauge::Coins => "coins".into(),
            Gauge::Potions => "potions".into(),
            Gauge::TradeRouteMat => "trade-route-mat".into(),
            Gauge::Sun => "sun".into(),
            Gauge::Coffers(p) => format!("{}/coffers", p.0),
            Gauge::Villagers(p) => format!("{}/villagers", p.0),
            Gauge::Favors(p) => format!("{}/favors", p.0),
            Gauge::Debt(p) => format!("{}/debt", p.0),
            Gauge::VpTokens(p) => format!("{}/vp-tokens", p.0),
            Gauge::PirateShip(p) => format!("{}/pirate-ship", p.0),
            Gauge::PileVp(id) => of_pile(self, id, "vp"),
            Gauge::PileDebt(id) => of_pile(self, id, "debt"),
            Gauge::Embargo(id) => of_pile(self, id, "embargo"),
            Gauge::TradeRoute(id) => of_pile(self, id, "trade-route"),
            Gauge::NextHand => "next-hand".into(),
            Gauge::Bought => "bought".into(),
            Gauge::HandPlays => "hand-plays".into(),
            Gauge::SkipTurns(p) => format!("{}/skip-turns", p.0),
            Gauge::TurnsSkipped(p) => format!("{}/turns-skipped", p.0),
        }
    }

    fn effect(&self, e: EffectState) -> Effect {
        let cards = |ids: &[CardId]| ids.iter().map(|&c| self.card(c)).collect();
        Effect {
            id: e.id,
            card: e.card.name().to_string(),
            source: e.source.map(|c| self.card(c)),
            owner: e.owner.0,
            until: match e.until {
                Until::EndOfTurn => events::Until::EndOfTurn,
                Until::StartOfTurn(p) => events::Until::StartOfTurn(p.0),
                Until::ExtraTurn => events::Until::ExtraTurn,
                Until::Removed => events::Until::Removed,
            },
            ending: e.ending,
            keeps: cards(&e.keeps),
            holds: cards(&e.holds),
            tokens: e.tokens,
        }
    }
}

/// What a log entry says that no card move or counter does.
fn occurrence(event: &dominion_engine::Event) -> Option<What> {
    use dominion_engine::Event as E;
    let name = |c: &Card| c.name().to_string();
    Some(match event {
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
        _ => return None,
    })
}

fn reason_of(reason: EndReason) -> &'static str {
    match reason {
        EndReason::ProvincesGone => "provinces",
        EndReason::ColoniesGone => "colonies",
        EndReason::PilesEmpty => "piles",
        EndReason::TurnLimit => "turn-limit",
        EndReason::Resigned => "resigned",
    }
}

/// A turn's kind, the card that gave it and the extra turn waiting it was.
fn turn_kind(kind: TurnKind) -> (events::TurnKind, Option<String>, Option<u32>) {
    match kind {
        TurnKind::Regular => (events::TurnKind::Regular, None, None),
        TurnKind::Extra { source, id } => (
            events::TurnKind::Extra,
            Some(source.name().to_string()),
            Some(id),
        ),
        TurnKind::Final => (events::TurnKind::Final, None, None),
    }
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

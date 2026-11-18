//! A game's `dominion/events@1` stream, written from the engine's journal.

use dominion_engine::{
    Card, CardId, EffectEnd, EffectState, EndReason, ExtraLimit, Game, Gauge, Kind, Mat, Occasion,
    Phase, PileId, PileKind, PileProps, PlayerId, SetupRole, Spot, Step, Token, TokenAt, TokenKind,
    Trace, TurnKind, Until, Verb, Zone,
};
use dominion_protocol::events::{
    self, At, CardRef, Choice, CounterChange, CounterKey, CounterName, CounterOf, Effect, Ending,
    Event, ExtraTurnQueued, Face, FoldError, GameEnd, Moment, Move, Pack, PileChange, PileRole,
    PileState, Play, Recipe, RecipeGroup, RecipeKind, Role, Rules, SeatResult, SeatZone, Setup,
    Table, TurnLimits, TurnStart, What,
};
use std::collections::{BTreeMap, BTreeSet};

/// Writes the full stream of a game started with [`Game::new_journaled`]: every card's
/// face, every shuffle's order. Saved with the game, it goes on where it left off.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Recorder {
    /// The table after the events written so far.
    table: Table,
    /// What each card is called now.
    names: Vec<String>,
    /// Where each card is, by id: what the stream has by name. Ordered zones top first.
    ids: BTreeMap<events::Zone, Vec<CardId>>,
    /// The cards lying face up in a set-aside zone or `looking`.
    up: BTreeSet<CardId>,
    /// Each pile's zone, once it has been seen.
    piles: Vec<Option<events::Zone>>,
    seq: u64,
    set_up: bool,
    ended: bool,
    /// Whose turn it is: who moves a card that goes between zones nobody owns.
    turn: u8,
    /// The event each of the journal's occurrences became, by its number, and which of
    /// its cards it is about when it is a move.
    occurrences: Vec<Option<(u64, u32)>>,
    /// How many cards each move event moved.
    moved: BTreeMap<u64, u32>,
    /// The occurrence the journal's steps happen inside now.
    within: Option<u32>,
    /// Whether the last step that was no marker wrote an event.
    wrote: bool,
    /// An event that was an occurrence now over: no later move joins it.
    #[serde(default)]
    closed: Option<u64>,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// The table after everything recorded so far.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// How many events it has written.
    pub fn written(&self) -> u64 {
        self.seq
    }

    /// Takes what `game` has journaled since the last call and returns it as events.
    /// `seats` names the players. An error means the journal doesn't describe one game:
    /// a bug in the engine or here.
    pub fn record(&mut self, game: &mut Game, seats: &[String]) -> Result<Vec<Event>, FoldError> {
        let mut steps = Steps(game.take_journal().into());
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
            match step {
                Step::Occurred(n) => {
                    self.occurred(n, &out);
                    continue;
                }
                Step::Within(occ) => {
                    self.within = occ;
                    continue;
                }
                Step::Closed(n) => {
                    // Only the last event can take more cards.
                    if let Some(seq) = self.event_of(Some(n)).filter(|&s| s + 1 == self.seq) {
                        self.closed = Some(seq);
                    }
                    continue;
                }
                _ => self.wrote = false,
            }
            let what = match step {
                Step::Trace {
                    entry: Trace::Created { card, is, zone },
                    ..
                } => {
                    self.name(card, is);
                    let to = self.zone(game, zone);
                    self.arrive(&to, card, At::Top);
                    What::Add {
                        cards: vec![self.card(card)],
                        to,
                    }
                }
                Step::Trace {
                    entry: Trace::Moved { from, to, .. },
                    verb: Some(Verb::Shuffle),
                } => {
                    // Its cards are journaled bottom first.
                    let mut ids = vec![];
                    let mut step = Some(step);
                    while let Some(Step::Trace {
                        entry: Trace::Moved { card, .. },
                        verb: Some(Verb::Shuffle),
                    }) = step
                    {
                        ids.push(card);
                        step = steps.next_if(|next| {
                            matches!(next, Step::Trace { entry: Trace::Moved { from: f, to: t, .. }, verb: Some(Verb::Shuffle) } if *f == from && *t == to)
                        });
                    }
                    ids.reverse();
                    let (from, to) = (self.zone(game, from), self.zone(game, to));
                    let at = if from == to { At::Top } else { At::Bottom };
                    What::Shuffle(self.shuffle(&from, &to, &ids, at))
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
                    let (pos, from_face) = self.leave(&from, card);
                    self.arrive(&to, card, at);
                    let face = to.has_faces().then(|| {
                        match steps.next_if(|next| matches!(next, Step::FaceUp(c) if *c == card)) {
                            Some(_) => {
                                self.up.insert(card);
                                Face::Up
                            }
                            None => Face::Down,
                        }
                    });
                    let played = (verb == Some(Verb::Play))
                        .then(|| {
                            steps.next_past_reads(
                                |next| matches!(next, Step::Played { card: c, .. } if *c == card),
                            )
                        })
                        .flatten();
                    if let Some((Step::Played { player, .. }, occ)) = played {
                        let what = What::Play(Play {
                            player: player.0,
                            card: self.card(card),
                            from: Some(from),
                            to: Some(to),
                            pos,
                            from_face,
                        });
                        self.push(&mut out, what)?;
                        if let Some(n) = occ {
                            self.occurred(n, &out);
                        }
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
                            pos: pos.into_iter().collect(),
                            face,
                            from_face,
                        },
                    )
                }
                Step::Trace {
                    entry: Trace::Changed { card, to, .. },
                    ..
                } => {
                    let was = self.card(card);
                    let zone = (self.ids.iter())
                        .find(|(_, ids)| ids.contains(&card))
                        .map(|(z, _)| z.clone())
                        .ok_or_else(|| FoldError {
                            seq: self.seq,
                            why: format!("{was} changes but is nowhere"),
                        })?;
                    let pos = (zone.ordered()).then(|| self.pos_of(&zone, card)).flatten();
                    self.name(card, to);
                    What::Rename {
                        card: was,
                        name: to.name().to_string(),
                        zone,
                        pos: pos.filter(|&p| p > 0),
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
                Step::EffectChanged(e) => {
                    let e = self.effect(e);
                    // Changed only in which copies it keeps or holds.
                    if self.table.effects.contains(&e) {
                        continue;
                    }
                    What::EffectChange(e)
                }
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
                    from_face: None,
                }),
                Step::Logged { index } => match occurrence(&game.log()[index]) {
                    Some(what) => what,
                    None => continue,
                },
                Step::Resolve {
                    player,
                    card,
                    occasion,
                    origin,
                } => self.resolve(player, card, occasion, origin),
                // Who saw what is not in the full stream.
                Step::Trace { .. } => continue,
                Step::Occurred(_) | Step::Within(_) | Step::Closed(_) => {
                    unreachable!("handled above")
                }
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
            self.within = None;
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
        let mut opening: Vec<(Vec<CardId>, usize)> = vec![(vec![], 0); n];
        for (id, is, zone) in created {
            self.name(id, is);
            match zone {
                // Journaled bottom first; the hand in the order drawn.
                Zone::Deck(p) => opening[p.index()].0.insert(0, id),
                Zone::Hand(p) => {
                    let (deck, drawn) = &mut opening[p.index()];
                    deck.insert(*drawn, id);
                    *drawn += 1;
                }
                zone => {
                    let zone = self.zone(game, zone);
                    let cards = table.zones.entry(zone.clone()).or_default();
                    match zone.ordered() {
                        true => cards.insert(0, self.card(id)),
                        false => events::bag_insert(cards, self.card(id)),
                    }
                    self.arrive(&zone, id, At::Top);
                }
            }
        }
        for zone in table.zones.keys() {
            self.ids.entry(zone.clone()).or_default();
        }
        let mut after = vec![];
        for (seat, (deck, drawn)) in opening.into_iter().enumerate() {
            let seat = seat as u8;
            let zone = events::Zone::Seat(seat, SeatZone::Deck);
            // Before the shuffle the deck is in no order anyone chose: by name.
            let mut unshuffled = deck.clone();
            unshuffled.sort_by_key(|&id| (self.card(id), id));
            table.zones.insert(
                zone.clone(),
                unshuffled.iter().map(|&id| self.card(id)).collect(),
            );
            self.ids.insert(zone.clone(), unshuffled);
            after.push(What::Shuffle(self.shuffle(&zone, &zone, &deck, At::Top)));
            if drawn > 0 {
                let hand = events::Zone::Seat(seat, SeatZone::Hand);
                for &id in &deck[..drawn] {
                    self.leave(&zone, id);
                    self.arrive(&hand, id, At::Top);
                }
                after.push(What::Draw(Move {
                    player: seat,
                    cards: deck[..drawn].iter().map(|&id| self.card(id)).collect(),
                    from: zone,
                    to: hand,
                    at: At::Top,
                    pos: vec![],
                    face: None,
                    from_face: None,
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
            by: self.event_of(self.within),
            what,
        };
        self.table.apply(&event)?;
        self.wrote = true;
        // Cards moving the same way one after the other, for the same cause, are one event,
        // unless that event was an occurrence that is over: the move is another one.
        if let (Some(last), Some((verb, m))) = (out.last_mut(), event.what.as_move())
            && last.by == event.by
            && self.closed != Some(last.seq)
            && let Some((v, _)) = last.what.as_move()
            && v == verb
            && verb != events::Verb::Shuffle
            && let Some(l) = last.what.as_move_mut()
            && (l.player, &l.from, &l.to, l.at, l.face, l.from_face)
                == (m.player, &m.from, &m.to, m.at, m.face, m.from_face)
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
            self.moved.insert(last.seq, l.cards.len() as u32);
            return Ok(());
        }
        if let Some((_, m)) = event.what.as_move() {
            self.moved.insert(event.seq, m.cards.len() as u32);
        }
        self.seq += 1;
        out.push(event);
        Ok(())
    }

    /// Occurrence `n` is the step that wrote the last event, if it wrote one.
    fn occurred(&mut self, n: u32, out: &[Event]) {
        let card = (out.last())
            .and_then(|e| e.what.as_move())
            .map_or(0, |(_, m)| m.cards.len() as u32 - 1);
        let event = self.wrote.then(|| (self.seq - 1, card));
        let n = n as usize;
        if self.occurrences.len() <= n {
            self.occurrences.resize(n + 1, None);
        }
        self.occurrences[n] = event;
    }

    /// The event occurrence `occ` became.
    fn event_of(&self, occ: Option<u32>) -> Option<u64> {
        self.occurrence(occ).map(|(seq, _)| seq)
    }

    fn occurrence(&self, occ: Option<u32>) -> Option<(u64, u32)> {
        occ.and_then(|n| self.occurrences.get(n as usize).copied().flatten())
    }

    fn resolve(
        &mut self,
        player: PlayerId,
        card: Card,
        occasion: Occasion,
        origin: Option<u32>,
    ) -> What {
        let (when, about, target) = match occasion {
            Occasion::GameStart => (Moment::GameStart, None, None),
            Occasion::StartTurn { .. } => (Moment::StartTurn, None, None),
            Occasion::StartBuyPhase { .. } => (Moment::StartBuyPhase, None, None),
            Occasion::EndBuyPhase { .. } => (Moment::EndBuyPhase, None, None),
            Occasion::StartNight { .. } => (Moment::StartNight, None, None),
            Occasion::StartCleanup { .. } => (Moment::StartCleanup, None, None),
            Occasion::DiscardFromPlay { .. } => (Moment::DiscardFromPlay, None, None),
            Occasion::EndTurn { .. } => (Moment::EndTurn, None, None),
            Occasion::Play { .. } => (Moment::Play, None, None),
            Occasion::AfterPlay { .. } => (Moment::AfterPlay, None, None),
            Occasion::Attack { target, .. } => (Moment::Attack, None, Some(target.0)),
            Occasion::Buy { card, .. } => (Moment::Buy, Some(card), None),
            Occasion::BuyLandscape { .. } => (Moment::Buy, None, None),
            Occasion::WouldGain { card, .. } => (Moment::WouldGain, Some(card), None),
            Occasion::Gain { card, .. } => (Moment::Gain, Some(card), None),
            Occasion::Trash { card, .. } => (Moment::Trash, Some(card), None),
            Occasion::LeaveSupply { card, .. } => (Moment::LeaveSupply, Some(card), None),
            Occasion::Discard { card, .. } => (Moment::Discard, Some(card), None),
            Occasion::WouldShuffle { .. } => (Moment::WouldShuffle, None, None),
            Occasion::Shuffled { .. } => (Moment::Shuffle, None, None),
        };
        // Written now, the move it answers can take no more cards.
        let index = (self.occurrence(self.within))
            .filter(|&(seq, _)| about.is_some() && self.moved.get(&seq) > Some(&1))
            .map(|(_, i)| i);
        What::Resolve {
            player: player.0,
            card: card.name().to_string(),
            when,
            about: about.map(|id| self.card(id)),
            target,
            origin: self.event_of(origin),
            index,
        }
    }

    fn name(&mut self, id: CardId, is: Card) {
        if self.names.len() <= id.index() {
            self.names.resize(id.index() + 1, String::new());
        }
        self.names[id.index()] = is.name().to_string();
    }

    fn card(&self, id: CardId) -> CardRef {
        CardRef::named(self.names[id.index()].clone())
    }

    /// `ids` as a bag.
    fn bag(&self, ids: &[CardId]) -> Vec<CardRef> {
        let mut cards: Vec<CardRef> = ids.iter().map(|&c| self.card(c)).collect();
        cards.sort();
        cards
    }

    /// How many cards lie over each of `cards` in `zone` as they leave it one after the
    /// other; nothing when each comes off the top. For Boons and Hexes, each of a name.
    fn depths(&self, zone: &events::Zone, cards: &[CardRef]) -> Vec<u32> {
        if !zone.ordered() {
            return vec![];
        }
        sequential_depths(self.table.zone(zone), cards)
    }

    /// How many cards lie over `id` in the ordered `zone`, as the stream says it: the
    /// topmost of the run of cards of its name it lies in, which are all the same to it.
    fn pos_of(&self, zone: &events::Zone, id: CardId) -> Option<u32> {
        let ids = self.ids.get(zone)?;
        let mut i = ids.iter().position(|&c| c == id)?;
        while i > 0 && self.names[ids[i - 1].index()] == self.names[id.index()] {
            i -= 1;
        }
        Some(i as u32)
    }

    /// `id` leaves `zone`: where it lay (when under other cards in an ordered zone), and
    /// `up` if it lay face up.
    fn leave(&mut self, zone: &events::Zone, id: CardId) -> (Option<u32>, Option<Face>) {
        let pos = match zone.ordered() {
            true => self.pos_of(zone, id).filter(|&p| p > 0),
            false => None,
        };
        if let Some(ids) = self.ids.get_mut(zone) {
            ids.retain(|&c| c != id);
        }
        let up = self.up.remove(&id).then_some(Face::Up);
        (pos, up)
    }

    /// `id` arrives in `zone` at `at`.
    fn arrive(&mut self, zone: &events::Zone, id: CardId, at: At) {
        let ids = self.ids.entry(zone.clone()).or_default();
        let i = match (zone.ordered(), at) {
            (false, _) | (true, At::Bottom) => ids.len(),
            (true, At::Top) => 0,
            (true, At::Under(n)) => (n as usize).min(ids.len()),
        };
        ids.insert(i, id);
    }

    /// The cards `ids` (listed top first as they end up) shuffled from `from` into `to`.
    fn shuffle(&mut self, from: &events::Zone, to: &events::Zone, ids: &[CardId], at: At) -> Move {
        let mut pos = vec![];
        for &id in ids {
            pos.push(self.pos_of(from, id).unwrap_or(0));
            self.leave(from, id);
        }
        if pos.iter().all(|&p| p == 0) {
            pos.clear();
        }
        let block = self.ids.entry(to.clone()).or_default();
        let i = if at == At::Bottom { block.len() } else { 0 };
        block.splice(i..i, ids.iter().copied());
        Move {
            player: self.mover(from, to),
            cards: ids.iter().map(|&id| self.card(id)).collect(),
            from: from.clone(),
            to: to.clone(),
            at,
            pos,
            face: None,
            from_face: None,
        }
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
            Gauge::NextHand => CounterKey::global(N::NextHand),
            Gauge::Bought => CounterKey::global(N::Bought),
            Gauge::HandPlays => CounterKey::global(N::HandPlays),
            Gauge::SkipTurns(p) => seat(p, N::SkipTurns),
            Gauge::TurnsSkipped(p) => seat(p, N::TurnsSkipped),
        }
    }

    fn effect(&self, e: EffectState) -> Effect {
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
            keeps: self.bag(&e.keeps),
            holds: self.bag(&e.holds),
            tokens: e.tokens,
            face_up: self.bag(
                &(e.holds.iter().copied())
                    .filter(|c| self.up.contains(c))
                    .collect::<Vec<_>>(),
            ),
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
                let card = CardRef::named(name(card));
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
                        face: None,
                        from_face: None,
                    },
                )
            }
            Step::FateShuffled { hexes, cards } => {
                let zone = |pack| match hexes {
                    true => events::Zone::Hexes(pack),
                    false => events::Zone::Boons(pack),
                };
                let cards: Vec<CardRef> =
                    cards.into_iter().map(|c| CardRef::named(name(c))).collect();
                let from = zone(Pack::Discard);
                let pos = self.depths(&from, &cards);
                What::Shuffle(Move {
                    player: self.turn,
                    cards,
                    from,
                    to: zone(Pack::Deck),
                    at: At::Top,
                    pos,
                    face: None,
                    from_face: None,
                })
            }
            Step::FateAdded { cards, to } => {
                let cards: Vec<CardRef> =
                    cards.into_iter().map(|c| CardRef::named(name(c))).collect();
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
                    .map(|(c, cost)| (name(c), cost.map(cost_of)))
                    .collect(),
                types: (types.into_iter())
                    .map(|(c, t)| (name(c), t.map(type_names)))
                    .collect(),
            },
            _ => return None,
        })
    }
}

/// The journal's steps not yet turned into events.
struct Steps(std::collections::VecDeque<Step>);

impl Steps {
    fn peek(&self) -> Option<&Step> {
        self.0.front()
    }

    fn next(&mut self) -> Option<Step> {
        self.0.pop_front()
    }

    fn next_if(&mut self, wanted: impl Fn(&Step) -> bool) -> Option<Step> {
        self.0
            .front()
            .is_some_and(wanted)
            .then(|| self.0.pop_front())
            .flatten()
    }

    /// The first step that is no reading of the table, if `wanted`, with the occurrence it
    /// is marked as; the readings before it stay. What a card's move into play changed (a
    /// cost) is read before its play.
    fn next_past_reads(&mut self, wanted: impl Fn(&Step) -> bool) -> Option<(Step, Option<u32>)> {
        let i = self.0.iter().position(|s| !is_reading(s))?;
        if !wanted(&self.0[i]) {
            return None;
        }
        let step = self.0.remove(i)?;
        let occ = match self.0.get(i) {
            Some(&Step::Occurred(n)) if i > 0 => {
                self.0.remove(i);
                Some(n)
            }
            _ => None,
        };
        Some((step, occ))
    }
}

/// Whether `step` is what the journal reads of the table each time it records.
fn is_reading(step: &Step) -> bool {
    matches!(
        step,
        Step::Gauge { .. }
            | Step::EffectBegan(_)
            | Step::EffectChanged(_)
            | Step::EffectEnded { .. }
            | Step::Token { .. }
            | Step::Holding { .. }
            | Step::Pile { .. }
            | Step::Landscape { .. }
            | Step::Notes(_)
            | Step::Derived { .. }
    )
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
/// after the other, each the first of its name; nothing when each comes off the top.
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

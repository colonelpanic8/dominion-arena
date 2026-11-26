//! Ordered server state and conservative player knowledge. Allocated card objects
//! are never treated as observations of hidden zones.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::{
    Error, Result,
    catalog::Catalog,
    knowledge::{KnowledgeZone, Observations},
    model,
    questions::{LogArgument, Question, QuestionContext, Story},
};
use dominion_protocol as view;

type Position = (
    model::TurnDescription,
    Vec<model::Counter>,
    Vec<model::Token>,
    BTreeMap<i32, (i32, i32, u32)>,
);

const DRUID_BOONS: i32 = 20;
const MOUSE_CARD: i32 = 22;
const THE_BOX: i32 = 30;
const SINISTER_PLOT: i32 = 13;

/// A zone as a snapshot gives it: each card's identity and name where it shows them.
struct Stock {
    zone: i32,
    cards: Vec<(i32, Option<i32>)>,
}

/// What was known of a position, for a snapshot of that position: nothing a zone showed
/// is forgotten. None if any zone differs from what was known of it: a card could have
/// left a hidden zone for it while another took its place, so no zone is trusted.
fn recalled(mut knowledge: Observations, stocked: &[Stock]) -> Option<Observations> {
    for stock in stocked {
        let shown: Vec<_> = stock
            .cards
            .iter()
            .filter_map(|&(id, name)| Some((id, name?)))
            .collect();
        if knowledge.count(stock.zone) != stock.cards.len()
            || knowledge.reveal(stock.zone, &shown).is_err()
        {
            return None;
        }
    }
    Some(knowledge)
}

/// Zones the view lists as Supply or non-Supply piles.
fn is_pile(kind: i32) -> bool {
    matches!(kind, 4..=6 | 13 | 32)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Protocol(message.into())
}
fn nonnegative(n: i32) -> Result<u32> {
    u32::try_from(n).map_err(|_| invalid("negative count"))
}

#[derive(Clone)]
pub struct Reducer {
    pub catalog: Arc<Catalog>,
    pub player_id: i32,
    pub names: BTreeMap<i32, String>,
    pub game_id: Option<i64>,
    pub epoch: u64,
    pub revision: u64,
    pub running: bool,
    pub result: Option<view::GameResult>,
    state: Option<model::GameState>,
    zones: BTreeMap<i32, KnowledgeZone>,
    knowledge: Observations,
    known: dominion_knowledge::KnownCards<i32, i32>,
    types: BTreeMap<i32, Vec<String>>,
    visible_ids: BTreeSet<i32>,
    question_visible: BTreeSet<i32>,
    question: Option<Question>,
    serial: u64,
    open_id: Option<String>,
    /// Each pile's named cards in the order the official client stacks them: the
    /// snapshot lists a pile bottom first, and its last named card is the top.
    pile_cards: BTreeMap<i32, Vec<i32>>,
    pub logs: Vec<view::LogEntry>,
}
impl Reducer {
    pub fn new(catalog: Arc<Catalog>, player_id: i32) -> Self {
        Self {
            catalog,
            player_id,
            names: BTreeMap::new(),
            game_id: None,
            epoch: 0,
            revision: 0,
            running: false,
            result: None,
            state: None,
            zones: BTreeMap::new(),
            knowledge: Observations::default(),
            known: Default::default(),
            types: BTreeMap::new(),
            visible_ids: BTreeSet::new(),
            question_visible: BTreeSet::new(),
            question: None,
            serial: 0,
            open_id: None,
            pile_cards: BTreeMap::new(),
            logs: Vec::new(),
        }
    }
    pub fn invalidate(&mut self) {
        self.epoch += 1;
        self.question = None;
        self.open_id = None;
        self.question_visible.clear();
        self.revision += 1;
    }
    /// Validate and normalize an authoritative result without account/session policy.
    pub fn finish(&mut self, result: &model::GameResult) -> Result<()> {
        if self.game_id != Some(result.game_id) {
            return Err(invalid("game result has unexpected id"));
        }
        let state = self.state()?;
        let expected: BTreeSet<_> = state.players.iter().map(|p| p.player_id).collect();
        if expected.len() != state.players.len() || result.players.len() != expected.len() {
            return Err(invalid("game result player count mismatch"));
        }
        if !(-crate::wire::MAX_SAFE_LONG..=crate::wire::MAX_SAFE_LONG).contains(&result.table_id)
            || result.rating_type.is_some_and(|v| !(0..4).contains(&v))
        {
            return Err(invalid("invalid game result metadata"));
        }
        let mut seen = BTreeSet::new();
        let mut players = Vec::new();
        for p in &result.players {
            if !expected.contains(&p.player_id) || !seen.insert(p.player_id) {
                return Err(invalid("unknown or duplicate result player"));
            }
            if p.rank < 1
                || p.rank as usize > expected.len()
                || p.score.used_turns < 0
                || p.resign_index < -1
                || p.resignation_type.is_some_and(|v| !(0..4).contains(&v))
                || p.card_names
                    .iter()
                    .any(|c| c.frequency < 0 || self.catalog.card(c.card_name).is_err())
                || p.score
                    .parts
                    .iter()
                    .any(|part| part.frequency < 0 || self.catalog.card(part.card_name).is_err())
            {
                return Err(invalid("invalid player result"));
            }
            let index = state
                .players
                .iter()
                .position(|s| s.player_id == p.player_id)
                .ok_or_else(|| invalid("result player not in snapshot"))?;
            players.push(view::ResultPlayer {
                index: index as u8,
                name: self.player_name(index as i32),
                vp: p.score.total_points,
                rank: p.rank as u32,
                turns: p.score.used_turns as u32,
                resigned: p.resignation_type.is_some(),
            });
        }
        for name in &result.empty_piles {
            self.catalog.card(*name)?;
        }
        players.sort_by_key(|p| p.index);
        self.invalidate();
        self.running = false;
        self.result = Some(view::GameResult {
            reason: Some("official server result".into()),
            players,
        });
        Ok(())
    }
    /// Whether a question is open.
    pub fn asking(&self) -> bool {
        self.question.is_some()
    }
    pub fn seat(&self) -> Option<u8> {
        self.state
            .as_ref()?
            .players
            .iter()
            .position(|p| p.player_id == self.player_id)
            .and_then(|i| u8::try_from(i).ok())
    }
    pub fn state(&self) -> Result<&model::GameState> {
        self.state
            .as_ref()
            .ok_or_else(|| Error::Rejected("NO_GAME: no official game snapshot".into()))
    }
    fn can_see_private(&self, owner: i32) -> bool {
        let seat = self.seat().map(i32::from);
        seat == Some(owner)
            || self
                .state
                .as_ref()
                .is_some_and(|s| seat == Some(s.turn.controller_id) && owner == s.turn.owner_id)
    }
    fn zone(&self, index: i32) -> Result<&model::Zone> {
        self.state()?
            .zones
            .iter()
            .find(|z| z.index == index)
            .ok_or_else(|| invalid(format!("unknown zone {index}")))
    }
    fn raw_name(&self, id: i32) -> Result<i32> {
        self.state()?
            .cards
            .get(usize::try_from(id).map_err(|_| invalid(format!("negative card instance {id}")))?)
            .map(|c| c.card_name)
            .ok_or_else(|| invalid(format!("unknown card instance {id}")))
    }
    fn disclosed(&self, id: i32) -> Result<Option<i32>> {
        if id == -1 {
            return Ok(None);
        }
        let name = self.raw_name(id)?;
        if name == 0
            || self
                .state()?
                .card_statuses
                .iter()
                .any(|s| s.card == id && s.properties.contains(&1))
        {
            return Ok(None);
        }
        Ok(Some(name))
    }
    pub fn replace(&mut self, snapshot: model::Snapshot) -> Result<()> {
        self.invalidate();
        let mut next = self.clone();
        next.replace_inner(snapshot)
            .map_err(|e| invalid(format!("snapshot: {e}")))?;
        *self = next;
        Ok(())
    }
    /// Where the game stands, apart from which cards are where.
    fn position(&self) -> Option<Position> {
        let state = self.state.as_ref()?;
        let zones = state
            .zones
            .iter()
            .map(|z| {
                let count = self.zones.get(&z.index).map_or(0, |k| k.count);
                (z.index, (z.kind, z.owner, count))
            })
            .collect();
        Some((
            state.turn.clone(),
            state.counters.clone(),
            state.tokens.clone(),
            zones,
        ))
    }
    fn replace_inner(&mut self, snapshot: model::Snapshot) -> Result<()> {
        let before = (self.game_id == Some(snapshot.game_id))
            .then(|| Some((self.position()?, self.knowledge.clone())))
            .flatten();
        self.game_id = Some(snapshot.game_id);
        self.running = true;
        self.result = None;
        self.logs.clear();
        self.zones.clear();
        self.knowledge = Observations::default();
        self.known = Default::default();
        self.types.clear();
        self.visible_ids.clear();
        self.pile_cards.clear();
        self.state = Some(snapshot.state);
        let state = self.state()?.clone();
        if !(2..=6).contains(&state.players.len()) {
            return Err(invalid("unsupported player count"));
        }
        let seat = self.seat().map(i32::from);
        let mut stocked = vec![];
        for zone in &state.zones {
            if zone.index < 0 || self.zones.contains_key(&zone.index) {
                return Err(invalid("duplicate/negative zone index"));
            }
            if zone.owner < -1 || zone.owner >= state.players.len() as i32 {
                return Err(invalid("zone owner outside players"));
            }
            self.knowledge.create_zone(zone.index);
            let can_see = match zone.kind {
                1 | 9 | 14..=19 | 30 | 33 => false,
                0 | 11 | 24 | 26..=28 | 31 => {
                    seat == Some(zone.owner)
                        || (seat == Some(state.turn.controller_id)
                            && zone.owner == state.turn.owner_id)
                }
                _ => true,
            };
            let mut cards = vec![];
            for id in &zone.cards {
                // Even when a snapshot allocates the entire deck, it is not a reveal.
                let name = if can_see { self.disclosed(*id)? } else { None };
                let id = if name.is_some() { *id } else { -1 };
                self.knowledge.insert(zone.index, id, name);
                if id >= 0 {
                    self.visible_ids.insert(id);
                }
                cards.push((id, name));
            }
            if is_pile(zone.kind) {
                let named = cards.iter().filter(|(_, n)| n.is_some()).map(|(id, _)| *id);
                self.pile_cards.insert(zone.index, named.collect());
            }
            stocked.push(Stock {
                zone: zone.index,
                cards,
            });
            self.zones.insert(zone.index, KnowledgeZone::default());
        }
        self.refresh_knowledge();
        if let Some((position, knowledge)) = before
            && self.position() == Some(position)
            && let Some(knowledge) = recalled(knowledge, &stocked)
        {
            self.knowledge = knowledge;
            self.refresh_knowledge();
        }
        for e in &state.temporary_effects.renames {
            self.rename(e.id, e.card_name)?;
        }
        for e in &state.temporary_effects.card_type_changes {
            self.change_types(&model::ChangeCardTypes {
                cards: vec![e.id],
                added: e.added.clone(),
                removed: vec![],
            })?;
        }
        for event in snapshot.events {
            self.apply_inner(event)?;
        }
        self.refresh_knowledge();
        self.validate()?;
        Ok(())
    }
    pub fn apply(&mut self, event: model::Event) -> Result<()> {
        let mut next = self.clone();
        if let Err(error) = next.apply_inner(event).and_then(|_| next.validate()) {
            self.invalidate();
            return Err(error);
        }
        next.revision += 1;
        *self = next;
        Ok(())
    }
    fn apply_inner(&mut self, event: model::Event) -> Result<()> {
        use model::Event::*;
        match event {
            CardMove(m) => self.move_cards(m)?,
            CounterChange(c) => {
                let counter = self
                    .state
                    .as_mut()
                    .ok_or_else(|| invalid("counter before snapshot"))?
                    .counters
                    .get_mut(c.index as usize)
                    .ok_or_else(|| invalid("unknown counter"))?;
                counter.value = c.new_value;
            }
            PileUpdate(p) => {
                let name = if p.top_card_id >= 0 {
                    self.disclosed(p.top_card_id)?
                } else {
                    None
                };
                if p.top_card_id < 0 {
                    self.knowledge.reconcile(p.index, 0);
                } else if let Some(name) = name {
                    self.knowledge.reveal(p.index, &[(p.top_card_id, name)])?;
                }
                let raw = self
                    .state
                    .as_mut()
                    .unwrap()
                    .zones
                    .iter_mut()
                    .find(|z| z.index == p.index)
                    .ok_or_else(|| invalid("unknown pile"))?;
                raw.cards.retain(|id| *id != p.top_card_id);
                if p.top_card_id >= 0 {
                    raw.cards.insert(0, p.top_card_id);
                }
                let kind = raw.kind;
                if is_pile(kind) {
                    let count = self.knowledge.count(p.index);
                    let cards = self.pile_cards.entry(p.index).or_default();
                    // As the client: the new top replaces an unnamed card if there is
                    // one, else every named card it doesn't already hold.
                    if p.top_card_id < 0 {
                        cards.clear();
                    } else if cards.contains(&p.top_card_id) || cards.len() < count {
                        cards.retain(|id| *id != p.top_card_id);
                        cards.push(p.top_card_id);
                    } else {
                        *cards = vec![p.top_card_id];
                    }
                }
            }
            TurnDescription(t) => {
                self.state
                    .as_mut()
                    .ok_or_else(|| invalid("turn before snapshot"))?
                    .turn = t;
                self.question = None;
                self.open_id = None;
                self.question_visible.clear();
            }
            Shuffle(s) => {
                let deck = self
                    .state()?
                    .zones
                    .iter()
                    .find(|z| z.owner == s.owner && z.kind == 1)
                    .map(|z| z.index)
                    .ok_or_else(|| invalid("shuffle without draw pile"))?;
                let discard = if s.include_discard {
                    Some(
                        self.state()?
                            .zones
                            .iter()
                            .find(|z| z.owner == s.owner && z.kind == 2)
                            .map(|z| z.index)
                            .ok_or_else(|| invalid("shuffle without discard"))?,
                    )
                } else {
                    None
                };
                self.knowledge.shuffle(deck, discard);
            }
            Inspection(i) => {
                let can_observe = i.is_public || self.can_see_private(i.player);
                if i.to_zone >= 0 && i.from_zone != i.to_zone {
                    self.move_cards(model::CardMove {
                        from_zone: i.from_zone,
                        to_zone: i.to_zone,
                        before: vec![-1; i.cards.len()],
                        after: if can_observe {
                            i.cards
                        } else {
                            vec![-1; i.cards.len()]
                        },
                        movement_type: 0,
                        animation_class: 0,
                    })?;
                } else if can_observe {
                    let revealed = i
                        .cards
                        .into_iter()
                        .filter_map(|id| match self.disclosed(id) {
                            Ok(Some(name)) => Some(Ok((id, name))),
                            Ok(None) => None,
                            Err(e) => Some(Err(e)),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    self.knowledge.reveal(i.from_zone, &revealed)?;
                }
            }
            CostReduction(c) => {
                self.state
                    .as_mut()
                    .ok_or_else(|| invalid("cost before snapshot"))?
                    .cost_reduction = c
            }
            TokenChange(t) => {
                self.zone(t.zone)?;
                let token = self
                    .state
                    .as_mut()
                    .unwrap()
                    .tokens
                    .get_mut(t.token as usize)
                    .ok_or_else(|| invalid("unknown token"))?;
                token.zone = t.zone;
                token.is_flipped = t.is_flipped;
            }
            ZoneCreated(z) => {
                if z.owner < -1 || z.owner >= self.state()?.players.len() as i32 {
                    return Err(invalid("zone owner outside players"));
                }
                if self.zones.contains_key(&z.index) || z.index < 0 {
                    return Err(invalid("zone already exists"));
                }
                self.state
                    .as_mut()
                    .ok_or_else(|| invalid("zone before snapshot"))?
                    .zones
                    .push(model::Zone {
                        index: z.index,
                        kind: z.kind,
                        pile_name: 0,
                        owner: z.owner,
                        cards: vec![],
                        created_by: z.created_by,
                        attached_trait: 0,
                        is_visible: true,
                    });
                self.knowledge.create_zone(z.index);
                self.zones.insert(z.index, KnowledgeZone::default());
                if is_pile(z.kind) {
                    self.pile_cards.insert(z.index, vec![]);
                }
            }
            ZoneDestroyed(z) => {
                let old = self
                    .zones
                    .remove(&z.index)
                    .ok_or_else(|| invalid("destroy unknown zone"))?;
                if old.count > 0 {
                    return Err(invalid("destroying a nonempty zone"));
                }
                self.knowledge.destroy_zone(z.index);
                self.pile_cards.remove(&z.index);
                self.state
                    .as_mut()
                    .unwrap()
                    .zones
                    .retain(|zone| zone.index != z.index);
            }
            Rename(r) => self.rename(r.id, r.card_name)?,
            BehaviourChange(change) => {
                self.raw_name(change.id)?;
                self.catalog.card(change.card_name)?;
                let changes = &mut self
                    .state
                    .as_mut()
                    .unwrap()
                    .temporary_effects
                    .behaviour_changes;
                changes.retain(|c| c.id != change.id);
                changes.push(change);
            }
            ImageChange(change) => {
                self.raw_name(change.id)?;
                self.catalog.card(change.card_name)?;
                let changes = &mut self.state.as_mut().unwrap().temporary_effects.image_changes;
                changes.retain(|c| c.id != change.id);
                changes.push(change);
            }
            PilesStatus(p) => self.piles_status(p)?,
            RemindersChange(r) => {
                self.state
                    .as_mut()
                    .ok_or_else(|| invalid("reminders before snapshot"))?
                    .players
                    .get_mut(r.player as usize)
                    .ok_or_else(|| invalid("reminder owner"))?
                    .reminders = r.reminders;
            }
            Association(a) => {
                self.raw_name(a.card)?;
                self.state
                    .as_mut()
                    .unwrap()
                    .associations
                    .retain(|v| v.card != a.card);
                self.state.as_mut().unwrap().associations.push(a);
            }
            CardStatus(s) => {
                if s.properties.contains(&1) {
                    self.knowledge.forget(s.card);
                    self.visible_ids.remove(&s.card);
                }
                let state = self
                    .state
                    .as_mut()
                    .ok_or_else(|| invalid("status before snapshot"))?;
                state.card_statuses.retain(|v| v.card != s.card);
                state.card_statuses.push(s);
            }
            ProjectBought(p) => {
                self.catalog.card(p.project)?;
                self.state
                    .as_mut()
                    .ok_or_else(|| invalid("project before snapshot"))?
                    .players
                    .get_mut(p.player as usize)
                    .ok_or_else(|| invalid("project owner"))?
                    .bought_projects
                    .push(p.project);
            }
            ProjectRemoved(p) => {
                self.state
                    .as_mut()
                    .ok_or_else(|| invalid("project before snapshot"))?
                    .players
                    .get_mut(p.player as usize)
                    .ok_or_else(|| invalid("project owner"))?
                    .bought_projects
                    .retain(|c| *c != p.project);
            }
            ChangeCardTypes(c) => self.change_types(&c)?,
            PileReorder(p) => {
                let zone = self.zone(p.pile)?.clone();
                let can_see = match zone.kind {
                    1 | 9 | 14..=19 | 30 | 33 => false,
                    0 | 11 | 24 | 26..=28 | 31 => self.can_see_private(zone.owner),
                    _ => true,
                };
                let mut revealed = vec![];
                for id in &p.cards {
                    if *id < -1 {
                        return Err(invalid("invalid reorder identity"));
                    }
                    if *id >= 0 {
                        self.raw_name(*id)?;
                        if can_see && let Some(name) = self.disclosed(*id)? {
                            revealed.push((*id, name));
                        }
                    }
                }
                self.knowledge.reconcile(p.pile, p.cards.len() as u32);
                self.knowledge.shuffle(p.pile, None);
                self.knowledge.reveal(p.pile, &revealed)?;
                if is_pile(zone.kind) {
                    let named = p
                        .cards
                        .iter()
                        .filter(|id| revealed.iter().any(|(r, _)| r == *id));
                    self.pile_cards.insert(p.pile, named.copied().collect());
                }
                self.state
                    .as_mut()
                    .unwrap()
                    .zones
                    .iter_mut()
                    .find(|z| z.index == p.pile)
                    .unwrap()
                    .cards = p.cards;
            }
        }
        self.refresh_knowledge();
        Ok(())
    }

    fn piles_status(&mut self, p: model::PilesStatus) -> Result<()> {
        if p.draw_size > 65536 || p.discard_size > 65536 {
            return Err(invalid("zone count exceeds protocol limit"));
        }
        let draw = nonnegative(p.draw_size)?;
        let discard = nonnegative(p.discard_size)?;
        let draw_zone = self.zone(p.draw_index)?;
        let discard_zone = self.zone(p.discard_index)?;
        if draw_zone.kind == 13 && matches!(discard_zone.kind, 15 | 16) {
            let (draw, discard) = (draw as usize, discard as usize);
            let before_draw = self.knowledge.count(p.draw_index);
            let before_discard = self.knowledge.count(p.discard_index);
            // Boons and Hexes announce the post-move counts before CardMove transfers the card.
            let drawing = draw + 1 == before_draw && discard == before_discard;
            let returning = draw == before_draw && discard == before_discard + 1;
            if drawing || returning {
                if p.top_card_id >= 0 {
                    self.raw_name(p.top_card_id)?;
                }
                return Ok(());
            }
            // Refilling the shared deck is announced by status alone.
            if before_draw == 0 && discard == 0 && draw > 0 && draw == before_discard {
                self.knowledge.shuffle(p.draw_index, Some(p.discard_index));
                return Ok(());
            }
        }
        self.knowledge.reconcile(p.draw_index, draw);
        self.knowledge.reconcile(p.discard_index, discard);
        if discard > 0
            && p.top_card_id >= 0
            && let Some(name) = self.disclosed(p.top_card_id)?
        {
            self.knowledge
                .reveal(p.discard_index, &[(p.top_card_id, name)])?;
        }
        Ok(())
    }

    fn move_cards(&mut self, m: model::CardMove) -> Result<()> {
        if m.before.len() != m.after.len() {
            return Err(invalid("movement parallel arrays"));
        }
        self.zone(m.from_zone)?;
        self.zone(m.to_zone)?;
        let cards = m
            .before
            .into_iter()
            .zip(m.after)
            .map(|(before, after)| {
                Ok((
                    before,
                    after,
                    self.disclosed(after)?.or(self.disclosed(before)?),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        if let Some(from) = self.pile_cards.get_mut(&m.from_zone) {
            from.retain(|id| !cards.iter().any(|(before, ..)| before == id));
        }
        if let Some(to) = self.pile_cards.get_mut(&m.to_zone) {
            // A card arriving unseen covers the top: what is on top is no longer known.
            for (_, after, name) in &cards {
                match name {
                    Some(_) if *after >= 0 => to.push(*after),
                    _ => to.clear(),
                }
            }
        }
        self.knowledge.move_cards(m.from_zone, m.to_zone, &cards)
    }
    fn rename(&mut self, id: i32, name: i32) -> Result<()> {
        self.catalog.card(name)?;
        let card = self
            .state
            .as_mut()
            .ok_or_else(|| invalid("rename before snapshot"))?
            .cards
            .get_mut(id as usize)
            .ok_or_else(|| invalid("rename unknown card"))?;
        card.card_name = name;
        self.knowledge.rename(id, name);
        Ok(())
    }
    fn change_types(&mut self, c: &model::ChangeCardTypes) -> Result<()> {
        for id in &c.cards {
            let base = self.catalog.card(self.raw_name(*id)?)?.types.clone();
            let types = self.types.entry(*id).or_insert(base);
            for t in &c.removed {
                let name = self
                    .catalog
                    .enums
                    .get("Types")
                    .and_then(|v| v.get(*t as usize))
                    .ok_or_else(|| invalid("unknown type"))?;
                types.retain(|s| !s.eq_ignore_ascii_case(name));
            }
            for t in &c.added {
                let name = self
                    .catalog
                    .enums
                    .get("Types")
                    .and_then(|v| v.get(*t as usize))
                    .ok_or_else(|| invalid("unknown type"))?;
                let name = title(name);
                if !types.contains(&name) {
                    types.push(name);
                }
            }
        }
        Ok(())
    }
    fn refresh_knowledge(&mut self) {
        let Some(state) = &self.state else {
            return;
        };
        self.known = self.knowledge.summary(state.players.len(), |index| {
            state
                .zones
                .iter()
                .find(|z| z.index == index)
                .and_then(|z| usize::try_from(z.owner).ok())
        });
        self.zones = self.knowledge.zones(&self.known);
        self.visible_ids = self
            .zones
            .values()
            .flat_map(|z| z.exact.keys().copied())
            .collect();
    }
    fn validate(&self) -> Result<()> {
        for zone in self.zones.values() {
            if zone.count > 65536 || zone.known.values().sum::<u32>() > zone.count {
                return Err(invalid("knowledge exceeds zone count"));
            }
        }
        Ok(())
    }
    pub fn ask(&mut self, question: Question) -> Result<()> {
        if self.seat().is_none() {
            return Err(Error::Unsupported("spectator questions".into()));
        }
        self.serial += 1;
        let id = format!(
            "online:{}:{}:{}:{}",
            self.game_id.unwrap_or_default(),
            self.epoch,
            self.serial,
            question.index
        );
        let old = std::mem::take(&mut self.question_visible);
        reveal_question(&question, &mut self.question_visible);
        if let Err(error) = question.project(self, &id, self.seat()) {
            self.question_visible = old;
            return Err(error);
        }
        self.question = Some(question);
        self.open_id = Some(id);
        self.revision += 1;
        Ok(())
    }
    /// Consume the decision before any network send. A failed send is never retried.
    pub fn answer(
        &mut self,
        answer: &view::Answer,
        expect: Option<&[String]>,
    ) -> Result<(i32, Vec<i32>)> {
        let q = self.question.as_ref().ok_or_else(|| {
            Error::Rejected("no open official decision; call state or wait".into())
        })?;
        let id = self
            .open_id
            .as_deref()
            .ok_or_else(|| Error::Rejected("decision invalidated".into()))?;
        let values = q.encode_answer(self, id, answer, expect)?;
        let index = q.index;
        self.question = None;
        self.open_id = None;
        self.question_visible.clear();
        self.revision += 1;
        Ok((index, values))
    }
    /// What a graphical table draws beside the view: each player's cards in play in the
    /// order they came into play, as the official client lays them out.
    pub fn table(&self) -> Result<view::api::TableExtras> {
        let state = self.state()?;
        let players = (0..state.players.len() as i32)
            .map(|owner| {
                let in_play = state
                    .zones
                    .iter()
                    .filter(|z| z.owner == owner && z.kind == 8)
                    .flat_map(|z| self.knowledge.ordered(z.index))
                    .filter_map(|(id, name)| {
                        let card = self.catalog.card(name?).ok()?.name.clone();
                        let status = state.card_statuses.iter().find(|s| Some(s.card) == id);
                        Some(view::api::InPlay {
                            stays: status.is_some_and(|s| s.stays_out),
                            holds: card == "Garrison"
                                || status.is_some_and(|s| !s.set_aside_card_lists.is_empty()),
                            card,
                        })
                    })
                    .collect();
                view::api::PlayerTable {
                    seat: owner as u8,
                    in_play,
                    discard_top: None,
                }
            })
            .collect();
        Ok(view::api::TableExtras { players })
    }
    pub fn view(&self) -> Result<view::AgentView> {
        let state = self.state()?;
        let seat = self.seat();
        let mut supply = Vec::new();
        let mut non_supply = Vec::new();
        let mut landscapes = Vec::new();
        let mut trash = view::ZoneView::default();
        for zone in &state.zones {
            if zone.kind == 3 {
                trash = self.zone_view(zone.index)?;
            }
            if matches!(zone.kind, 4..=7 | 13 | 32 | 34) {
                let name = self.pile_name(zone);
                let info = self.catalog.card(name)?;
                if name == 0 {
                    continue;
                }
                if matches!(zone.kind, 7 | 34) {
                    let tokens = |kind| {
                        let on_it = |c: &&model::Counter| {
                            c.zone == zone.index && c.kind == kind && c.is_visible
                        };
                        let n: i32 = state.counters.iter().filter(on_it).map(|c| c.value).sum();
                        (n != 0).then_some(n)
                    };
                    let kind = info
                        .types
                        .iter()
                        .find(|t| {
                            matches!(
                                t.as_str(),
                                "Event"
                                    | "Landmark"
                                    | "Project"
                                    | "Way"
                                    | "Ally"
                                    | "Trait"
                                    | "Prophecy"
                            )
                        })
                        .cloned()
                        .unwrap_or_else(|| "other".into())
                        .to_lowercase();
                    let bought = matches!(kind.as_str(), "event" | "project");
                    landscapes.push(view::LandscapeView {
                        card: info.name.clone(),
                        cost: bought.then(|| info.cost.clone()),
                        kind,
                        types: info.types.clone(),
                        vp: tokens(7),
                        suns: tokens(18).and_then(|n| u32::try_from(n).ok()),
                        active: None,
                        owners: state
                            .players
                            .iter()
                            .enumerate()
                            .filter(|(_, p)| p.bought_projects.contains(&name))
                            .map(|(i, _)| i as u8)
                            .collect(),
                    });
                } else {
                    // Only a top known to be there: a mixed pile's other cards say nothing
                    // of which is on top.
                    let top = self
                        .pile_cards
                        .get(&zone.index)
                        .and_then(|cards| cards.last())
                        .and_then(|id| {
                            self.zones
                                .get(&zone.index)?
                                .exact
                                .get(id)
                                .map(|n| (*id, *n))
                        });
                    let top_info = top
                        .map(|(_, n)| self.catalog.card(n))
                        .transpose()?
                        .unwrap_or(info);
                    let pile = view::PileView {
                        card: info.name.clone(),
                        count: self.zones[&zone.index].count,
                        top: (top_info.name != info.name).then(|| top_info.name.clone()),
                        cost: Some(self.cost(
                            top.map(|(id, _)| id),
                            top.map_or(zone.pile_name, |(_, n)| n),
                        )?),
                        types: top_info.types.clone(),
                        kingdom: info.kingdom,
                        hidden: false,
                        members: Vec::new(),
                        structure: None,
                        tokens: view::PileTokens {
                            embargo: state
                                .counters
                                .iter()
                                .filter(|c| c.zone == zone.index && c.kind == 9 && c.is_visible)
                                .map(|c| c.value.max(0) as u32)
                                .sum(),
                            blockade: state
                                .counters
                                .iter()
                                .filter(|c| {
                                    c.zone == zone.index
                                        && c.kind == BLOCKADE_TOKENS
                                        && c.is_visible
                                })
                                .map(|c| c.value.max(0) as u32)
                                .sum(),
                            trade_route: state.counters.iter().any(|c| {
                                c.zone == zone.index && c.kind == 10 && c.is_visible && c.value > 0
                            }),
                            vp: state
                                .counters
                                .iter()
                                .filter(|c| c.zone == zone.index && c.kind == 7 && c.is_visible)
                                .map(|c| c.value)
                                .sum(),
                            debt: state
                                .counters
                                .iter()
                                .filter(|c| c.zone == zone.index && c.kind == 6 && c.is_visible)
                                .map(|c| c.value)
                                .sum(),
                            players: state
                                .tokens
                                .iter()
                                .filter(|t| t.zone == zone.index && t.kind <= 5 && t.owner >= 0)
                                .map(|t| view::PlayerToken {
                                    player: t.owner as u8,
                                    token: self
                                        .catalog
                                        .label("TokenNames", t.kind)
                                        .unwrap_or_else(|| format!("token {}", t.kind)),
                                })
                                .collect(),
                        },
                        traits: if zone.attached_trait > 0 {
                            vec![self.catalog.card(zone.attached_trait)?.name.clone()]
                        } else {
                            vec![]
                        },
                    };
                    if matches!(zone.kind, 4..=6) {
                        supply.push(pile);
                    } else {
                        non_supply.push(pile);
                    }
                }
            }
        }
        let turn_player = u8::try_from(state.turn.owner_id)
            .ok()
            .filter(|i| (*i as usize) < state.players.len());
        let counter = |kind| {
            state
                .counters
                .iter()
                .find(|c| {
                    c.kind == kind
                        && c.owner == state.turn.owner_id
                        && c.zone < 0
                        && c.card < 0
                        && c.is_visible
                })
                .and_then(|c| u32::try_from(c.value).ok())
        };
        let turn = view::TurnView {
            number: u32::try_from(state.turn.turn_number).ok(),
            player: turn_player,
            active_player: turn_player.map(|p| self.player_name(p as i32)),
            your_turn: seat.is_some()
                && (seat == turn_player || seat.map(i32::from) == Some(state.turn.controller_id)),
            phase: None,
            extra: (state.turn.turn_type > 1)
                .then(|| self.catalog.label("ExtraTurnTypes", state.turn.turn_type))
                .flatten(),
            actions: counter(0),
            buys: counter(3),
            coins: counter(1),
            potions: counter(2),
            debt: counter(6),
        };
        let you = seat.map(|s| self.player(s)).transpose()?;
        let opponents = (1..=state.players.len())
            .filter_map(|offset| {
                let i = (seat.map_or(state.players.len() - 1, usize::from) + offset)
                    % state.players.len();
                (seat != Some(i as u8)).then_some(i as u8)
            })
            .map(|s| self.player(s))
            .collect::<Result<Vec<_>>>()?;
        let decision = self
            .question
            .as_ref()
            .zip(self.open_id.as_deref())
            .map(|(q, id)| q.project(self, id, seat))
            .transpose()?;
        Ok(view::AgentView {
            schema: view::SCHEMA.into(),
            game: view::GameInfo {
                id: self.game_id.map(|id| format!("dominion.games:{id}")),
                running: self.running,
                result: self.result.clone(),
                kingdom: supply
                    .iter()
                    .filter(|p| p.kingdom)
                    .map(|p| p.card.clone())
                    .collect(),
                landscapes: landscapes.iter().map(|p| p.card.clone()).collect(),
            },
            seat,
            spectating: seat.is_none(),
            turn,
            you,
            opponents,
            supply,
            non_supply,
            landscapes,
            trash,
            uncertain: self
                .known
                .uncertain
                .iter()
                .filter(|u| {
                    let owner = self.zone(u.zones[0].0).ok().map(|z| z.owner);
                    u.outside == 0
                        && !(owner.is_some_and(|p| p >= 0)
                            && u.zones
                                .iter()
                                .all(|(z, _)| self.zone(*z).ok().map(|z| z.owner) == owner))
                })
                .map(|u| {
                    Ok(view::SharedUncertain {
                        zones: u
                            .zones
                            .iter()
                            .map(|(z, n)| {
                                let zone = self.zone(*z)?;
                                Ok(view::ZoneRef {
                                    player: u8::try_from(zone.owner).ok(),
                                    pile: if zone.owner < 0
                                        && matches!(zone.kind, 4..=7 | 13 | 32 | 34)
                                    {
                                        Some(self.catalog.card(self.pile_name(zone))?.name.clone())
                                    } else {
                                        None
                                    },
                                    zone: self.projected_zone(*z)?,
                                    count: *n as u32,
                                })
                            })
                            .collect::<Result<_>>()?,
                        count: u.zones.iter().map(|(_, n)| *n as u32).sum(),
                        candidates: self.named_counts(&u.candidates)?,
                        unknown: u.unknown as u32,
                    })
                })
                .collect::<Result<_>>()?,
            boons: None,
            hexes: None,
            artifacts: (0..state.players.len())
                .map(|i| {
                    self.reminder_cards(i as u8, "Artifact").map(|cards| {
                        cards
                            .into_iter()
                            .map(|card| view::ArtifactView {
                                card,
                                holder: Some(i as u8),
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect(),
            lasting: state
                .card_statuses
                .iter()
                .filter(|s| s.stays_out && self.visible_ids.contains(&s.card))
                .filter_map(|s| {
                    let zone = state
                        .zones
                        .iter()
                        .find(|z| self.zones[&z.index].exact.contains_key(&s.card))?;
                    if zone.owner < 0 {
                        return None;
                    }
                    let card = self.card_name(s.card)?;
                    let cards = s
                        .set_aside_card_lists
                        .iter()
                        .flatten()
                        .filter_map(|id| self.card_name(*id))
                        .collect();
                    Some(view::LastingView {
                        card,
                        owner: zone.owner as u8,
                        until: "official server lasting effect".into(),
                        until_player: None,
                        cards,
                        count: None,
                    })
                })
                .collect(),
            extra_turns: vec![],
            notes: self.notes()?,
            counters: self.counters(-1),
            decision,
            since_last_decision: self.logs.clone(),
        })
    }
    fn notes(&self) -> Result<Vec<view::NoteView>> {
        let state = self.state()?;
        let mut notes = Vec::new();
        for c in &state.counters {
            if c.kind == SINISTER_PLOT
                && c.is_visible
                && let Ok(seat) = u8::try_from(c.owner)
                && let Some(p) = state.players.get(usize::from(seat))
                && p.bought_projects.iter().any(|&project| {
                    self.catalog
                        .card(project)
                        .is_ok_and(|p| p.key == "SINISTER_PLOT")
                })
            {
                notes.push(view::NoteView {
                    setup: None,
                    source: "Sinister Plot".into(),
                    player: Some(seat),
                    pile: None,
                    card: None,
                    count: c.value,
                });
            }
        }
        for marker in &state.pile_markers {
            let zone = self.zone(marker.zone_index)?;
            let (source, setup) = match marker.kind {
                0 => ("Young Witch", Some(view::SetupRole::Bane)),
                1 => ("Obelisk", Some(view::SetupRole::ChosenPile)),
                2 => ("Split pile", None),
                3 => ("Mixed pile", None),
                4 => ("Ferryman", Some(view::SetupRole::ExtraPile)),
                _ => ("Pile marker", None),
            };
            notes.push(view::NoteView {
                setup,
                source: source.into(),
                player: None,
                pile: Some(self.catalog.card(zone.pile_name)?.name.clone()),
                card: None,
                count: 1,
            });
        }
        for zone in state.zones.iter().filter(|z| z.owner < 0) {
            let source = match zone.kind {
                DRUID_BOONS => "Druid",
                MOUSE_CARD => "Way of the Mouse",
                _ => continue,
            };
            for (_, name) in self.knowledge.ordered(zone.index) {
                if let Some(name) = name {
                    notes.push(setup_set_aside(
                        source,
                        self.catalog.card(name)?.name.clone(),
                    ));
                }
            }
        }
        for (player, p) in state.players.iter().enumerate() {
            for reminder in &p.reminders {
                notes.push(view::NoteView {
                    setup: None,
                    source: if reminder.blinking {
                        "Active reminder"
                    } else {
                        "Reminder"
                    }
                    .into(),
                    player: Some(player as u8),
                    pile: None,
                    card: Some(self.catalog.card(reminder.card_name)?.name.clone()),
                    count: reminder.frequency,
                });
            }
        }
        for change in &state.temporary_effects.behaviour_changes {
            if let Some(card) = self.card_name(change.id) {
                notes.push(view::NoteView {
                    setup: None,
                    source: format!(
                        "Uses {} abilities",
                        self.catalog.card(change.card_name)?.name
                    ),
                    player: None,
                    pile: None,
                    card: Some(card),
                    count: 1,
                });
            }
        }
        for association in &state.associations {
            // The official table shows Riverboat's association even when its card is
            // allocated off-table or in the Box. Player zones still use normal visibility.
            if association.linked_cards.iter().any(|&id| {
                self.disclosed(id)
                    .ok()
                    .flatten()
                    .and_then(|name| self.catalog.card(name).ok())
                    .is_some_and(|card| card.key == "RIVERBOAT")
            }) && !state
                .zones
                .iter()
                .any(|z| z.kind != THE_BOX && z.cards.contains(&association.card))
                && let Some(name) = self.disclosed(association.card)?
            {
                let note = setup_set_aside("Riverboat", self.catalog.card(name)?.name.clone());
                if !notes.contains(&note) {
                    notes.push(note);
                }
                continue;
            }
            if let Some(card) = self.card_name(association.card) {
                for linked in &association.linked_cards {
                    if let Some(name) = self.card_name(*linked) {
                        notes.push(view::NoteView {
                            setup: None,
                            source: format!("Associated with {name}"),
                            player: None,
                            pile: None,
                            card: Some(card.clone()),
                            count: 1,
                        });
                    }
                }
            }
        }
        Ok(notes)
    }
    /// `owner`'s counters (nobody's: -1) that no other part of the view shows, by name.
    /// Left out are the turn's Actions, Buys and money, a player's points and tokens,
    /// and the tokens on piles and landscapes.
    fn counters(&self, owner: i32) -> BTreeMap<String, i64> {
        let Ok(state) = self.state() else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for c in &state.counters {
            let shown = match (c.owner >= 0, c.zone >= 0) {
                _ if c.kind == BLOCKADE_TOKENS && c.zone >= 0 => true,
                (true, _) => matches!(c.kind, 0..=8 | 12 | SINISTER_PLOT | 15),
                (false, true) => matches!(c.kind, 6 | 7 | 9 | 10 | 18),
                (false, false) => false,
            };
            if c.owner != owner || !c.is_visible || shown || c.value == 0 {
                continue;
            }
            let mut name = self
                .catalog
                .label("CounterNames", c.kind)
                .unwrap_or_else(|| "counter".into());
            let pile = self.zone(c.zone).ok().map(|z| self.pile_name(z));
            if let Some(card) = pile.and_then(|n| self.catalog.card(n).ok()) {
                name = format!("{name} on {}", card.name);
            }
            *out.entry(name).or_default() += i64::from(c.value);
        }
        out
    }
    /// The card a neutral zone is named after. The official server names a pile's zone
    /// and leaves a landscape's unnamed: that one is the card lying in it.
    fn pile_name(&self, zone: &model::Zone) -> i32 {
        if zone.pile_name != 0 || !matches!(zone.kind, 7 | 34) {
            return zone.pile_name;
        }
        let lying = self.zones.get(&zone.index).map(|k| &k.known);
        lying
            .and_then(|known| known.keys().find(|n| **n != 0).copied())
            .unwrap_or(0)
    }
    fn named_counts(&self, counts: &BTreeMap<i32, u32>) -> Result<view::CardCount> {
        let mut out = BTreeMap::new();
        for (n, count) in counts {
            *out.entry(self.catalog.card(*n)?.name.clone()).or_default() += count;
        }
        Ok(out)
    }
    fn zone_view(&self, index: i32) -> Result<view::ZoneView> {
        let k = self
            .zones
            .get(&index)
            .ok_or_else(|| invalid("missing projected zone"))?;
        Ok(view::ZoneView {
            count: k.count,
            known: self.named_counts(&k.known)?,
            unknown: k.count - k.known.values().sum::<u32>(),
        })
    }
    pub fn player_name(&self, index: i32) -> String {
        self.state
            .as_ref()
            .and_then(|s| s.players.get(index as usize))
            .and_then(|p| self.names.get(&p.player_id))
            .cloned()
            .unwrap_or_else(|| format!("Player {}", index + 1))
    }
    fn reminder_cards(&self, index: u8, kind: &str) -> Result<Vec<String>> {
        self.state()?.players[index as usize]
            .reminders
            .iter()
            .filter_map(|r| match self.catalog.card(r.card_name) {
                Ok(card) if card.types.iter().any(|t| t == kind) => Some(Ok(card.name.clone())),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect()
    }
    fn projected_zone(&self, index: i32) -> Result<String> {
        let zone = self.zone(index)?;
        Ok(match zone.kind {
            0 => "hand".into(),
            1 => "deck".into(),
            2 => "discard".into(),
            3 => "trash".into(),
            4..=7 | 13 | DRUID_BOONS | MOUSE_CARD | 32 | 34 if zone.owner < 0 => "pile".into(),
            8 => "inPlay".into(),
            _ => zone_key(zone.kind).into(),
        })
    }
    fn player(&self, index: u8) -> Result<view::PlayerView> {
        let state = self.state()?;
        let owner = i32::from(index);
        let mut by_kind: BTreeMap<i32, view::ZoneView> = BTreeMap::new();
        let mut other_zones = BTreeMap::new();
        let mut total = 0;
        let mut located = view::CardCount::new();
        for zone in state.zones.iter().filter(|z| z.owner == owner) {
            let v = self.zone_view(zone.index)?;
            total += v.count;
            for (n, c) in &v.known {
                *located.entry(n.clone()).or_default() += c;
            }
            if matches!(zone.kind, 0 | 1 | 2 | 8) {
                let a = by_kind.entry(zone.kind).or_default();
                a.count += v.count;
                a.unknown += v.unknown;
                for (n, c) in v.known {
                    *a.known.entry(n).or_default() += c;
                }
            } else if v.count > 0 {
                let a: &mut view::ZoneView = other_zones
                    .entry(zone_key(zone.kind).to_string())
                    .or_default();
                a.count += v.count;
                a.unknown += v.unknown;
                for (n, c) in v.known {
                    *a.known.entry(n).or_default() += c;
                }
            }
        }
        let owned = self.named_counts(&self.known.owned[index as usize].0)?;
        let unlocated = owned
            .iter()
            .filter_map(|(n, c)| {
                let remaining = c.saturating_sub(*located.get(n).unwrap_or(&0));
                (remaining > 0).then(|| (n.clone(), remaining))
            })
            .collect();
        let deck = by_kind.remove(&1).unwrap_or_default();
        let count = |kind| {
            state
                .counters
                .iter()
                .find(|c| {
                    c.kind == kind && c.owner == owner && c.zone < 0 && c.card < 0 && c.is_visible
                })
                .map(|c| c.value)
                .unwrap_or_default()
        };
        let token = |kind| {
            state
                .tokens
                .iter()
                .find(|t| t.owner == owner && t.kind == kind)
        };
        Ok(view::PlayerView {
            index,
            name: self.player_name(owner),
            vp: state
                .counters
                .iter()
                .find(|c| c.owner == owner && c.kind == 4 && c.is_visible)
                .map(|c| c.value),
            hand: by_kind.remove(&0).unwrap_or_default(),
            in_play: by_kind.remove(&8).unwrap_or_default(),
            deck: view::DeckView {
                count: deck.count,
                known: deck.known,
                unknown: deck.unknown,
                order_known: false,
            },
            discard: by_kind.remove(&2).unwrap_or_default(),
            other_zones,
            owned: view::Owned {
                unknown: total.saturating_sub(owned.values().sum()),
                known: owned,
            },
            uncertain: self
                .known
                .uncertain
                .iter()
                .filter(|u| {
                    u.zones.iter().all(|(z, _)| {
                        state
                            .zones
                            .iter()
                            .any(|raw| raw.index == *z && raw.owner == owner)
                    })
                })
                .map(|u| {
                    Ok(view::Uncertain {
                        zones: u
                            .zones
                            .iter()
                            .map(|(z, _)| self.projected_zone(*z))
                            .collect::<Result<_>>()?,
                        count: u.zones.iter().map(|(_, n)| *n as u32).sum(),
                        candidates: self.named_counts(&u.candidates)?,
                        outside: u.outside as u32,
                        unknown: u.unknown as u32,
                    })
                })
                .collect::<Result<_>>()?,
            unlocated,
            tokens: view::Tokens {
                coffers: count(5).max(0) as u32,
                villagers: count(12).max(0) as u32,
                favors: count(15).max(0) as u32,
                debt: count(6).max(0) as u32,
                vp_tokens: count(7),
                pirate_ship: count(8).max(0) as u32,
                minus_card: token(7).is_some_and(|t| t.is_flipped),
                minus_coin: token(8).is_some_and(|t| t.is_flipped),
                journey_face_up: token(6).map(|t| !t.is_flipped),
            },
            adventure_tokens: state
                .tokens
                .iter()
                .filter(|t| t.owner == owner && t.kind <= 5)
                .map(|t| {
                    Ok((
                        self.catalog
                            .label("TokenNames", t.kind)
                            .unwrap_or_else(|| format!("token {}", t.kind)),
                        self.zone(t.zone)
                            .ok()
                            .filter(|z| z.pile_name > 0)
                            .map(|z| self.catalog.card(z.pile_name).map(|c| c.name.clone()))
                            .transpose()?,
                    ))
                })
                .collect::<Result<_>>()?,
            projects: state.players[index as usize]
                .bought_projects
                .iter()
                .map(|n| self.catalog.card(*n).map(|c| c.name.clone()))
                .collect::<Result<_>>()?,
            states: self.reminder_cards(index, "State")?,
            boons: self.reminder_cards(index, "Boon")?,
            artifacts: self.reminder_cards(index, "Artifact")?,
            turns_taken: None,
            counters: self.counters(owner),
        })
    }
    pub fn cost(&self, id: Option<i32>, name: i32) -> Result<view::Cost> {
        let info = self.catalog.card(name)?;
        let mut cost = info.cost.clone();
        if info
            .types
            .iter()
            .any(|t| matches!(t.as_str(), "Event" | "Project"))
        {
            return Ok(cost);
        }
        let reduction = &self.state()?.cost_reduction;
        if let Some((_, c)) = reduction.cost_override.iter().find(|(n, _)| *n == name) {
            return Ok(view::Cost {
                coins: nonnegative(c.coins)?,
                potions: nonnegative(c.potions)?,
                debt: nonnegative(c.debt)?,
            });
        }
        let types = id.and_then(|id| self.types.get(&id)).unwrap_or(&info.types);
        let by_type: i32 = reduction
            .by_type
            .iter()
            .filter(|(t, _)| {
                self.catalog.enums["Types"]
                    .get(*t as usize)
                    .is_some_and(|name| types.iter().any(|s| s.eq_ignore_ascii_case(name)))
            })
            .map(|(_, n)| n)
            .sum();
        let by_card: i32 = reduction
            .by_card
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, n)| n)
            .sum();
        cost.coins = (i64::from(cost.coins)
            - i64::from(reduction.generic)
            - i64::from(by_type)
            - i64::from(by_card))
        .max(0)
        .try_into()
        .map_err(|_| invalid("cost overflow"))?;
        Ok(cost)
    }
    pub fn render_argument(&self, arg: &LogArgument) -> Option<String> {
        Some(match arg {
            LogArgument::CardNames(v) => v
                .iter()
                .map(|c| {
                    self.catalog
                        .card(c.name)
                        .ok()
                        .map(|n| format!("{} {}", c.frequency, n.name))
                })
                .collect::<Option<Vec<_>>>()?
                .join(", "),
            LogArgument::Player(p) => self.player_name(*p),
            LogArgument::Players(v) => v
                .iter()
                .map(|p| self.player_name(*p))
                .collect::<Vec<_>>()
                .join(", "),
            LogArgument::Number(n) => n.to_string(),
            LogArgument::Card(n) => self.card_name(*n)?,
            LogArgument::SingularCard(n) => self.catalog.card(*n).ok()?.name.clone(),
            LogArgument::Zone(n) => self.zone_name(*n)?,
            LogArgument::OwnZone { player, zone } => {
                format!("{}'s {}", self.player_name(*player), zone_label(*zone))
            }
            LogArgument::Amount {
                coins,
                potions,
                debt,
            } => format!("${coins}, {potions} Potions, {debt} Debt"),
            LogArgument::Counter(n) => self.catalog.label("CounterNames", *n)?,
            LogArgument::Token(n) => self.catalog.label("TokenNames", *n)?,
            LogArgument::Turn { owner, number, .. } => {
                format!("{} turn {number}", self.player_name(*owner))
            }
            LogArgument::Ability(a) => self.catalog.label("AbilityNames", a.name)?,
            LogArgument::DirectionalZone { direction, zone } => format!(
                "{} of {}",
                if *direction == 0 { "from" } else { "to" },
                self.zone_name(*zone)?
            ),
            LogArgument::Event { ability, .. } => {
                self.catalog.label("AbilityNames", ability.name)?
            }
            LogArgument::MetaGame(m) => format!("game {}", m.game_id),
        })
    }
    /// Replaces the log from `change.start_index` on, and learns what the entries say of
    /// cards that no event shows.
    pub fn log(&mut self, change: &model::LogChange) -> Result<()> {
        let start =
            usize::try_from(change.start_index).map_err(|_| invalid("negative log index"))?;
        self.logs.retain(|e| e.seq < start);
        for (offset, entry) in change.entries.iter().enumerate() {
            let line = self
                .log_entry(start + offset, entry)
                .ok_or_else(|| invalid("unknown or unsupported official log entry"))?;
            self.logs.push(line);
            self.learn(entry);
        }
        Ok(())
    }
    /// Before the first turn every card a player has is one they are said to start with.
    fn learn(&mut self, entry: &model::LogEntry) {
        let model::LogEntry::Named {
            name, arguments, ..
        } = entry
        else {
            return;
        };
        let key = usize::try_from(*name)
            .ok()
            .and_then(|i| self.catalog.enums.get("LogEntryNames")?.get(i));
        let (Some("STARTS_WITH"), [LogArgument::Player(player), LogArgument::CardNames(cards)]) =
            (key.map(String::as_str), arguments.as_slice())
        else {
            return;
        };
        let Some(state) = self.state.as_ref().filter(|s| s.turn.turn_number == 0) else {
            return;
        };
        let zones: Vec<i32> = state
            .zones
            .iter()
            .filter(|z| z.owner == *player)
            .map(|z| z.index)
            .collect();
        for card in cards {
            if card.name > 0
                && self.catalog.card(card.name).is_ok()
                && let Ok(count) = u32::try_from(card.frequency)
            {
                self.knowledge.declare(&zones, card.name, count);
            }
        }
        self.refresh_knowledge();
    }
    /// An official log entry as agents and watchers get it: a turn header is `turn-start`
    /// ("Ann starts turn 3", an extra turn's card as `card`), another named entry
    /// `official-<name>` with the player it starts with. None for an unknown entry.
    pub fn log_entry(&self, seq: usize, entry: &model::LogEntry) -> Option<view::LogEntry> {
        let (kind, player, card, text) = match entry {
            model::LogEntry::Named {
                name, arguments, ..
            } => {
                let key = self
                    .catalog
                    .enums
                    .get("LogEntryNames")?
                    .get(*name as usize)?;
                match (key.as_str(), arguments.as_slice()) {
                    ("NEW_TURN", [LogArgument::Turn { kind: 1, .. }]) => {
                        ("between-turns".into(), None, None, "Between turns".into())
                    }
                    (
                        "NEW_TURN",
                        [
                            LogArgument::Turn {
                                owner,
                                number,
                                kind,
                                ..
                            },
                        ],
                    ) => {
                        let card = (*kind > 1)
                            .then(|| {
                                let key = self
                                    .catalog
                                    .enums
                                    .get("ExtraTurnTypes")?
                                    .get(*kind as usize)?;
                                Some(
                                    self.catalog
                                        .card(self.catalog.ordinal(key)?)
                                        .ok()?
                                        .name
                                        .clone(),
                                )
                            })
                            .flatten();
                        (
                            "turn-start".into(),
                            u8::try_from(*owner).ok(),
                            card,
                            format!("{} starts turn {number}", self.player_name(*owner)),
                        )
                    }
                    _ => {
                        let text = self.render_template(
                            self.catalog.label("LogEntryNames", *name)?,
                            arguments,
                        )?;
                        let player = match arguments.first() {
                            Some(LogArgument::Player(p)) => u8::try_from(*p).ok(),
                            _ => None,
                        };
                        let mut cards = arguments.iter().flat_map(|a| match a {
                            LogArgument::CardNames(v) => {
                                v.iter().map(|c| (c.name, c.frequency)).collect()
                            }
                            LogArgument::SingularCard(n) => vec![(*n, 1)],
                            _ => vec![],
                        });
                        let card = match (cards.next(), cards.next()) {
                            (Some((n, 1)), None) => {
                                self.catalog.card(n).ok().map(|c| c.name.clone())
                            }
                            _ => None,
                        };
                        (
                            format!("official-{}", key.to_lowercase().replace('_', "-")),
                            player,
                            card,
                            text,
                        )
                    }
                }
            }
            model::LogEntry::Decision {
                player, autoplayed, ..
            } => (
                "decision".into(),
                u8::try_from(*player).ok(),
                None,
                format!(
                    "{} {}",
                    self.player_name(*player),
                    if *autoplayed {
                        "resolved a forced decision"
                    } else {
                        "answered"
                    }
                ),
            ),
        };
        Some(view::LogEntry {
            seq,
            kind,
            player,
            card,
            text,
        })
    }
    pub fn render_template(&self, template: String, args: &[LogArgument]) -> Option<String> {
        let mut result = String::new();
        let mut remaining = template.as_str();
        while !remaining.is_empty() {
            let token = ["Argument", "NUMBER", "PLAYER", "CARDS", "AMOUNT"]
                .iter()
                .find_map(|prefix| {
                    let rest = remaining.strip_prefix(prefix)?;
                    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                    (digits > 0).then_some((prefix.len(), digits))
                });
            if let Some((prefix, digits)) = token {
                let index = remaining[prefix..prefix + digits].parse::<usize>().ok()?;
                result.push_str(&self.render_argument(args.get(index)?)?);
                remaining = &remaining[prefix + digits..];
            } else {
                let ch = remaining.chars().next()?;
                result.push(ch);
                remaining = &remaining[ch.len_utf8()..];
            }
        }
        Some(result)
    }
}
impl QuestionContext for Reducer {
    fn card_name(&self, id: i32) -> Option<String> {
        if !self.visible_ids.contains(&id) && !self.question_visible.contains(&id) {
            return None;
        }
        self.disclosed(id)
            .ok()
            .flatten()
            .and_then(|n| self.catalog.card(n).ok())
            .map(|c| c.name.clone())
    }
    fn catalog_name(&self, ordinal: i32) -> Option<String> {
        self.catalog.card(ordinal).ok().map(|c| c.name.clone())
    }
    fn catalog_ordinal(&self, name: &str) -> Option<i32> {
        let ordinal = self.catalog.ordinal(name)?;
        self.name_choices()
            .iter()
            .any(|(id, _)| *id == ordinal)
            .then_some(ordinal)
    }
    fn zone_name(&self, index: i32) -> Option<String> {
        self.zone(index)
            .ok()
            .map(|z| format!("{} {}", self.player_name(z.owner), zone_label(z.kind)))
    }
    fn enum_label(&self, catalog: &str, ordinal: i32) -> Option<String> {
        self.catalog.label(catalog, ordinal)
    }
    fn story_text(&self, story: &Story) -> Option<String> {
        if let Some(context) = &story.context {
            return self.render_argument(context);
        }
        self.render_template(
            self.catalog.label("QuestionIds", story.question_id)?,
            &story.arguments,
        )
    }
    fn ability_text(&self, a: &crate::questions::Ability) -> Option<String> {
        let catalog = if a.description_type == 0 {
            "MovementTypes"
        } else {
            "AbilityNames"
        };
        let template = if a.description_type == 0 {
            self.catalog.label(catalog, a.name)
        } else {
            self.catalog
                .label("AbilityDescriptions", a.name)
                .or_else(|| self.catalog.label(catalog, a.name))
        }?;
        let text = self.render_template(template, &a.arguments)?;
        let arguments = a
            .arguments
            .iter()
            .map(|arg| self.render_argument(arg))
            .collect::<Option<Vec<_>>>()?;
        Some(
            if arguments.is_empty() || text.contains(&arguments.join(", ")) {
                text
            } else {
                format!("{text} ({})", arguments.join(", "))
            },
        )
    }
    fn name_choices(&self) -> Vec<(i32, String)> {
        self.catalog
            .cards
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                !c.types.is_empty()
                    && !c.types.iter().any(|t| {
                        matches!(
                            t.as_str(),
                            "Randomizer"
                                | "Event"
                                | "Landmark"
                                | "Project"
                                | "Way"
                                | "Ally"
                                | "Trait"
                                | "Prophecy"
                                | "State"
                                | "Artifact"
                                | "Boon"
                                | "Hex"
                        )
                    })
            })
            .map(|(i, c)| (i as i32, c.name.clone()))
            .collect()
    }
    fn card_cost(&self, id: i32) -> Option<view::Cost> {
        self.cost(Some(id), self.raw_name(id).ok()?).ok()
    }
    fn card_zone(&self, id: i32) -> Option<String> {
        self.zones
            .iter()
            .find(|(_, k)| k.exact.contains_key(&id))
            .and_then(|(index, _)| self.zone(*index).ok())
            .map(|z| zone_key(z.kind).into())
    }
}
fn setup_set_aside(source: &str, card: String) -> view::NoteView {
    view::NoteView {
        source: source.into(),
        player: None,
        pile: None,
        card: Some(card),
        count: 0,
        setup: Some(view::SetupRole::SetAside),
    }
}

fn title(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
    }
}
/// A zone's name in a view, one of `dominion_protocol::contract`'s: cards being revealed
/// or looked at are `looking`, and those the official client keeps out of sight for a
/// question are set aside.
/// The official client's `CounterNames.BLOCKADE_TOKENS`.
const BLOCKADE_TOKENS: i32 = 14;

fn zone_key(kind: i32) -> &'static str {
    match kind {
        4..=7 | 13 | DRUID_BOONS | MOUSE_CARD | 32 | 34 => "pile",
        23 | 25 | 29 => "looking",
        9 | 14..=19 | THE_BOX | 33 => "setAside",
        _ => zone_label(kind),
    }
}

fn zone_label(kind: i32) -> &'static str {
    match kind {
        0 => "hand",
        1 => "deck",
        2 => "discard",
        3 => "trash",
        4..=6 => "supply",
        7 | 34 => "landscape",
        8 => "inPlay",
        10 => "tavern",
        11 => "nativeVillage",
        12 => "island",
        13 | 32 => "nonSupply",
        DRUID_BOONS => "druidBoons",
        21 => "exile",
        MOUSE_CARD => "wayOfTheMouse",
        23 | 25 | 29 => "revealed",
        24 | 26..=28 | 31 => "setAside",
        THE_BOX => "box",
        _ => "hidden",
    }
}

#[cfg(test)]
mod tests;

fn reveal_question(q: &Question, ids: &mut BTreeSet<i32>) {
    use crate::questions::{QuestionElements, QuestionKind};
    if q.description.association >= 0 {
        ids.insert(q.description.association);
    }
    match &q.kind {
        QuestionKind::Complex(children) => {
            for child in children {
                reveal_question(child, ids);
            }
        }
        QuestionKind::Choice { elements, .. } => match elements {
            QuestionElements::Cards(v) => ids.extend(v.iter().copied().filter(|id| *id >= 0)),
            QuestionElements::BuyableCards(v) => {
                ids.extend(v.iter().map(|(id, _)| *id).filter(|id| *id >= 0))
            }
            QuestionElements::ExplainedCards(v) => {
                ids.extend(v.iter().map(|(id, _, _)| *id).filter(|id| *id >= 0))
            }
            QuestionElements::Abilities(v) => {
                ids.extend(v.iter().map(|a| a.association).filter(|id| *id >= 0))
            }
            _ => {}
        },
        _ => {}
    }
}

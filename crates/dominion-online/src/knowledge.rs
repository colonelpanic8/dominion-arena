//! Translates observed wire identities into private bookkeeping slots. Hidden allocation
//! IDs never enter the tracker; a mix severs every observed ID-to-slot association.
use std::collections::BTreeMap;

use dominion_knowledge::{KnownCards, Mind};

use crate::{Error, Result};

#[derive(Clone, Debug, Default)]
pub(super) struct KnowledgeZone {
    pub count: u32,
    pub known: BTreeMap<i32, u32>,
    pub exact: BTreeMap<i32, i32>,
}

#[derive(Clone)]
pub(super) struct Observations {
    mind: Mind<i32, usize, i32>,
    members: BTreeMap<i32, Vec<usize>>,
    locations: Vec<i32>,
    observed: BTreeMap<i32, usize>,
}

impl Default for Observations {
    fn default() -> Self {
        Self {
            mind: Mind::with_unknown(0),
            members: BTreeMap::new(),
            locations: vec![],
            observed: BTreeMap::new(),
        }
    }
}

/// Where the cards a count correction takes out of a zone are.
const NOWHERE: i32 = i32::MIN;

fn invalid(message: &str) -> Error {
    Error::Protocol(message.into())
}

impl Observations {
    pub fn create_zone(&mut self, zone: i32) {
        self.members.entry(zone).or_default();
    }

    pub fn destroy_zone(&mut self, zone: i32) {
        self.members.remove(&zone);
    }

    pub fn insert(&mut self, zone: i32, id: i32, name: Option<i32>) {
        let slot = self.locations.len();
        self.locations.push(zone);
        self.members.entry(zone).or_default().push(slot);
        self.mind
            .new_group(vec![slot], Some(BTreeMap::from([(name.unwrap_or(0), 1)])));
        if id >= 0 && name.is_some() {
            self.observed.insert(id, slot);
        }
    }

    pub fn count(&self, zone: i32) -> usize {
        self.members.get(&zone).map_or(0, Vec::len)
    }

    fn select(
        &self,
        zone: i32,
        id: i32,
        name: Option<i32>,
        excluded: &[usize],
    ) -> Result<(usize, bool)> {
        if let Some(&slot) = self.observed.get(&id)
            && self.locations[slot] == zone
            && !excluded.contains(&slot)
        {
            if name.is_some_and(|n| !self.mind.may_be(slot, n)) {
                return Err(invalid("observed copy changed identity without a rename"));
            }
            return Ok((slot, true));
        }
        self.candidates(zone, id, excluded)
            .into_iter()
            .find(|slot| name.is_none_or(|n| self.mind.may_be(*slot, n)))
            .map(|slot| (slot, false))
            .ok_or_else(|| invalid("observation contradicts zone knowledge"))
    }

    fn candidates(&self, zone: i32, id: i32, excluded: &[usize]) -> Vec<usize> {
        self.members
            .get(&zone)
            .into_iter()
            .flatten()
            .copied()
            .filter(|slot| {
                !excluded.contains(slot)
                    && (id < 0
                        || !self
                            .observed
                            .iter()
                            .any(|(other, s)| *other != id && s == slot))
            })
            .collect()
    }

    /// Simultaneous reveals are distinct copies; separate inspections may show the same one.
    pub fn reveal(&mut self, zone: i32, cards: &[(i32, i32)]) -> Result<()> {
        if !self.members.contains_key(&zone) {
            return Err(invalid("inspection of unknown zone"));
        }
        self.mind.settle();
        let mut seen = vec![];
        for &(id, name) in cards {
            let (slot, followed) = self.select(zone, id, Some(name), &seen)?;
            let among = self.candidates(zone, id, &seen);
            self.mind
                .see(slot, name, zone, (!followed).then_some(among.as_slice()));
            self.mind.remember(slot);
            seen.push(slot);
            if id >= 0 {
                self.observed.insert(id, slot);
            }
        }
        self.finish();
        Ok(())
    }

    pub fn move_cards(
        &mut self,
        from: i32,
        to: i32,
        cards: &[(i32, i32, Option<i32>)],
    ) -> Result<()> {
        let mut selected = vec![];
        for &(before, after, name) in cards {
            let id = if before >= 0 { before } else { after };
            let (slot, followed) = self.select(from, id, name, &selected)?;
            let among = self.candidates(from, id, &selected);
            selected.push(slot);
            if let Some(name) = name {
                self.mind
                    .see(slot, name, from, (!followed).then_some(among.as_slice()));
            } else {
                self.mind
                    .lose(slot, from, to, (!followed).then_some(among.as_slice()));
            }
            // Without a public copy identity this could have been any source card.
            if !followed {
                self.observed.retain(|_, s| !among.contains(s));
            }
            self.observed.retain(|_, s| *s != slot);
            self.members.get_mut(&from).unwrap().retain(|s| *s != slot);
            self.members.entry(to).or_default().push(slot);
            self.locations[slot] = to;
            if after >= 0 && name.is_some() {
                self.observed.insert(after, slot);
            }
        }
        self.finish();
        Ok(())
    }

    pub fn shuffle(&mut self, deck: i32, discard: Option<i32>) {
        self.mind.settle();
        if let Some(discard) = discard {
            for slot in std::mem::take(self.members.entry(discard).or_default()) {
                self.locations[slot] = deck;
                self.members.entry(deck).or_default().push(slot);
            }
        }
        let slots = self.members.get(&deck).cloned().unwrap_or_default();
        self.mind.mix(&slots);
        self.observed.retain(|_, slot| !slots.contains(slot));
        self.finish();
    }

    /// A count correction has no movement evidence: missing cards join the zone unknown,
    /// and surplus ones, which could be any of its cards, leave it for no zone. What is
    /// known of every other zone stays.
    pub fn reconcile(&mut self, zone: i32, count: u32) {
        let (have, count) = (self.count(zone), count as usize);
        if have == count {
            return;
        }
        self.mind.settle();
        if have < count {
            for _ in have..count {
                self.insert(zone, -1, None);
            }
        } else {
            let slots = self.members[&zone].clone();
            if count > 0 {
                self.mind.mix(&slots);
            }
            self.observed.retain(|_, slot| !slots.contains(slot));
            for slot in self.members.get_mut(&zone).unwrap().split_off(count) {
                self.locations[slot] = NOWHERE;
            }
        }
        self.finish();
    }

    /// `count` of the cards in `zones` are `name`, whichever of them those are.
    pub fn declare(&mut self, zones: &[i32], name: i32, count: u32) {
        let slots: Vec<usize> = zones
            .iter()
            .flat_map(|zone| self.members.get(zone).into_iter().flatten().copied())
            .collect();
        self.mind.settle();
        self.mind.learn(&slots, name, count);
        self.finish();
    }

    pub fn rename(&mut self, id: i32, name: i32) {
        if let Some(&slot) = self.observed.get(&id)
            && let Some(old) = self.mind.identity(slot)
        {
            self.mind.change(slot, old, name);
        }
    }

    pub fn forget(&mut self, id: i32) {
        self.observed.remove(&id);
    }

    fn finish(&mut self) {
        self.mind.settle();
        self.mind.reduce();
        self.mind.clear_recent();
    }

    pub fn summary(
        &self,
        players: usize,
        owner: impl Fn(i32) -> Option<usize>,
    ) -> KnownCards<i32, i32> {
        let owner = |zone| (zone != NOWHERE).then(|| owner(zone)).flatten();
        let mut known = self
            .mind
            .summarize(players, |s| self.locations[s], owner, |_| None);
        known.zones.remove(&NOWHERE);
        for u in &mut known.uncertain {
            if let Some(i) = u.zones.iter().position(|(z, _)| *z == NOWHERE) {
                u.outside += u.zones.remove(i).1;
            }
        }
        known.uncertain.retain(|u| u.zones.len() > 1);
        known
    }

    /// The cards of `zone` in the order they arrived: the wire identity of each one
    /// still followed, and its name if known.
    pub fn ordered(&self, zone: i32) -> Vec<(Option<i32>, Option<i32>)> {
        let ids: BTreeMap<usize, i32> = self.observed.iter().map(|(&id, &s)| (s, id)).collect();
        self.members
            .get(&zone)
            .into_iter()
            .flatten()
            .map(|slot| (ids.get(slot).copied(), self.mind.identity(*slot)))
            .collect()
    }

    pub fn zones(&self, summary: &KnownCards<i32, i32>) -> BTreeMap<i32, KnowledgeZone> {
        self.members
            .keys()
            .map(|&z| {
                let known = summary.zone(z);
                let exact = self
                    .observed
                    .iter()
                    .filter_map(|(&id, &slot)| {
                        (self.locations[slot] == z)
                            .then(|| self.mind.identity(slot).map(|name| (id, name)))
                            .flatten()
                    })
                    .collect();
                (
                    z,
                    KnowledgeZone {
                        count: known.count as u32,
                        known: known.known,
                        exact,
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COPPER: i32 = 1;
    const SILVER: i32 = 2;
    const HAND: i32 = 0;
    const DECK: i32 = 1;
    const DISCARD: i32 = 2;
    const HEXES: i32 = 3;

    /// A player whose discard pile is 3 Copper, 2 Silver and a card never seen, shuffled
    /// into their empty deck, from which they draw 2 cards unseen.
    fn reshuffled() -> Observations {
        let mut t = Observations::default();
        for zone in [HAND, DECK, DISCARD] {
            t.create_zone(zone);
        }
        for (id, name) in [COPPER, COPPER, COPPER, SILVER, SILVER]
            .into_iter()
            .enumerate()
        {
            t.insert(DISCARD, id as i32, Some(name));
        }
        t.insert(DISCARD, -1, None);
        t.shuffle(DECK, Some(DISCARD));
        t.move_cards(DECK, HAND, &[(-1, -1, None), (-1, -1, None)])
            .unwrap();
        t
    }

    fn owner(zone: i32) -> Option<usize> {
        (zone != HEXES).then_some(0)
    }

    #[test]
    fn a_reshuffle_keeps_the_discard_piles_cards_as_the_decks_and_hands() {
        let known = reshuffled().summary(1, owner);
        assert_eq!(
            known.uncertain,
            vec![dominion_knowledge::UncertainCards {
                zones: vec![(HAND, 2), (DECK, 4)],
                candidates: BTreeMap::from([(COPPER, 3), (SILVER, 2)]),
                outside: 0,
                unknown: 1,
            }]
        );
        assert_eq!(
            known.owned[0],
            (BTreeMap::from([(COPPER, 3), (SILVER, 2)]), 1)
        );
    }

    #[test]
    fn cards_a_player_is_said_to_have_are_followed_from_the_opening_shuffle() {
        let mut t = Observations::default();
        for zone in [HAND, DECK, DISCARD] {
            t.create_zone(zone);
        }
        for _ in 0..5 {
            t.insert(HAND, -1, None);
        }
        t.declare(&[HAND, DECK, DISCARD], COPPER, 3);
        t.declare(&[HAND, DECK, DISCARD], SILVER, 2);
        t.move_cards(HAND, DECK, &[(-1, -1, None); 5]).unwrap();
        t.shuffle(DECK, None);
        t.move_cards(DECK, HAND, &[(-1, -1, None); 3]).unwrap();
        t.move_cards(
            HAND,
            DISCARD,
            &[(-1, 7, Some(SILVER)), (-1, 8, Some(SILVER))],
        )
        .unwrap();
        let known = t.summary(1, owner);
        assert_eq!(known.zone(HAND).known, BTreeMap::from([(COPPER, 1)]));
        assert_eq!(known.zone(DECK).known, BTreeMap::from([(COPPER, 2)]));
        // Saying it again, or of more cards than are unknown, changes nothing.
        t.declare(&[HAND, DECK, DISCARD], COPPER, 3);
        t.declare(&[HAND, DECK, DISCARD], SILVER, 4);
        assert_eq!(t.summary(1, owner), known);
    }

    #[test]
    fn a_count_correction_keeps_what_is_known_of_other_zones() {
        let mut t = reshuffled();
        t.create_zone(HEXES);
        for _ in 0..3 {
            t.insert(HEXES, -1, None);
        }
        let before = t.summary(1, owner);
        t.reconcile(HEXES, 0);
        let empty = t.summary(1, owner);
        assert_eq!(empty.zone(HEXES).count, 0);
        assert_eq!(
            (&empty.uncertain, &empty.owned),
            (&before.uncertain, &before.owned)
        );
        t.reconcile(HEXES, 3);
        assert_eq!(t.summary(1, owner), before);
    }

    #[test]
    fn a_zone_corrected_down_may_have_lost_any_of_its_cards() {
        let mut t = Observations::default();
        for zone in [HAND, DISCARD] {
            t.create_zone(zone);
        }
        t.insert(HAND, 0, Some(COPPER));
        for (id, name) in [(1, COPPER), (2, COPPER), (3, SILVER)] {
            t.insert(DISCARD, id, Some(name));
        }
        t.reconcile(DISCARD, 2);
        let known = t.summary(1, owner);
        assert_eq!(known.zone(HAND).known, BTreeMap::from([(COPPER, 1)]));
        let discard = known.zone(DISCARD);
        assert_eq!((discard.count, discard.unknown), (2, 1));
        assert_eq!(discard.known, BTreeMap::from([(COPPER, 1)]));
        assert!(known.uncertain.is_empty());
        assert!(t.zones(&known)[&DISCARD].exact.is_empty());
    }

    #[test]
    fn partial_observations_remain_true_through_hidden_moves_and_reveals() {
        for seed in 0..200_u64 {
            let mut random = seed + 1;
            let mut pick = |n: usize| {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                (random >> 32) as usize % n
            };
            let names = [1, 1, 2, 2, 3, 3];
            let mut actual = [vec![0, 1, 2], vec![3, 4, 5], vec![]];
            let mut tracker = Observations::default();
            for (z, cards) in actual.iter().enumerate() {
                tracker.create_zone(z as i32);
                for &id in cards {
                    let seen = pick(2) == 0;
                    tracker.insert(
                        z as i32,
                        if seen { id as i32 } else { -1 },
                        seen.then_some(names[id]),
                    );
                }
            }
            for step in 0..150 {
                let from = pick(3);
                if actual[from].is_empty() {
                    continue;
                }
                let position = pick(actual[from].len());
                let id = actual[from][position];
                let action = pick(4);
                let result = match action {
                    0 => {
                        tracker.shuffle(from as i32, None);
                        Ok(())
                    }
                    1 => tracker.reveal(from as i32, &[(id as i32, names[id])]),
                    action => {
                        let to = (from + 1 + pick(2)) % 3;
                        let seen = action == 2;
                        actual[from].remove(position);
                        actual[to].push(id);
                        let wire = if seen { id as i32 } else { -1 };
                        tracker.move_cards(
                            from as i32,
                            to as i32,
                            &[(wire, wire, seen.then_some(names[id]))],
                        )
                    }
                };
                assert!(result.is_ok(), "seed {seed} step {step}: {result:?}");
                let known = tracker.summary(2, |z| Some(usize::from(z == 2)));
                for (z, cards) in actual.iter().enumerate() {
                    let k = known.zone(z as i32);
                    assert_eq!(k.count, cards.len(), "seed {seed} step {step}");
                    assert_eq!(k.known.values().sum::<u32>() as usize + k.unknown, k.count);
                    for (name, n) in k.known {
                        assert!(
                            cards.iter().filter(|&&id| names[id] == name).count() >= n as usize,
                            "seed {seed} step {step}: zone {z} claims {n} of {name}, actual {cards:?}"
                        );
                    }
                }
                for (p, (owned, unknown)) in known.owned.iter().enumerate() {
                    let cards: Vec<_> = actual
                        .iter()
                        .enumerate()
                        .filter(|(z, _)| usize::from(*z == 2) == p)
                        .flat_map(|(_, c)| c)
                        .collect();
                    assert_eq!(owned.values().sum::<u32>() as usize + unknown, cards.len());
                    for (&name, &n) in owned {
                        assert!(
                            cards.iter().filter(|&&&id| names[id] == name).count() >= n as usize,
                            "seed {seed} step {step}: owner {p} claims {n} of {name}"
                        );
                    }
                }
            }
        }
    }
}

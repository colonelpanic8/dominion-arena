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

    /// Count corrections contain no movement evidence. Rebuild conservative bounds and
    /// discard correlations instead of assigning the missing cards to an invented zone.
    pub fn reconcile(&mut self, zone: i32, count: u32) {
        if self.count(zone) == count as usize {
            return;
        }
        self.finish();
        let summary = self
            .mind
            .summarize(0, |s| self.locations[s], |_| None, |_| None);
        let mut zones: BTreeMap<_, _> = self
            .members
            .keys()
            .map(|z| (*z, summary.zone(*z)))
            .collect();
        let target = zones.entry(zone).or_default();
        let lost = target.count.saturating_sub(count as usize) as u32;
        target.count = count as usize;
        for n in target.known.values_mut() {
            *n = n.saturating_sub(lost);
        }
        target.known.retain(|_, n| *n > 0);
        let exact: Vec<_> = self
            .observed
            .iter()
            .filter_map(|(&id, &slot)| {
                (self.locations[slot] != zone)
                    .then(|| {
                        self.mind
                            .identity(slot)
                            .map(|name| (self.locations[slot], id, name))
                    })
                    .flatten()
            })
            .collect();
        *self = Self::default();
        for (z, knowledge) in zones {
            self.create_zone(z);
            let mut remaining = knowledge.known;
            let mut added = 0;
            for &(_, id, name) in exact.iter().filter(|(zone, _, _)| *zone == z) {
                if let Some(n) = remaining.get_mut(&name)
                    && *n > 0
                {
                    self.insert(z, id, Some(name));
                    *n -= 1;
                    added += 1;
                }
            }
            let first_anonymous = self.locations.len();
            for (name, n) in std::mem::take(&mut remaining) {
                for _ in 0..n {
                    self.insert(z, -1, Some(name));
                    added += 1;
                }
            }
            for _ in added..knowledge.count {
                self.insert(z, -1, None);
            }
            let anonymous: Vec<_> = (first_anonymous..self.locations.len()).collect();
            self.mind.mix(&anonymous);
        }
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
        self.mind
            .summarize(players, |s| self.locations[s], owner, |_| None)
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

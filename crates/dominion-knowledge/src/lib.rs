//! Observation-driven card identity constraints shared by the local and official backends.
//! Adapters supply only observed identities and specify when copies become indistinguishable.
//! Instance handles are opaque bookkeeping, never evidence of a card's identity.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
pub type Counts<C> = BTreeMap<C, u32>;

pub trait Instance: Copy + Eq + std::hash::Hash + std::fmt::Debug {
    fn index(self) -> usize;
}
impl Instance for usize {
    fn index(self) -> usize {
        self
    }
}
pub trait Location: Copy + Ord + std::hash::Hash {
    fn is_hand(self) -> bool {
        false
    }
    fn is_discard(self) -> bool {
        false
    }
}
impl Location for i32 {}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(
    serialize = "C: Serialize, I: Serialize, Z: Serialize",
    deserialize = "C: Deserialize<'de>, I: Deserialize<'de>, Z: Deserialize<'de>"
))]
pub struct Mind<C: Ord, I: Eq + std::hash::Hash, Z> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unknown: Option<C>,
    group_of: Vec<usize>,
    groups: Vec<Group<C, I>>,
    #[serde(default)]
    families: Vec<Family<C>>,
    /// Cards shown to this viewer since the last decision that haven't moved since: they
    /// can follow them.
    recent: HashSet<I>,
    #[serde(default)]
    leaving: Option<Leaving<I, Z>>,
    /// Hands discarded whole unseen: their identities arrived together, their positions
    /// within the discard pile are unknown.
    #[serde(default)]
    arrived: Vec<Vec<I>>,
}

/// Cards a viewer watched leave a held set for one place without seeing which they were.
/// Until the viewer knows where the rest of the set goes, the groups stay as they were.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Leaving<I, Z> {
    from: Z,
    to: Z,
    left: Vec<I>,
    /// The cards still in the set.
    rest: Vec<I>,
    /// Cards of the set the viewer has seen since.
    seen: Vec<I>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Group<C: Ord, I> {
    cards: Vec<I>,
    /// `None` when the viewer doesn't know which cards these are.
    ids: Option<Counts<C>>,
    /// How many of `ids` are not these cards: the cards are some of `ids`, and the others
    /// are in the other groups of its family.
    #[serde(default)]
    extra: u32,
    #[serde(default)]
    family: Option<usize>,
}

/// Groups split from one the viewer knew the cards of, when only some of its cards
/// became indistinguishable from others (some left a set held out of sight): together
/// they are exactly `ids`, each of them some of its own `ids`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Family<C: Ord> {
    ids: Option<Counts<C>>,
    groups: Vec<usize>,
}

/// One group of indistinguishable cards, as one viewer knows it.
#[derive(Clone, Copy, Debug)]
pub struct GroupView<'a, C, I> {
    pub cards: &'a [I],
    /// Their identities as a multiset, if the viewer knows it: with `extra` more than
    /// there are cards, which of them the cards are.
    pub ids: Option<&'a Counts<C>>,
    pub extra: u32,
}

/// What a viewer knows about the cards in one zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneKnowledge<C> {
    pub count: usize,
    pub known: Counts<C>,
    pub unknown: usize,
}

/// Cards of a group spread over several zones: together they are `candidates`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncertainCards<C, Z> {
    /// Each zone and how many of the cards are there.
    pub zones: Vec<(Z, usize)>,
    pub candidates: Counts<C>,
    /// How many of the candidates are not these cards but are elsewhere.
    pub outside: usize,
    /// How many more candidates there are whose identity the viewer doesn't know.
    pub unknown: usize,
}

/// A viewer's knowledge of every zone, as zone summaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownCards<C, Z: Eq + std::hash::Hash> {
    pub zones: HashMap<Z, ZoneKnowledge<C>>,
    pub uncertain: Vec<UncertainCards<C, Z>>,
    /// Per player: the identities of cards they own that the viewer knows, wherever they
    /// are among that player's zones, and how many owned cards are unknown.
    pub owned: Vec<(Counts<C>, usize)>,
}

impl<C: Clone, Z: Copy + Eq + std::hash::Hash> KnownCards<C, Z> {
    pub fn zone(&self, zone: Z) -> ZoneKnowledge<C> {
        self.zones.get(&zone).cloned().unwrap_or_default()
    }
}

fn add<C: Ord>(count: &mut Counts<C>, card: C, n: u32) {
    *count.entry(card).or_default() += n;
}

fn add_all<C: Copy + Ord>(to: &mut Option<Counts<C>>, from: Option<&Counts<C>>) {
    match (to.as_mut(), from) {
        (Some(a), Some(b)) => b.iter().for_each(|(&c, &n)| add(a, c, n)),
        _ => *to = None,
    }
}

fn remove<C: Ord>(count: &mut Counts<C>, card: C) -> bool {
    match count.get_mut(&card) {
        Some(n) if *n > 1 => *n -= 1,
        Some(_) => {
            count.remove(&card);
        }
        None => return false,
    }
    true
}

impl<C: Copy + Ord + std::fmt::Debug, I: Instance, Z: Location> Mind<C, I, Z> {
    pub fn views(&self) -> impl Iterator<Item = GroupView<'_, C, I>> {
        self.groups
            .iter()
            .filter(|g| !g.cards.is_empty())
            .map(|g| GroupView {
                cards: &g.cards,
                ids: g.ids.as_ref(),
                extra: g.extra,
            })
    }

    /// Reserve a symbol for an identity not yet observed (never a real card name).
    pub fn with_unknown(unknown: C) -> Self {
        Self {
            unknown: Some(unknown),
            ..Self::default()
        }
    }

    pub fn may_be(&self, card: I, name: C) -> bool {
        let group = &self.groups[self.group_of[card.index()]];
        let allows = |name| {
            group.ids.as_ref().is_none_or(|ids| ids.contains_key(&name))
                && group
                    .family
                    .and_then(|f| self.families[f].ids.as_ref())
                    .is_none_or(|ids| ids.contains_key(&name))
        };
        allows(name) || self.unknown.is_some_and(allows)
    }

    // An unknown identity can equal a named candidate. Keeping subset constraints
    // would incorrectly consume the named copy when revealing an unknown copy of it.
    fn merge_partial_families(&mut self) {
        let Some(unknown) = self.unknown else {
            return;
        };
        let sets: Vec<Vec<I>> = self
            .families
            .iter()
            .filter(|f| f.ids.as_ref().is_some_and(|ids| ids.contains_key(&unknown)))
            .map(|f| {
                f.groups
                    .iter()
                    .flat_map(|&g| self.groups[g].cards.iter().copied())
                    .collect()
            })
            .collect();
        for cards in sets {
            self.merge(cards);
        }
    }

    fn tighten(&mut self) {
        for group in &mut self.groups {
            if let Some(ids) = &mut group.ids
                && let Some(all) = group.family.and_then(|f| self.families[f].ids.as_ref())
            {
                for (name, n) in ids.iter_mut() {
                    let most = all.get(name).copied().unwrap_or(0);
                    group.extra -= n.saturating_sub(most);
                    *n = (*n).min(most);
                }
                ids.retain(|_, n| *n > 0);
            }
        }
    }

    pub fn identity(&self, card: I) -> Option<C> {
        let ids = self.groups[self.group_of[card.index()]].ids.as_ref()?;
        (ids.len() == 1)
            .then(|| *ids.keys().next().unwrap())
            .filter(|c| Some(*c) != self.unknown)
    }

    pub fn group_of(&self, card: I) -> usize {
        self.group_of[card.index()]
    }
    pub fn is_recent(&self, card: I) -> bool {
        self.recent.contains(&card)
    }
    pub fn remember(&mut self, card: I) {
        self.recent.insert(card);
    }
    pub fn forget_recent(&mut self, card: I) -> bool {
        self.recent.remove(&card)
    }
    pub fn clear_recent(&mut self) {
        self.recent.clear();
    }
    pub fn take_arrivals(&mut self) -> Vec<Vec<I>> {
        std::mem::take(&mut self.arrived)
    }
    pub fn keep_arrival(&mut self, set: Vec<I>) {
        self.arrived.push(set);
    }
    pub fn change(&mut self, card: I, from: C, to: C) {
        let group = &mut self.groups[self.group_of[card.index()]];
        if let Some(ids) = &mut group.ids {
            remove(ids, from);
            add(ids, to, 1);
        }
        if let Some(f) = group.family
            && let Some(ids) = &mut self.families[f].ids
        {
            remove(ids, from);
            add(ids, to, 1);
        }
    }

    fn ensure(&mut self, card: I) {
        if self.group_of.len() <= card.index() {
            self.group_of.resize(card.index() + 1, usize::MAX);
        }
    }

    pub fn new_group(&mut self, cards: Vec<I>, ids: Option<Counts<C>>) {
        let g = self.groups.len();
        for &c in &cards {
            self.ensure(c);
            self.group_of[c.index()] = g;
        }
        self.groups.push(Group {
            cards,
            ids,
            ..Group::default()
        });
    }

    pub fn add_to(&mut self, g: usize, card: I, is: C) {
        self.ensure(card);
        self.group_of[card.index()] = g;
        let group = &mut self.groups[g];
        group.cards.push(card);
        if let Some(ids) = &mut group.ids {
            add(ids, is, 1);
        }
        if let Some(f) = group.family
            && let Some(ids) = &mut self.families[f].ids
        {
            add(ids, is, 1);
        }
    }

    /// The viewer sees `card`, which is `is`.
    pub fn identify(&mut self, card: I, is: C) {
        if let Some(i) = self.arrived.iter().position(|set| set.contains(&card)) {
            let among = self.arrived[i].clone();
            self.arrived[i].retain(|&c| c != card);
            if self.arrived[i].len() < 2 {
                self.arrived.swap_remove(i);
            }
            self.identify_among(card, is, &among);
            return;
        }
        self.identify_exact(card, is);
    }

    fn identify_exact(&mut self, card: I, is: C) {
        self.tighten();
        let g = self.group_of[card.index()];
        let family = self.groups[g].family;
        let removed = if self.groups[g]
            .ids
            .as_ref()
            .is_some_and(|ids| ids.contains_key(&is))
        {
            is
        } else {
            self.unknown.unwrap_or(is)
        };
        if let Some(f) = family
            && let Some(ids) = &mut self.families[f].ids
            && !remove(ids, removed)
        {
            debug_assert!(
                false,
                "{card:?} ({is:?}) missing from its family's identities"
            );
            self.families[f].ids = None;
        }
        let group = &mut self.groups[g];
        if group.cards.len() == 1 {
            group.ids = Some(Counts::<C>::from([(is, 1)]));
            group.extra = 0;
            if let Some(f) = group.family.take() {
                self.families[f].groups.retain(|&x| x != g);
                self.tidy(f);
            }
            return;
        }
        group.cards.retain(|&c| c != card);
        if let Some(ids) = &mut group.ids
            && !remove(ids, removed)
        {
            debug_assert!(
                false,
                "{card:?} ({is:?}) missing from its group's identities"
            );
            group.ids = None;
        }
        self.new_group(vec![card], Some(Counts::<C>::from([(is, 1)])));
    }

    /// Cards with only one possible identity no longer constrain their families.
    pub fn reduce(&mut self) {
        loop {
            self.tighten();
            let known: Vec<_> = self
                .groups
                .iter()
                .filter_map(|g| {
                    let ids = g.ids.as_ref()?;
                    (ids.len() == 1
                        && !g.cards.is_empty()
                        && g.family.is_some()
                        && ids.keys().next().copied() != self.unknown)
                        .then(|| (g.cards.clone(), *ids.keys().next().unwrap()))
                })
                .collect();
            if known.is_empty() {
                break;
            }
            for (cards, is) in known {
                for c in cards {
                    self.identify_exact(c, is);
                }
            }
        }
    }

    /// A family down to one group: that group is exactly the family's cards.
    fn tidy(&mut self, f: usize) {
        if let [g] = self.families[f].groups[..] {
            let family = std::mem::take(&mut self.families[f]);
            let group = &mut self.groups[g];
            group.ids = family.ids;
            group.extra = 0;
            group.family = None;
        }
    }

    /// Splits the groups `cards` are in that also hold other cards, so that `cards` are
    /// groups of their own: each part is some of what the group was.
    fn split_off(&mut self, cards: &[I]) {
        let inside: HashSet<I> = cards.iter().copied().collect();
        let mut groups: Vec<usize> = cards.iter().map(|c| self.group_of[c.index()]).collect();
        groups.sort_unstable();
        groups.dedup();
        for g in groups {
            let (inn, out): (Vec<I>, Vec<I>) = self.groups[g]
                .cards
                .iter()
                .partition(|c| inside.contains(c));
            if out.is_empty() {
                continue;
            }
            let group = &self.groups[g];
            let (ids, extra) = (group.ids.clone(), group.extra);
            let family = match group.family {
                Some(f) => Some(f),
                None if ids.is_some() => {
                    self.families.push(Family {
                        ids: ids.clone(),
                        groups: vec![g],
                    });
                    Some(self.families.len() - 1)
                }
                None => None,
            };
            let new = self.groups.len();
            for &c in &inn {
                self.group_of[c.index()] = new;
            }
            let group = &mut self.groups[g];
            group.extra = extra + inn.len() as u32;
            group.family = family;
            self.groups.push(Group {
                ids,
                extra: extra + out.len() as u32,
                family,
                cards: inn,
            });
            self.groups[g].cards = out;
            if let Some(f) = family {
                self.families[f].groups.push(new);
            }
        }
    }

    /// The viewer can no longer tell `cards` apart, but nothing else moved with them: the
    /// rest of their groups is some of what each was. Cards the viewer already couldn't
    /// tell apart stay with the rest of their group.
    pub fn mix(&mut self, cards: &[I]) {
        let group = |c: &I| self.group_of[c.index()];
        if cards.iter().all(|c| group(c) == group(&cards[0])) {
            return;
        }
        self.split_off(cards);
        self.merge(cards.iter().copied());
        self.merge_partial_families();
    }

    /// The viewer is told that `count` of `cards` are `is` without seeing which: the ones
    /// they know of, and for the rest cards they know nothing of. False, and nothing
    /// learned, when what they know of `cards` is tied to cards outside them or leaves
    /// too few unknown.
    pub fn learn(&mut self, cards: &[I], is: C, count: u32) -> bool {
        let Some(unknown) = self.unknown else {
            return false;
        };
        let inside: HashSet<I> = cards.iter().copied().collect();
        let mut groups: Vec<usize> = cards.iter().map(|c| self.group_of[c.index()]).collect();
        groups.sort_unstable();
        groups.dedup();
        let of = |g: &Group<C, I>, card| g.ids.as_ref().and_then(|ids| ids.get(&card)).copied();
        let apart = |g: &Group<C, I>| {
            g.family.is_none() && g.ids.is_some() && g.cards.iter().all(|c| inside.contains(c))
        };
        if !groups.iter().all(|&g| apart(&self.groups[g])) {
            return false;
        }
        let total = |card| -> u32 {
            groups
                .iter()
                .filter_map(|&g| of(&self.groups[g], card))
                .sum()
        };
        let more = count.saturating_sub(total(is));
        if more > total(unknown) {
            return false;
        }
        if more == 0 {
            return true;
        }
        let blank: Vec<I> = groups
            .iter()
            .filter(|&&g| of(&self.groups[g], unknown).is_some())
            .flat_map(|&g| self.groups[g].cards.iter().copied())
            .collect();
        self.merge(blank.iter().copied());
        let ids = self.groups[self.group_of[blank[0].index()]]
            .ids
            .as_mut()
            .unwrap();
        for _ in 0..more {
            remove(ids, unknown);
        }
        add(ids, is, more);
        true
    }

    /// The viewer sees that one of `among` is an `is`: `card`, but any of them that may be
    /// an `is` could be the one.
    fn identify_among(&mut self, card: I, is: C, among: &[I]) {
        let may_be = |c: &I| self.may_be(*c, is);
        let same: Vec<I> = among
            .iter()
            .copied()
            .filter(|c| !self.recent.contains(c))
            .filter(may_be)
            .chain([card])
            .collect();
        // This may be a copy the viewer already counted in the held set.
        let known = std::iter::once(card).chain(same.iter().copied()).find(|c| {
            self.groups[self.group_of[c.index()]]
                .ids
                .as_ref()
                .is_some_and(|ids| ids.len() == 1 && ids.contains_key(&is))
        });
        if let Some(other) = known {
            let (a, b) = (self.group_of[card.index()], self.group_of[other.index()]);
            if a != b {
                for c in &mut self.groups[a].cards {
                    if *c == card {
                        *c = other;
                    }
                }
                for c in &mut self.groups[b].cards {
                    if *c == other {
                        *c = card;
                    }
                }
                self.group_of[card.index()] = b;
                self.group_of[other.index()] = a;
            }
        } else {
            self.merge(same);
            self.merge_partial_families();
        }
        self.identify(card, is);
    }

    /// The viewer sees `card`, an `is`, in or leaving `zone`. `held` is the set it is
    /// held in out of their sight, if any.
    pub fn see(&mut self, card: I, is: C, zone: Z, held: Option<&[I]>) {
        let Some(held) = held else {
            self.disturb(card, zone);
            self.identify(card, is);
            return;
        };
        let mut among = held.to_vec();
        match &mut self.leaving {
            // Cards of this set left unseen: this one could as well be one of those.
            Some(l) if l.rest.contains(&card) && among.iter().all(|c| l.rest.contains(c)) => {
                l.rest.retain(|&c| c != card);
                l.seen.push(card);
                among.extend(&l.left);
            }
            _ => self.settle(),
        }
        self.identify_among(card, is, &among);
    }

    /// `card` leaves `from` for `to` and the viewer doesn't see what it is. `held` is the
    /// set it was held in out of their sight, if any.
    pub fn lose(&mut self, card: I, from: Z, to: Z, held: Option<&[I]>) {
        self.disturb(card, from);
        let arrived = self
            .arrived
            .iter()
            .position(|set| set.contains(&card))
            .map(|i| self.arrived.swap_remove(i));
        let continuing = self
            .leaving
            .as_ref()
            .filter(|l| l.from == from && l.to == to && l.rest.contains(&card))
            .map(|l| l.rest.clone());
        let held = held.or(arrived.as_deref()).or(continuing.as_deref());
        let Some(held) = held else {
            return;
        };
        let rest: Vec<I> = held.iter().copied().filter(|&c| c != card).collect();
        match &mut self.leaving {
            Some(l)
                if l.to == to
                    && l.rest.contains(&card)
                    && rest.iter().all(|c| l.rest.contains(c)) =>
            {
                l.rest.retain(|&c| c != card);
                l.left.push(card);
            }
            _ => {
                self.settle();
                self.leaving = Some(Leaving {
                    from,
                    to,
                    left: vec![card],
                    rest,
                    seen: vec![],
                });
            }
        }
    }

    /// A card the viewer can follow is in `zone`, or leaving it: if that touches the
    /// cards that just left a held set, the viewer stops waiting for the rest of it.
    fn disturb(&mut self, card: I, zone: Z) {
        let touched =
            |l: &Leaving<I, Z>| !l.seen.contains(&card) && (l.to == zone || l.left.contains(&card));
        if self.leaving.as_ref().is_some_and(touched) {
            self.settle();
        }
    }

    /// The cards that left a held set unseen could be any of it.
    pub fn settle(&mut self) {
        if let Some(l) = self.leaving.take() {
            if l.rest.is_empty() {
                if l.left.len() > 1
                    && l.from.is_hand()
                    && l.to.is_discard()
                    && l.left
                        .iter()
                        .any(|c| self.group_of[c.index()] != self.group_of[l.left[0].index()])
                {
                    self.arrived.push(l.left);
                }
                return;
            }
            // Only the set's cards: the rest of their groups didn't go anywhere.
            let set: Vec<I> = l.left.into_iter().chain(l.rest).collect();
            self.mix(&set);
        }
    }

    /// The viewer can no longer tell `cards` apart: their groups become one. A family all
    /// of whose groups are among them is whole again.
    fn merge(&mut self, cards: impl IntoIterator<Item = I>) {
        let mut groups: Vec<usize> = cards
            .into_iter()
            .map(|c| self.group_of[c.index()])
            .collect();
        groups.sort_unstable();
        groups.dedup();
        let Some((&into, rest)) = groups.split_first() else {
            return;
        };
        if rest.is_empty() {
            return;
        }
        let mut families: Vec<usize> = groups
            .iter()
            .filter_map(|&g| self.groups[g].family)
            .collect();
        families.sort_unstable();
        families.dedup();
        let (whole, partial): (Vec<usize>, Vec<usize>) = families
            .into_iter()
            .partition(|&f| self.families[f].groups.iter().all(|g| groups.contains(g)));
        // What the cards outside partial families are, and what the others may be.
        let mut exact = Some(Counts::<C>::new());
        let mut pool = Some(Counts::<C>::new());
        let mut extra = 0;
        for &g in &groups {
            let group = &self.groups[g];
            match group.family {
                Some(f) if partial.contains(&f) => {
                    add_all(&mut pool, group.ids.as_ref());
                    extra += group.extra;
                }
                Some(_) => {}
                None => add_all(&mut exact, group.ids.as_ref()),
            }
        }
        for &f in &whole {
            let family = std::mem::take(&mut self.families[f]);
            add_all(&mut exact, family.ids.as_ref());
        }
        for &g in rest {
            let taken = std::mem::take(&mut self.groups[g]);
            for &c in &taken.cards {
                self.group_of[c.index()] = into;
            }
            self.groups[into].cards.extend(taken.cards);
        }
        let Some((&f, others)) = partial.split_first() else {
            let group = &mut self.groups[into];
            (group.ids, group.extra, group.family) = (exact, 0, None);
            return;
        };
        // The partial families become one, which the cards that joined them are part of.
        for &o in others {
            let family = std::mem::take(&mut self.families[o]);
            for &g in &family.groups {
                self.groups[g].family = Some(f);
            }
            add_all(&mut self.families[f].ids, family.ids.as_ref());
            self.families[f].groups.extend(family.groups);
        }
        add_all(&mut self.families[f].ids, exact.as_ref());
        self.families[f].groups.retain(|g| !groups.contains(g));
        self.families[f].groups.push(into);
        add_all(&mut pool, exact.as_ref());
        // No more of a card than the whole family has.
        if let (Some(pool), Some(all)) = (&mut pool, &self.families[f].ids) {
            for (card, n) in pool.iter_mut() {
                let most = all.get(card).copied().unwrap_or(0);
                extra -= n.saturating_sub(most);
                *n = (*n).min(most);
            }
            pool.retain(|_, n| *n > 0);
        }
        let group = &mut self.groups[into];
        (group.ids, group.extra, group.family) = (pool, extra, Some(f));
        self.tidy(f);
    }
}

impl<C: Ord, I: Eq + std::hash::Hash, Z> Default for Mind<C, I, Z> {
    fn default() -> Self {
        Self {
            group_of: vec![],
            groups: vec![],
            families: vec![],
            recent: HashSet::new(),
            leaving: None,
            arrived: vec![],
            unknown: None,
        }
    }
}

impl<C: Ord, I> Default for Group<C, I> {
    fn default() -> Self {
        Self {
            cards: vec![],
            ids: None,
            extra: 0,
            family: None,
        }
    }
}

impl<C: Ord> Default for Family<C> {
    fn default() -> Self {
        Self {
            ids: None,
            groups: vec![],
        }
    }
}

impl<C> Default for ZoneKnowledge<C> {
    fn default() -> Self {
        Self {
            count: 0,
            known: Counts::new(),
            unknown: 0,
        }
    }
}

impl<C, Z: Eq + std::hash::Hash> Default for KnownCards<C, Z> {
    fn default() -> Self {
        Self {
            zones: HashMap::new(),
            uncertain: vec![],
            owned: vec![],
        }
    }
}

impl<C: Copy + Ord + std::fmt::Debug, I: Instance, Z: Location> Mind<C, I, Z> {
    pub fn summarize(
        &self,
        players: usize,
        zone_of: impl Fn(I) -> Z,
        owner_of: impl Fn(Z) -> Option<usize>,
        visible_as: impl Fn(I) -> Option<C>,
    ) -> KnownCards<C, Z> {
        let mut out = KnownCards {
            owned: vec![(Counts::<C>::new(), 0); players],
            ..KnownCards::default()
        };
        let zones_of = |cards: &mut dyn Iterator<Item = I>| {
            let mut zones: Vec<(Z, usize)> = vec![];
            for c in cards {
                let zone = zone_of(c);
                match zones.iter_mut().find(|(z, _)| *z == zone) {
                    Some((_, n)) => *n += 1,
                    None => zones.push((zone, 1)),
                }
            }
            // Not in the cards' order, which says which card is where.
            zones.sort();
            zones
        };
        // A card is owned as what it is printed as; how many cards each player owns is
        // public, whoever's they can't be told apart from.
        let mut own = |zones: &[(Z, usize)], ids: Option<&Counts<C>>| {
            let owner = owner_of(zones[0].0);
            let one_owner = zones.iter().all(|(z, _)| owner_of(*z) == owner);
            match (owner.filter(|_| one_owner), ids) {
                (Some(p), Some(ids)) => {
                    let owned = &mut out.owned[p].0;
                    ids.iter().for_each(|(&c, &n)| add(owned, c, n));
                }
                _ => {
                    for &(zone, n) in zones {
                        if let Some(p) = owner_of(zone) {
                            out.owned[p].1 += n;
                        }
                    }
                }
            }
        };
        let mut whole = HashSet::new();
        let mut whole_zones = vec![];
        for (i, family) in self
            .families
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.groups.is_empty())
        {
            let mut cards = family
                .groups
                .iter()
                .flat_map(|&g| self.groups[g].cards.iter().copied());
            let zones = zones_of(&mut cards);
            own(&zones, family.ids.as_ref());
            if let ([(zone, n)], Some(ids)) = (&zones[..], &family.ids) {
                whole.insert(i);
                whole_zones.push((*zone, *n, ids));
            }
        }
        let groups = self.groups.iter().filter(|g| !g.cards.is_empty());
        for (group, zones) in groups.map(|g| (g, zones_of(&mut g.cards.iter().copied()))) {
            if group.family.is_none() {
                own(&zones, group.ids.as_ref());
            }
        }
        for (zone, n, ids) in whole_zones {
            let z = out.zones.entry(zone).or_default();
            z.count += n;
            for (&c, &k) in ids {
                add(&mut z.known, c, k);
            }
        }
        for group in self.groups.iter().filter(|g| !g.cards.is_empty()) {
            if group.family.is_some_and(|f| whole.contains(&f)) {
                continue;
            }
            let zones = zones_of(&mut group.cards.iter().copied());
            let mut extra = group.extra;
            let ids = match (&group.cards[..], &group.ids) {
                ([c], Some(_)) if visible_as(*c).is_some() => {
                    visible_as(*c).map(|card| Counts::<C>::from([(card, 1)]))
                }
                (_, Some(ids)) => {
                    // No more of a card than its whole family has.
                    let all = group.family.and_then(|f| self.families[f].ids.as_ref());
                    let mut ids = ids.clone();
                    if let Some(all) = all {
                        for (card, n) in ids.iter_mut() {
                            let most = all.get(card).copied().unwrap_or(0);
                            extra -= n.saturating_sub(most);
                            *n = (*n).min(most);
                        }
                        ids.retain(|_, n| *n > 0);
                    }
                    Some(ids)
                }
                (_, None) => None,
            };
            // Cards that can only be one card are known wherever they are.
            let only = match &ids {
                Some(ids) if ids.len() == 1 => ids.keys().next().copied(),
                _ => None,
            };
            for &(zone, n) in &zones {
                let z = out.zones.entry(zone).or_default();
                z.count += n;
                match (&ids, only) {
                    (_, Some(card)) => add(&mut z.known, card, n as u32),
                    // Some of the candidates: each is there as often as it is a candidate
                    // more times than there are candidates elsewhere.
                    (Some(ids), _) => {
                        let mut known = 0;
                        for (&c, &k) in ids {
                            let at_least = k.saturating_sub(extra + (group.cards.len() - n) as u32);
                            if at_least > 0 {
                                add(&mut z.known, c, at_least);
                                known += at_least as usize;
                            }
                        }
                        z.unknown += n - known;
                    }
                    _ => z.unknown += n,
                }
            }
            if zones.len() > 1
                && only.is_none()
                && let Some(candidates) = ids
            {
                out.uncertain.push(UncertainCards {
                    zones,
                    candidates,
                    outside: extra as usize,
                    unknown: 0,
                });
            }
        }
        if let Some(unknown) = self.unknown {
            for zone in out.zones.values_mut() {
                zone.unknown += zone.known.remove(&unknown).unwrap_or(0) as usize;
            }
            for (known, count) in &mut out.owned {
                *count += known.remove(&unknown).unwrap_or(0) as usize;
            }
            for u in &mut out.uncertain {
                u.unknown = u.candidates.remove(&unknown).unwrap_or(0) as usize;
            }
            out.uncertain.retain(|u| !u.candidates.is_empty());
        }
        out.uncertain
            .sort_by(|a, b| (&a.zones, &a.candidates).cmp(&(&b.zones, &b.candidates)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_minds_deserialize_without_new_fields() {
        let json = serde_json::json!({
            "group_of": [0, 0],
            "groups": [{"cards": [0, 1], "ids": {"1": 1, "2": 1}}],
            "recent": []
        });
        let mind: Mind<i32, usize, i32> = serde_json::from_value(json).unwrap();
        let known = mind.summarize(1, |_| 0, |_| Some(0), |_| None);
        assert_eq!(known.zone(0).known, Counts::from([(1, 1), (2, 1)]));
        assert!(serde_json::to_value(mind).unwrap().get("unknown").is_none());
    }

    #[test]
    fn an_unknown_copy_does_not_exhaust_a_named_candidate_elsewhere() {
        let mut mind = Mind::<i32, usize, i32>::with_unknown(0);
        mind.new_group(vec![0, 1], Some(Counts::from([(2, 1), (3, 1)])));
        mind.new_group(vec![2, 3, 4], Some(Counts::from([(0, 1), (1, 1), (2, 1)])));
        // One of {2, 3} joins {unknown, 1, 2}; the other remains elsewhere.
        mind.mix(&[1, 2, 3, 4]);
        mind.identify(2, 2);
        mind.identify(3, 1);
        mind.identify(4, 3);
        mind.reduce();
        let known = mind.summarize(1, |c| i32::from(c != 0), |_| Some(0), |_| None);
        assert_eq!(known.zone(0).unknown, 1);
        assert!(known.zone(0).known.is_empty());
        assert_eq!(known.owned[0], (Counts::from([(1, 1), (2, 2), (3, 1)]), 1));
    }

    #[test]
    fn mixing_with_unknown_cards_preserves_guaranteed_composition() {
        let mut mind = Mind::<i32, usize, i32>::with_unknown(0);
        mind.new_group(vec![0, 1], Some(Counts::from([(1, 2)])));
        mind.new_group(vec![2], Some(Counts::from([(2, 1)])));
        mind.new_group(vec![3], Some(Counts::from([(0, 1)])));
        mind.mix(&[0, 1, 2, 3]);
        let known = mind.summarize(1, |c| i32::from(c == 0), |_| Some(0), |_| None);
        assert_eq!(known.zone(0).known, Counts::from([(1, 1)]));
        assert_eq!(known.zone(0).unknown, 2);
        assert_eq!(known.owned[0], (Counts::from([(1, 2), (2, 1)]), 1));
        assert_eq!(
            known.uncertain,
            vec![UncertainCards {
                zones: vec![(0, 3), (1, 1)],
                candidates: Counts::from([(1, 2), (2, 1)]),
                outside: 0,
                unknown: 1,
            }]
        );
    }
}

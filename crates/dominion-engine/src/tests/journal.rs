//! What the journal says caused each step.

use super::*;
use crate::game::Phase;

/// A journal's steps, markers taken out, each with the index of the step recording the
/// occurrence it happened inside.
struct Journal {
    steps: Vec<(Step, Option<usize>)>,
    /// Each occurrence's step, by its number.
    occurrences: Vec<usize>,
}

impl Journal {
    fn of(g: &mut Game) -> Journal {
        let mut j = Journal {
            steps: vec![],
            occurrences: vec![],
        };
        let mut within = None;
        for step in g.take_journal() {
            match step {
                Step::Occurred(n) => {
                    assert_eq!(n as usize, j.occurrences.len());
                    j.occurrences.push(j.steps.len() - 1);
                }
                Step::Within(occ) => within = occ.map(|n| j.occurrences[n as usize]),
                step => j.steps.push((step, within)),
            }
        }
        j
    }

    fn all(&self, what: impl Fn(&Step) -> bool) -> Vec<usize> {
        (0..self.steps.len())
            .filter(|&i| what(&self.steps[i].0))
            .collect()
    }

    fn only(&self, what: impl Fn(&Step) -> bool) -> usize {
        let found = self.all(what);
        assert_eq!(found.len(), 1, "{found:?}");
        found[0]
    }

    fn by(&self, i: usize) -> Option<usize> {
        self.steps[i].1
    }

    /// The steps `i` caused that are moves of `verb`.
    fn moves_by(&self, i: usize, verb: Verb) -> usize {
        let moves = self.all(|s| matches!(s, Step::Trace { verb: Some(v), .. } if *v == verb));
        moves.into_iter().filter(|&m| self.by(m) == Some(i)).count()
    }
}

fn played(g: &Game, card: Card) -> impl Fn(&Step) -> bool + '_ {
    move |s| matches!(s, Step::Played { card: c, .. } if g.card(*c) == card)
}

fn resolved(card: Card) -> impl Fn(&Step) -> bool {
    move |s| matches!(s, Step::Resolve { card: c, .. } if *c == card)
}

fn moved(g: &Game, verb: Verb, card: Card) -> impl Fn(&Step) -> bool + '_ {
    move |s| {
        matches!(s, Step::Trace { entry: Trace::Moved { card: c, .. }, verb: Some(v) }
            if *v == verb && g.printed(*c) == card)
    }
}

fn logged<'a>(g: &'a Game, what: impl Fn(&Event) -> bool + 'a) -> impl Fn(&Step) -> bool + 'a {
    move |s| matches!(s, Step::Logged { index } if what(&g.log()[*index]))
}

/// Ends phases and resolves start-of-turn abilities in the order offered until `p`'s
/// `n`th turn asks for its first move.
fn to_turn(g: &mut Game, p: PlayerId, n: u32) {
    while !(g.turn().player == p
        && g.player(p).turns_taken == n
        && matches!(
            g.pending().map(|d| &d.kind),
            Some(DecisionKind::Turn { .. })
        ))
    {
        match &g.pending().expect("a decision is pending").kind {
            DecisionKind::Turn { .. } => end_phase(g),
            DecisionKind::Choose { .. } if purpose(g) == Purpose::Trigger => {
                respond(g, Response::option(Opt::Trigger(0)))
            }
            k => panic!("unexpected decision {k:?}"),
        }
    }
}

#[test]
fn a_duration_played_twice_is_two_plays_each_with_its_next_turn() {
    let mut g = setup(&[Card::Wharf, Card::ThroneRoom]);
    hand(&mut g, P0, &[Card::ThroneRoom, Card::Wharf]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.enable_journal();
    g.start();
    play(&mut g, Card::ThroneRoom);
    choose_kinds(&mut g, &[Card::Wharf]);
    to_turn(&mut g, P0, 2);
    let j = Journal::of(&mut g);
    let throne = j.only(played(&g, Card::ThroneRoom));
    // The Action it cost, and none for the plays it makes.
    assert_eq!(
        j.steps[throne + 1],
        (
            Step::Gauge {
                gauge: Gauge::Actions,
                was: 1,
                now: 0
            },
            Some(throne)
        )
    );
    let wharfs = j.all(played(&g, Card::Wharf));
    assert_eq!(wharfs.len(), 2);
    for &w in &wharfs {
        assert_eq!(j.by(w), Some(throne));
        assert_eq!(j.moves_by(w, Verb::Draw), 2);
    }
    let turn = j.all(|s| matches!(s, Step::Turn { player: P0, .. }))[1];
    let resolves = j.all(resolved(Card::Wharf));
    let mut origins = vec![];
    for &r in &resolves {
        let Step::Resolve {
            occasion: Occasion::StartTurn { player: P0 },
            origin: Some(origin),
            ..
        } = j.steps[r].0
        else {
            panic!("{:?}", j.steps[r]);
        };
        assert_eq!(j.by(r), Some(turn));
        assert_eq!(j.moves_by(r, Verb::Draw), 2);
        origins.push(j.occurrences[origin as usize]);
    }
    assert_eq!(origins, wharfs);
}

#[test]
fn a_reaction_answers_the_attack_against_its_player() {
    let mut g = setup(&[Card::Militia, Card::Moat]);
    hand(&mut g, P0, &[Card::Militia]);
    hand(
        &mut g,
        P1,
        &[
            Card::Moat,
            Card::Copper,
            Card::Copper,
            Card::Estate,
            Card::Estate,
        ],
    );
    g.enable_journal();
    g.start();
    let militia = find(&g, P0, Card::Militia);
    play(&mut g, Card::Militia);
    respond(&mut g, Response::option(Opt::Trigger(0)));
    let j = Journal::of(&mut g);
    let play = j.only(played(&g, Card::Militia));
    let moat = j.only(resolved(Card::Moat));
    assert_eq!(
        j.steps[moat],
        (
            Step::Resolve {
                player: P1,
                card: Card::Moat,
                occasion: Occasion::Attack {
                    attacker: P0,
                    card: militia,
                    target: P1
                },
                origin: None,
            },
            Some(play)
        )
    );
    let unaffected = j.only(logged(&g, |e| matches!(e, Event::Unaffected { .. })));
    assert_eq!(j.by(unaffected), Some(moat));
}

#[test]
fn a_would_gain_ability_answers_the_buy_before_the_gain() {
    let mut g = setup(&[Card::NomadCamp]);
    hand(&mut g, P0, &[Card::Silver, Card::Silver]);
    g.enable_journal();
    g.start();
    play(&mut g, Card::Silver);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::NomadCamp);
    let j = Journal::of(&mut g);
    let buy = j.only(logged(&g, |e| matches!(e, Event::Bought { .. })));
    // Its price.
    for (gauge, was, now) in [(Gauge::Buys, 1, 0), (Gauge::Coins, 4, 0)] {
        let paid = j.only(|s| *s == Step::Gauge { gauge, was, now });
        assert_eq!(j.by(paid), Some(buy));
    }
    let camp = j.only(resolved(Card::NomadCamp));
    assert!(matches!(
        j.steps[camp].0,
        Step::Resolve {
            occasion: Occasion::WouldGain { .. },
            ..
        }
    ));
    assert_eq!(j.by(camp), Some(buy));
    let gain = j.only(moved(&g, Verb::Gain, Card::NomadCamp));
    assert!(camp < gain);
    assert_eq!(j.by(gain), Some(buy));
}

#[test]
fn a_when_gain_ability_answers_the_gain() {
    let mut g = setup(&[Card::IllGottenGains]);
    hand(&mut g, P0, &[Card::Gold, Card::Silver]);
    g.enable_journal();
    g.start();
    play(&mut g, Card::Gold);
    play(&mut g, Card::Silver);
    buy(&mut g, Card::IllGottenGains);
    let j = Journal::of(&mut g);
    let gain = j.only(moved(&g, Verb::Gain, Card::IllGottenGains));
    let ability = j.only(resolved(Card::IllGottenGains));
    assert!(matches!(
        j.steps[ability].0,
        Step::Resolve {
            occasion: Occasion::Gain { .. },
            ..
        }
    ));
    assert_eq!(j.by(ability), Some(gain));
    let curse = j.only(moved(&g, Verb::Gain, Card::Curse));
    assert_eq!(j.by(curse), Some(ability));
}

#[test]
fn a_when_trash_ability_answers_its_own_cards_trash() {
    let mut g = setup(&[Card::Chapel, Card::Fortress]);
    hand(
        &mut g,
        P0,
        &[Card::Chapel, Card::Copper, Card::Fortress, Card::Estate],
    );
    g.enable_journal();
    g.start();
    play(&mut g, Card::Chapel);
    choose_kinds(&mut g, &[Card::Copper, Card::Fortress, Card::Estate]);
    let j = Journal::of(&mut g);
    let chapel = j.only(played(&g, Card::Chapel));
    let trashed = j.all(|s| {
        matches!(
            s,
            Step::Trace {
                verb: Some(Verb::Trash),
                ..
            }
        )
    });
    assert_eq!(trashed.len(), 3);
    for &t in &trashed {
        assert_eq!(j.by(t), Some(chapel));
    }
    let fortress = j.only(moved(&g, Verb::Trash, Card::Fortress));
    assert_ne!(Some(&fortress), trashed.last());
    let ability = j.only(resolved(Card::Fortress));
    assert_eq!(j.by(ability), Some(fortress));
    assert_eq!(j.moves_by(ability, Verb::Put), 1);
}

#[test]
fn clean_up_discards_and_draws_for_its_phase() {
    let mut g = setup(&[]);
    hand(&mut g, P0, &[Card::Copper, Card::Estate]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.enable_journal();
    g.start();
    play(&mut g, Card::Copper);
    to_turn(&mut g, P1, 1);
    let j = Journal::of(&mut g);
    let turn = j.only(|s| matches!(s, Step::Turn { player: P0, .. }));
    let cleanup = j.all(|s| *s == Step::Phase(Phase::Cleanup))[0];
    assert_eq!(j.by(cleanup), Some(turn));
    assert_eq!(j.moves_by(cleanup, Verb::Discard), 2);
    assert_eq!(j.moves_by(cleanup, Verb::Draw), 5);
    // The Copper's +$1 is its play's, not the phase's.
    let copper = j.only(played(&g, Card::Copper));
    let coins = j.only(|s| {
        matches!(
            s,
            Step::Gauge {
                gauge: Gauge::Coins,
                now: 1,
                ..
            }
        )
    });
    assert_eq!(j.by(coins), Some(copper));
}

#[test]
fn a_turn_ends_inside_itself_and_what_ends_with_it_inside_its_end() {
    let mut g = setup(&[Card::Outpost]);
    hand(&mut g, P0, &[Card::Outpost]);
    deck(&mut g, P0, &[Card::Copper; 20]);
    g.enable_journal();
    g.start();
    play(&mut g, Card::Outpost);
    to_turn(&mut g, P0, 2);
    let j = Journal::of(&mut g);
    let outpost = j.only(played(&g, Card::Outpost));
    let began = j.all(|s| matches!(s, Step::EffectBegan(_)));
    assert!(!began.is_empty());
    for &e in &began {
        assert_eq!(j.by(e), Some(outpost));
    }
    let queued = j.only(|s| matches!(s, Step::ExtraTurnQueued { .. }));
    assert_eq!(j.by(queued), Some(outpost));
    let turns = j.all(|s| matches!(s, Step::Turn { player: P0, .. }));
    let end = j.only(|s| matches!(s, Step::TurnEnded { player: P0 }));
    assert_eq!(j.by(end), Some(turns[0]));
    let expired = j.only(|s| {
        matches!(
            s,
            Step::EffectEnded {
                why: EffectEnd::Expired,
                ..
            }
        )
    });
    assert_eq!(j.by(expired), Some(end));
    // Released as the extra turn is taken, between turns.
    let released = j.only(|s| {
        matches!(
            s,
            Step::EffectEnded {
                why: EffectEnd::ExtraTurn,
                ..
            }
        )
    });
    assert_eq!(j.by(released), None);
    assert!(matches!(
        j.steps[turns[1]],
        (
            Step::Turn {
                kind: TurnKind::Extra {
                    source: Card::Outpost,
                    ..
                },
                ..
            },
            None
        )
    ));
}

#[test]
fn what_a_received_boon_does_is_inside_its_receiving() {
    let mut g = setup(&[Card::Bard]);
    hand(&mut g, P0, &[Card::Bard]);
    deck(&mut g, P0, &[Card::Copper; 10]);
    g.boons.draw = vec![Card::TheFieldsGift];
    g.enable_journal();
    g.start();
    play(&mut g, Card::Bard);
    let j = Journal::of(&mut g);
    let bard = j.only(played(&g, Card::Bard));
    let received = j.only(logged(&g, |e| matches!(e, Event::Received { .. })));
    assert_eq!(j.by(received), Some(bard));
    // +1 Action and +$1.
    let actions = j.only(|s| {
        matches!(
            s,
            Step::Gauge {
                gauge: Gauge::Actions,
                was: 0,
                now: 1
            }
        )
    });
    assert_eq!(j.by(actions), Some(received));
    let taken = j.only(|s| {
        matches!(
            s,
            Step::Fate {
                card: Card::TheFieldsGift,
                ..
            }
        )
    });
    assert_eq!(j.by(taken), Some(bard));
}

#[test]
fn a_hex_received_by_another_player_is_inside_its_receiving() {
    let mut g = setup(&[Card::Skulk]);
    hand(&mut g, P0, &[Card::Skulk]);
    g.hexes.draw = vec![Card::Delusion];
    g.enable_journal();
    g.start();
    play(&mut g, Card::Skulk);
    let j = Journal::of(&mut g);
    let skulk = j.only(played(&g, Card::Skulk));
    let received = j.only(logged(&g, |e| {
        matches!(
            e,
            Event::Received {
                player: P1,
                card: Card::Delusion
            }
        )
    }));
    assert_eq!(j.by(received), Some(skulk));
    let deluded = j.only(|s| {
        matches!(
            s,
            Step::Holding {
                card: Card::Deluded,
                to: Some(P1),
                ..
            }
        )
    });
    assert_eq!(j.by(deluded), Some(received));
}

use super::cornucopia_guilds::fuzz_sets;
use super::*;

#[test]
fn random_games_with_every_card() {
    let plays = fuzz_sets(&[Set::Hinterlands], &[Card::Province, Card::Gold], 300);
    for card in Card::in_set(Set::Hinterlands) {
        if !card.is(Types::VICTORY) {
            assert!(plays.iter().any(|&(c, _)| c == card), "{card} never played");
        }
    }
}

#[test]
fn random_games_mixing_sets() {
    fuzz_sets(
        &[Set::Base, Set::CornucopiaGuilds, Set::Hinterlands],
        &[Card::Province],
        300,
    );
}

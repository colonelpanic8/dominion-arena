// What the dominion.games transport carries beyond a seat's table: the score page after
// the game, a game watched, the cards their client lacks.
export const OFFICIAL_TRANSPORT = [
  {
    name: 'game-end-score',
    about: 'the score page after the end notice: three piles empty',
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 20 Curse, Estate, Cellar'],
    after: ['turns 60 Curse, Estate, Cellar'],
    show: [{ button: 'Ok' }],
  },
  {
    name: 'resigned-score',
    about: 'the score page after Bob resigned',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 6 Gold, Laboratory, Silver'],
    after: ['resign Bob'],
    show: [{ button: 'Ok' }],
  },
  {
    name: 'first-edition-refused',
    about: "first-edition cards their client doesn't have: its lobby refuses the game",
    spec: { kingdom: ['Adventurer', 'Chancellor', 'Feast', 'Spy', 'Thief', 'Woodcutter', 'Coppersmith', 'Great Hall', 'Saboteur', 'Tribute'] },
    steps: [],
    refused: 'Error setting up the kingdom.',
  },
  {
    name: 'spectator-midgame',
    about: 'a game watched without a seat: every hand face up',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 6 Gold, Laboratory, Silver'],
    watch: true,
  },
];

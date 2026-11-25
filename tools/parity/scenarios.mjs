// The positions both clients are compared at. Each is a lobby table (`spec`, dealt with
// the run's fixed seed) and the steps that lead from the start to the position (see
// drive.mjs), then any steps played once both clients are watching (`after`). Steps can
// only play cards whose questions the dominion.games transport maps
// (`GET /api/official/coverage`); other cards are shown in an opening position.
export const SCENARIOS = [
  {
    name: 'opening',
    about: 'the first turn: the basic columns, a full kingdom, an Event and a Project',
    spec: {
      kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Mine', 'Witch', 'Gardens', 'Laboratory'],
      landscapes: ['Expedition', 'Cathedral'],
    },
    steps: [],
  },
  {
    name: 'colonies',
    about: 'Platinum, Colony and Potion in the basic columns; potion and debt costs',
    spec: {
      kingdom: ["Worker's Village", 'Alchemist', 'Engineer', 'Courtyard', 'Nobles', 'Mill', 'Bridge', 'Torturer', 'Baron', 'Upgrade'],
      colonies: true,
    },
    steps: ['turns 4 Silver'],
  },
  {
    name: 'seaside-mats',
    about: 'mats, Durations in play, Coffers, Villagers and a split pile after a few turns',
    spec: {
      kingdom: ['Native Village', 'Island', 'Haven', 'Lighthouse', 'Caravan', 'Baker', 'Recruiter', 'Settlers', 'Bazaar', 'Outpost'],
    },
    steps: ['turns 8 Island, Native Village, Baker, Recruiter, Haven, Lighthouse, Caravan, Silver'],
  },
  {
    name: 'intrigue-midgame',
    about: 'a midgame: bigger discard piles, mixed-type cards, a turn in progress',
    spec: {
      kingdom: ['Courtier', 'Masquerade', 'Nobles', 'Mining Village', 'Minion', 'Diplomat', 'Duke', 'Farm', 'Lurker', 'Pawn'],
    },
    steps: ['turns 10 Gold, Minion, Nobles, Mining Village, Courtier, Silver'],
  },
  {
    name: 'prosperity',
    about: 'Prosperity: Platinum and Colony, a variable cost (Peddler), King\'s Court',
    spec: {
      kingdom: ['Bank', 'Bishop', 'City', 'Expand', 'Forge', 'Goons', "King's Court", 'Monument', 'Peddler', 'Hoard'],
      colonies: true,
    },
    steps: [],
  },
  {
    name: 'empires',
    about: 'split and mixed piles (Castles, Patrician, Gladiator, Catapult, Encampment), Landmarks and Events',
    spec: {
      kingdom: ['Patrician', 'Castles', 'Gladiator', 'Catapult', 'Encampment', 'Chariot Race', 'Temple', 'Villa', 'Wild Hunt', 'Forum'],
      landscapes: ['Tomb', 'Obelisk', 'Triumph', 'Dominate'],
    },
    steps: [],
  },
  {
    name: 'cornucopia',
    about: 'a Bane (Young Witch) and Prizes beside the Supply',
    spec: {
      kingdom: ['Young Witch', 'Tournament', 'Fairgrounds', 'Horn of Plenty', 'Menagerie', 'Remake', 'Harvest', 'Hamlet', 'Jester', 'Hunting Party'],
    },
    steps: [],
  },
  {
    name: 'renaissance',
    about: 'Projects and Ways together',
    spec: {
      kingdom: ['Ducat', 'Villain', 'Priest', 'Sculptor', 'Hideout', 'Scholar', 'Seer', 'Lackeys', 'Patron', 'Inventor'],
      landscapes: ['Academy', 'Barracks', 'Way of the Ox', 'Way of the Mole'],
    },
    steps: [],
  },
  {
    name: 'allies',
    about: 'split piles whose types show on the pile (Augur, Clash, Fort, Odyssey, Townsfolk, Wizard) and an Ally',
    spec: {
      kingdom: ['Augurs', 'Clashes', 'Forts', 'Odysseys', 'Townsfolk', 'Wizards', 'Courier', 'Bauble', 'Sycophant', 'Underling'],
      landscapes: ['League of Shopkeepers'],
    },
    steps: [],
  },
  {
    name: 'plunder',
    about: 'Traits on piles and Durations',
    spec: {
      kingdom: ['Cage', 'Grotto', 'Fortune Hunter', 'Sailor', 'Shaman', 'Buried Treasure', 'Cabin Boy', 'Crew', 'Flagship', 'Harbor Village'],
      landscapes: ['Cheap', 'Rich'],
    },
    steps: [],
  },
  {
    name: 'dark-ages',
    about: 'Knights, Ruins and the non-Supply piles (Spoils, Madman, Mercenary)',
    spec: {
      kingdom: ['Knights', 'Marauder', 'Urchin', 'Hermit', 'Rats', 'Squire', 'Graverobber', 'Death Cart', 'Bandit Camp', 'Pillage'],
    },
    steps: [],
  },
  {
    name: 'adventures',
    about: 'Travellers and their non-Supply piles, Reserves, Events',
    spec: {
      kingdom: ['Page', 'Peasant', 'Ratcatcher', 'Raze', 'Amulet', 'Caravan Guard', 'Dungeon', 'Gear', 'Guide', 'Duplicate'],
      landscapes: ['Alms', 'Bonfire'],
    },
    steps: [],
  },
  {
    name: 'menagerie',
    about: 'Horses beside the Supply, Ways, Exile, Hinterlands Reactions',
    spec: {
      kingdom: ['Cache', 'Tunnel', 'Trader', 'Nomads', 'Souk', 'Wheelwright', 'Sleigh', 'Supplies', 'Black Cat', 'Livery'],
      landscapes: ['Way of the Horse', 'Way of the Seal'],
    },
    steps: [],
  },
  {
    name: 'guilds',
    about: 'overpay and Coffers (Guilds)',
    spec: {
      kingdom: ['Advisor', 'Butcher', 'Candlestick Maker', 'Doctor', 'Masterpiece', 'Merchant Guild', 'Soothsayer', 'Stonemason', 'Taxman', 'Plaza'],
    },
    steps: [],
  },
  {
    name: 'hinterlands',
    about: 'Hinterlands: a cost reducer (Highway) and Reactions',
    spec: {
      kingdom: ['Border Village', 'Cartographer', 'Crossroads', 'Develop', 'Farmland', 'Haggler', 'Inn', 'Jack of All Trades', 'Margrave', 'Highway'],
    },
    steps: [],
  },
  {
    name: 'nocturne',
    about: 'Heirlooms in the starting deck, Night cards and their non-Supply piles (Wish, Bat, Ghost, Imp)',
    spec: {
      kingdom: ['Secret Cave', 'Cemetery', 'Shepherd', "Devil's Workshop", 'Pooka', 'Night Watchman', 'Ghost Town', 'Changeling', 'Monastery', 'Conclave'],
    },
    steps: [],
  },
  {
    name: 'nocturne-fate',
    about: "the Boons' and Hexes' decks, Druid's Boons, Heirlooms of Fate and Doom cards",
    spec: {
      kingdom: ['Druid', 'Bard', 'Tracker', 'Fool', 'Pixie', 'Leprechaun', 'Skulk', 'Cursed Village', 'Idol', 'Blessed Village'],
    },
    steps: [],
  },
  {
    name: 'tokens',
    about: 'Trade Route tokens on the Victory piles, Artifacts and Adventures tokens in the game',
    spec: {
      kingdom: ['Trade Route', 'Embargo', 'Swashbuckler', 'Flag Bearer', 'Treasurer', 'Border Guard', 'Bridge Troll', 'Relic', 'Ranger', 'Lost City'],
      landscapes: ['Lost Arts', 'Seaway'],
    },
    steps: [],
  },
  {
    name: 'rising-sun',
    about: "a Prophecy's Sun tokens, a Landmark's VP, Riverboat's card in the Box, Way of the Mouse's card",
    spec: {
      kingdom: ['Riverboat', 'Rustic Village', 'Poet', 'Kitsune', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar'],
      landscapes: ['Great Leader', 'Arena', 'Way of the Mouse', 'Sinister Plot'],
    },
    steps: [],
  },
  {
    name: 'seaside2',
    about: "Seaside's second edition: Durations and Treasures",
    spec: {
      kingdom: ['Astrolabe', 'Blockade', 'Corsair', 'Monkey', 'Pirate', 'Sailor', 'Sea Chart', 'Sea Witch', 'Tide Pools', 'Treasure Map'],
    },
    steps: [],
  },
  {
    name: 'play-row',
    about: "mid-Action phase: Durations from last turn and this turn's Actions in a full play row",
    spec: {
      kingdom: ['Village', 'Fishing Village', 'Caravan', 'Merchant Ship', 'Wharf', 'Festival', 'Bazaar', 'Laboratory', 'Market', 'Smithy'],
    },
    steps: [
      'turns 10 Laboratory, Fishing Village, Caravan, Festival, Silver',
      'until Alice Fishing Village+Laboratory: Laboratory, Caravan, Fishing Village, Silver',
      'play Fishing Village',
      'play Fishing Village',
      'play Laboratory',
    ],
  },
  {
    name: 'chapel-trash',
    about: 'choosing cards from the hand: Chapel trashing up to 4',
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 4 Remodel, Chapel', 'until Alice Chapel', 'play Chapel'],
  },
  {
    name: 'remodel-gain',
    about: "gaining from the Supply: Remodel's gain, the piles it can gain from highlighted",
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 4 Remodel, Chapel', 'until Alice Remodel', 'play Remodel', 'pick Estate'],
  },
  {
    name: 'pawn-mode',
    about: 'choosing modes: Pawn\'s two of four',
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 2 Pawn', 'until Alice Pawn', 'play Pawn'],
  },
  {
    name: 'sentry-look',
    about: 'cards looked at off the deck: Sentry asking what to trash',
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 8 Sentry, Silver', 'until Alice Sentry', 'play Sentry'],
  },
  {
    name: 'moat-militia',
    about: "an off-turn decision: Bob's Militia, Alice holding Moat",
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 6 Militia, Moat', 'until Bob Militia & Alice Moat: Militia, Moat', 'play Militia'],
  },
  {
    name: 'bob-turn',
    about: "watching Bob's turn: his play row, the Kingdom moved down",
    spec: {
      kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'],
    },
    steps: ['turns 6 Smithy, Village, Silver', 'until Bob Village+Smithy: Smithy, Village, Silver', 'play Village', 'play Smithy'],
  },
  {
    name: 'buy-phase',
    about: 'the Buy phase with Treasures played and $ shown',
    spec: {
      kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'],
    },
    steps: ['turns 6 Gold, Silver', 'until Alice Gold+Silver: Gold, Silver', 'treasures'],
  },
  {
    name: 'late-game',
    about: 'a late game: big discard piles, Native Village and Island mats, Reserves on the Tavern mat',
    spec: {
      kingdom: ['Native Village', 'Island', 'Ratcatcher', 'Guide', 'Coin of the Realm', 'Duplicate', 'Royal Carriage', 'Haven', 'Village', 'Smithy'],
    },
    steps: ['turns 20 Gold, Royal Carriage, Duplicate, Island, Native Village, Ratcatcher, Silver'],
  },
  {
    name: 'game-end',
    about: 'the score screen once three piles are empty',
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 20 Curse, Estate, Cellar'],
    after: ['turns 60 Curse, Estate, Cellar'],
  },
  {
    name: 'night-watchman',
    about: 'a Night-phase decision: Night Watchman looking at the top 5 cards',
    spec: {
      kingdom: ['Night Watchman', 'Monastery', 'Exorcist', 'Ghost Town', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar'],
    },
    steps: ['turns 4 Night Watchman, Silver', 'until Alice Night Watchman', 'treasures', 'end', 'play Night Watchman'],
  },
  {
    name: 'wishing-well-name',
    about: 'naming a card: Wishing Well',
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['turns 2 Wishing Well', 'until Alice Wishing Well', 'play Wishing Well'],
  },
  {
    name: 'masquerade-pass',
    about: 'passing a card to the next player: Masquerade',
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['turns 2 Masquerade', 'until Alice Masquerade', 'play Masquerade'],
  },
  {
    name: 'courtier-modes',
    about: "a question after a question: Courtier's modes once a Silver is revealed",
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['until Alice Courtier: Courtier, Silver', 'play Courtier', 'pick Silver'],
  },
  {
    name: 'lurker-pile',
    about: "a pile picked after a mode: Lurker trashing an Action from the Supply",
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['until Alice Lurker: Lurker', 'play Lurker', 'pick Trash an Action'],
  },
  {
    name: 'torturer-choice',
    about: "an attacked player's choice: Bob's Torturer",
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['until Bob Torturer: Torturer, Silver', 'play Torturer'],
  },
  {
    name: 'amulet-mode',
    about: 'choosing one of three: Amulet',
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['turns 2 Amulet', 'until Alice Amulet', 'play Amulet'],
  },
  {
    name: 'secret-passage-number',
    about: "choosing a number: where Secret Passage's card goes in the deck",
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['until Alice Secret Passage: Secret Passage, Silver', 'play Secret Passage', 'pick Copper'],
  },
  {
    name: 'masterpiece-overpay',
    about: 'overpaying: buying Masterpiece with $4',
    spec: { kingdom: ['Wishing Well', 'Masquerade', 'Courtier', 'Lurker', 'Torturer', 'Secret Passage', 'Masterpiece', 'Doctor', 'Graverobber', 'Amulet'] },
    steps: ['until Alice Copper+Copper+Copper+Copper', 'treasures', 'buy Masterpiece'],
  },
  {
    name: 'way-choice',
    about: 'playing an Action in a game with Ways: the card or a Way',
    spec: {
      kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'],
      landscapes: ['Way of the Ox', 'Way of the Mole'],
    },
    steps: ['turns 2 Village', 'until Alice Village', 'play Village'],
  },
];

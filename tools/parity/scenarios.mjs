// The positions both clients are compared at. Each is a lobby table (`spec`, dealt with
// the run's fixed seed) and the steps that lead from the start to the position (see
// drive.mjs), then any steps played once both clients are watching (`after`). Steps can
// only play cards whose questions the dominion.games transport maps
// (`GET /api/official/coverage`); other cards are shown in an opening position.
const bigHand = (d) => d.name === 'Alice' && d.view.you.hand.count >= 10;

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
  {
    name: 'three-players',
    about: 'three players a few turns in: two opponents above, a turn in progress',
    players: 3,
    spec: {
      kingdom: ['Village', 'Smithy', 'Market', 'Militia', 'Moat', 'Festival', 'Council Room', 'Workshop', 'Laboratory', 'Gardens'],
    },
    steps: ['turns 9 Gold, Laboratory, Market, Silver', 'act'],
  },
  {
    name: 'four-players-opening',
    about: "four players' first turn",
    players: 4,
    spec: {
      kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Mine', 'Witch', 'Gardens', 'Laboratory'],
      landscapes: ['Expedition', 'Cathedral'],
    },
    steps: [],
  },
  {
    name: 'four-players',
    about: 'four players a few turns in: three opponents, Durations in their play rows',
    players: 4,
    spec: {
      kingdom: ['Fishing Village', 'Caravan', 'Smithy', 'Market', 'Militia', 'Moat', 'Haven', 'Merchant Ship', 'Laboratory', 'Duke'],
    },
    steps: ['turns 12 Gold, Merchant Ship, Caravan, Fishing Village, Silver', 'act'],
  },
  {
    name: 'five-players',
    about: 'five players a few turns in',
    players: 5,
    spec: {
      kingdom: ['Village', 'Smithy', 'Market', 'Militia', 'Moat', 'Festival', 'Council Room', 'Workshop', 'Laboratory', 'Gardens'],
    },
    steps: ['turns 10 Gold, Laboratory, Market, Silver', 'act'],
  },
  {
    name: 'six-players',
    about: 'six players a few turns in: the most opponents, the largest piles',
    players: 6,
    spec: {
      kingdom: ['Fishing Village', 'Caravan', 'Smithy', 'Market', 'Militia', 'Moat', 'Haven', 'Merchant Ship', 'Laboratory', 'Duke'],
    },
    steps: ['turns 12 Gold, Merchant Ship, Caravan, Fishing Village, Silver', 'act'],
  },
  {
    name: 'split-piles',
    about: 'split piles whose top cards ran out, an emptied pile, a Trait on a split pile',
    spec: {
      kingdom: ['Settlers', 'Patrician', 'Catapult', 'Gladiator', 'Sauna', 'Encampment', 'Cellar', 'Village', 'Pawn', 'Chapel'],
      landscapes: ['Cheap'],
    },
    steps: ['spree 44 Sauna, Gladiator, Catapult, Settlers, Patrician, Cellar', 'act'],
  },
  {
    name: 'rotated-piles',
    about: "Allies' split piles rotated and bought down, an Ally with Favors",
    spec: {
      kingdom: ['Augurs', 'Clashes', 'Forts', 'Odysseys', 'Townsfolk', 'Wizards', 'Underling', 'Sycophant', 'Bauble', 'Importer'],
      landscapes: ['Band of Nomads'],
    },
    steps: ['prefer rotate', 'turns 12 Battle Plan, Herb Gatherer, Tent, Old Map, Student, Underling, Town Crier', 'act'],
  },
  {
    name: 'dark-ages-piles',
    about: 'Knights and Castles with other cards on top, Ruins running down',
    spec: {
      kingdom: ['Knights', 'Castles', 'Marauder', 'Death Cart', 'Cultist', 'Urchin', 'Squire', 'Village', 'Smithy', 'Market'],
    },
    steps: ['turns 14 Knights, Cultist, Death Cart, Marauder, Humble Castle', 'act'],
  },
  {
    name: 'pile-tokens',
    about: 'Trade Route tokens moved to the mat, Embargo tokens stacked, VP tokens on piles and Landmarks',
    spec: {
      kingdom: ['Trade Route', 'Embargo', 'Village', 'Smithy', 'Market', 'Workshop', 'Cellar', 'Militia', 'Moat', 'Laboratory'],
      landscapes: ['Aqueduct', 'Defiled Shrine', 'Basilica', 'Battlefield'],
    },
    steps: ['turns 16 Duchy, Trade Route, Embargo, Silver, Estate', 'act'],
  },
  {
    name: 'loot',
    about: 'the Loot pile and Loot in play (Jewelled Egg, Sack of Loot, Search, Pickaxe), a Trait whose pile emptied',
    spec: {
      kingdom: ['Jewelled Egg', 'Sack of Loot', 'Search', 'Pickaxe', 'Chapel', 'Cellar', 'Village', 'Smithy', 'Market', 'Moat'],
      landscapes: ['Looting', 'Peril', 'Rich'],
    },
    steps: ['prefer Jewelled Egg, Silver', 'spree 16 Looting, Sack of Loot, Pickaxe, Search, Jewelled Egg', 'act'],
  },
  {
    name: 'many-landscapes',
    about: 'an Event, a Project, Ways, an Ally, a Prophecy and a Landmark at once',
    spec: {
      kingdom: ['Rustic Village', 'Poet', 'Underling', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Laboratory'],
      landscapes: ['Alms', 'Pageant', 'Way of the Ox', 'Way of the Horse', 'League of Shopkeepers', 'Great Leader', 'Aqueduct', 'Way of the Pig'],
    },
    steps: [],
  },
  {
    name: 'landscapes-in-play',
    about: 'Actions played using Ways, Projects both players bought, an active Prophecy, Favors',
    spec: {
      kingdom: ['Rustic Village', 'Poet', 'Underling', 'Sycophant', 'Mountain Shrine', 'River Shrine', 'Smithy', 'Market', 'Moat', 'Cellar'],
      landscapes: ['Pageant', 'Fair', 'Way of the Ox', 'Way of the Mouse', 'Way of the Pig', 'League of Shopkeepers', 'Good Harvest'],
    },
    steps: ['turn Pageant', 'turn Pageant', 'turn Fair', 'turn Fair', 'turns 12 Rustic Village, Poet, River Shrine, Underling', 'prefer Way of the Ox', 'act'],
  },
  {
    name: 'states-artifacts',
    about: 'States from Hexes and Lost in the Woods, Artifacts that changed hands',
    spec: {
      kingdom: ['Fool', 'Skulk', 'Leprechaun', 'Cursed Village', 'Flag Bearer', 'Border Guard', 'Treasurer', 'Swashbuckler', 'Village', 'Smithy'],
    },
    steps: ['prefer Key', 'turns 4 Flag Bearer, Skulk', 'turns 10 Skulk, Treasurer, Fool, Flag Bearer, Cursed Village, Border Guard', 'act'],
  },
  {
    name: 'durations',
    about: 'Durations stacked in play over turns (Hireling, Archive, Church, Fishing Village, ...)',
    spec: {
      kingdom: ['Hireling', 'Archive', 'Church', 'Fishing Village', 'Amulet', 'Gear', 'Caravan', 'Wharf', 'Haven', 'Dungeon'],
    },
    steps: ['spree 16 Hireling, Archive, Wharf, Caravan, Fishing Village, Church, Gear, Amulet, Dungeon, Haven', 'act'],
  },
  {
    name: 'reserves-exile',
    about: 'the Tavern mat with several Reserves, the Exile mat with cards',
    spec: {
      kingdom: ['Ratcatcher', 'Coin of the Realm', 'Guide', 'Duplicate', 'Transmogrify', 'Royal Carriage', 'Distant Lands', 'Camel Train', 'Stockpile', 'Bounty Hunter'],
    },
    steps: ['turns 16 Distant Lands, Royal Carriage, Duplicate, Guide, Ratcatcher, Coin of the Realm, Camel Train, Stockpile', 'act'],
  },
  {
    name: 'big-hand',
    about: 'a hand of ten or more cards',
    spec: {
      kingdom: ['Village', 'Festival', 'Council Room', 'Library', 'Laboratory', 'Smithy', "Worker's Village", 'Lost City', 'Market', 'Moat'],
    },
    steps: ['villages', (d) => d.until(bigHand, ['Lost City', 'Council Room', 'Gold', 'Silver'])],
  },
  {
    name: 'big-play',
    about: 'many cards in play in one turn',
    spec: {
      kingdom: ['Village', 'Festival', 'Council Room', 'Library', 'Laboratory', 'Smithy', "Worker's Village", 'Lost City', 'Market', 'Moat'],
    },
    steps: ['villages', (d) => d.until(bigHand, ['Lost City', 'Council Room', 'Gold', 'Silver']), 'act'],
  },
  // Piles and the Supply, and cards of Prosperity, Cornucopia & Guilds, Empires, Allies
  // and Dark Ages played (parity round 3).
  {
    name: 'cost-reducers',
    about: 'costs lowered on the piles: Bridge, Highway and Quarry in play, Canal bought, Peddler in the Buy phase',
    spec: {
      kingdom: ['Bridge', 'Highway', 'Peddler', 'Quarry', 'Grand Market', 'Counting House', 'Mint', 'Venture', 'Loan', 'Investment'],
      landscapes: ['Canal'],
    },
    steps: [buyEach(28, ['Canal', 'Bridge', 'Highway', 'Quarry', 'Venture', 'Loan', 'Investment'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'tax-debt',
    about: "Tax's Debt on every Supply pile; Debt costs on piles and Events",
    spec: {
      kingdom: ['Capital', 'City Quarter', 'Royal Blacksmith', 'Overlord', 'Engineer', 'Charm', 'Crown', 'Groundskeeper', "Farmers' Market", 'Enchantress'],
      landscapes: ['Tax', 'Annex', 'Donate', 'Wedding', 'Mountain Pass', 'Museum', 'Orchard', 'Palace'],
    },
    steps: [],
  },
  {
    name: 'bane-ferryman',
    about: "Young Witch's Bane and Ferryman's pile marked on their piles, Cornucopia & Guilds played",
    spec: {
      kingdom: ['Young Witch', 'Ferryman', 'Footpad', 'Fortune Teller', 'Herald', 'Horse Traders', 'Farming Village', 'Carnival', 'Shop', 'Journeyman'],
    },
    steps: [buyEach(28, ['Young Witch', 'Ferryman', 'Footpad', 'Fortune Teller', 'Herald', 'Horse Traders', 'Journeyman', 'Farming Village', 'Carnival', 'Shop'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'adventures-tokens',
    about: "both players' Adventures tokens on piles: +1 Card, +1 Action, +$1, +1 Buy, -$2 cost, Trashing",
    spec: {
      kingdom: ['Market', 'Festival', 'Laboratory', 'Village', 'Smithy', 'Bazaar', 'Moat', 'Cellar', 'Workshop', 'Militia'],
      landscapes: ['Pathfinding', 'Lost Arts', 'Training', 'Seaway', 'Ferry', 'Plan'],
    },
    steps: [
      buyEach(28, ['Pathfinding', 'Lost Arts', 'Training', 'Seaway', 'Ferry', 'Plan'], ['Gold', 'Silver'], {
        Pathfinding: 'Smithy',
        'Lost Arts': 'Village',
        Training: 'Festival',
        Seaway: 'Workshop',
        Ferry: 'Militia',
        Plan: 'Cellar',
      }),
      'prefer',
      'act',
    ],
  },
  {
    name: 'prosperity-play',
    about: "Prosperity's Treasures and Attacks played: Talisman, Royal Seal, Contraband, Tiara, War Chest, Charlatan",
    spec: {
      kingdom: ['Talisman', 'Royal Seal', 'Contraband', 'Crystal Ball', 'Tiara', 'War Chest', 'Charlatan', 'Magnate', 'Vault', 'Clerk'],
      colonies: true,
    },
    steps: [buyEach(28, ['Platinum', 'Tiara', 'Royal Seal', 'Crystal Ball', 'War Chest', 'Contraband', 'Talisman', 'Charlatan', 'Magnate', 'Vault', 'Clerk'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'empires-play',
    about: "Empires played: Debt from Capital, Gathering VP on piles, Enchantress, Crown, Landmarks' VP; Debt costs on piles",
    spec: {
      kingdom: ['Capital', 'City Quarter', 'Royal Blacksmith', 'Charm', 'Crown', 'Groundskeeper', "Farmers' Market", 'Enchantress', 'Legionary', 'Sacrifice'],
      landscapes: ['Baths', 'Colonnade', 'Advance', 'Banquet'],
    },
    steps: [buyEach(28, ['Capital', 'Enchantress', 'Legionary', 'Crown', 'Charm', "Farmers' Market", 'Groundskeeper', 'Sacrifice'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'dark-ages-play',
    about: 'Dark Ages played: Band of Misfits, Counterfeit, Fortress, Altar, Armory, Count',
    spec: {
      kingdom: ['Altar', 'Armory', 'Band of Misfits', 'Catacombs', 'Count', 'Counterfeit', 'Fortress', 'Hunting Grounds', 'Ironmonger', 'Junk Dealer'],
    },
    steps: [buyEach(28, ['Altar', 'Hunting Grounds', 'Band of Misfits', 'Count', 'Counterfeit', 'Catacombs', 'Armory', 'Fortress', 'Ironmonger', 'Junk Dealer'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'dark-ages-play-2',
    about: 'Dark Ages played: Mystic, Procession, Sage, Scavenger, Storeroom, Vagrant, Wandering Minstrel, Rogue',
    spec: {
      kingdom: ['Mystic', 'Poor House', 'Procession', 'Sage', 'Scavenger', 'Storeroom', 'Vagrant', 'Wandering Minstrel', 'Rogue', 'Market Square'],
    },
    steps: [buyEach(28, ['Rogue', 'Procession', 'Mystic', 'Wandering Minstrel', 'Market Square', 'Sage', 'Scavenger', 'Storeroom', 'Vagrant', 'Poor House'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'allies-play',
    about: 'Allies played: Highwayman and Royal Galley in play, Barbarian, Capital City, Galleria, Innkeeper',
    spec: {
      kingdom: ['Barbarian', 'Capital City', 'Carpenter', 'Galleria', 'Highwayman', 'Hunter', 'Innkeeper', 'Marquis', 'Merchant Camp', 'Royal Galley'],
      landscapes: ["Architects' Guild"],
    },
    steps: [buyEach(28, ['Highwayman', 'Royal Galley', 'Barbarian', 'Marquis', 'Galleria', 'Capital City', 'Hunter', 'Innkeeper', 'Carpenter', 'Merchant Camp'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'allies-liaisons',
    about: 'Liaisons played for Favors: Broker, Emissary, Guildmaster, Contract; Sentinel, Skirmisher, Town',
    spec: {
      kingdom: ['Broker', 'Emissary', 'Guildmaster', 'Contract', 'Sentinel', 'Skirmisher', 'Specialist', 'Swap', 'Town', 'Modify'],
      landscapes: ['Cave Dwellers'],
    },
    steps: [buyEach(28, ['Guildmaster', 'Contract', 'Emissary', 'Broker', 'Skirmisher', 'Specialist', 'Town', 'Sentinel', 'Swap', 'Modify'], ['Gold', 'Silver']), 'act'],
  },
  {
    name: 'black-market',
    about: 'the Black Market deck beside the Supply',
    spec: {
      kingdom: ['Black Market', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop'],
    },
    steps: [],
  },
  {
    name: 'pile-of-one',
    about: 'a Supply pile down to its last card, Omens taking Sun tokens off a Prophecy',
    spec: {
      kingdom: ['Rustic Village', 'Poet', 'Village', 'Smithy', 'Market', 'Workshop', 'Cellar', 'Festival', 'Laboratory', 'Moat'],
      landscapes: ['Great Leader'],
    },
    steps: ['turns 6 Poet, Rustic Village, Silver', lastOne('Alice', 'Estate'), 'act'],
  },
];

function pileCount(view, card) {
  return (view.supply ?? []).find((p) => p.card === card)?.count;
}

/// Turns until `who` starts one with one card left in `card`'s pile, which only they buy.
function lastOne(who, card) {
  return async (d) => {
    for (let i = 0; i < 60; i++) {
      const x = await d.deciding();
      if (!x) throw new Error('lastOne: nobody is deciding');
      const left = pileCount(x.view, card);
      if (x.name === who && x.decision.kind === 'turn' && left === 1) return;
      await d.turn(x.name === who && left > 1 ? [card, 'Silver'] : ['Silver']);
    }
    throw new Error(`lastOne: ${card} never ran down to 1`);
  };
}

/// `n` turns, each buying the first of `once` it can that its player hasn't bought yet,
/// else the first of `then`; what is asked after buying X picks `piles[X]` if it can.
function buyEach(n, once, then, piles = {}) {
  return async (d) => {
    const bought = {};
    const answer = d.answer.bind(d);
    d.answer = async (dd, indices) => {
      for (const i of indices) {
        const o = (dd.decision.options ?? []).find((o) => o.index === i);
        if (o && (o.action === 'buy' || o.action === 'buy-landscape')) {
          (bought[dd.name] ??= new Set()).add(o.card);
          d.prefer = piles[o.card] ? [piles[o.card]] : [];
        }
      }
      return answer(dd, indices);
    };
    try {
      for (let i = 0; i < n; i++) {
        const dd = await d.deciding();
        if (!dd) return;
        const done = bought[dd.name] ?? new Set();
        await d.turn([...once.filter((c) => !done.has(c)), ...then]);
      }
    } finally {
      d.answer = answer;
    }
  };
}

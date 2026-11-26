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

  // Table shapes, turn states, game ends and windows (parity round 3).
  {
    name: 'viewport-4x3',
    about: 'a 1280×1024 window: a squarer table, the Buy phase',
    viewport: '1280x1024',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 6 Gold, Silver', 'until Alice Gold+Silver: Gold, Silver', 'treasures'],
  },
  {
    name: 'viewport-ultrawide',
    about: 'a 3440×1440 window: the widest table, the Buy phase',
    viewport: '3440x1440',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 6 Gold, Silver', 'until Alice Gold+Silver: Gold, Silver', 'treasures'],
  },
  {
    name: 'viewport-720',
    about: 'a 1280×720 window: the smallest table, a few turns in',
    viewport: '1280x720',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 10 Gold, Laboratory, Market, Silver', 'act'],
  },
  {
    name: 'long-names',
    about: 'three players with the longest names, two sharing their initials',
    players: 3,
    names: ['Maximilian Wolfgang', 'Maximiliana Wolfe-X', 'Qi'],
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 6 Gold, Laboratory, Silver', 'act'],
  },
  {
    name: 'cleanup',
    about: "a Clean-up decision: Alchemist going back on the deck",
    spec: { kingdom: ['Alchemist', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop'] },
    steps: ['turns 4 Alchemist, Potion, Silver', 'until Alice Alchemist+Potion: Alchemist, Potion, Silver', 'play Alchemist', 'end', 'treasures', 'end'],
  },
  {
    name: 'outpost-turn',
    about: "an extra turn: Alice's Outpost turn",
    spec: { kingdom: ['Outpost', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop'] },
    steps: ['turns 4 Outpost, Silver', 'until Alice Outpost: Outpost, Silver', 'play Outpost', 'treasures', 'end', 'act'],
  },
  {
    name: 'villagers',
    about: 'no Actions left, Villagers to spend, an Action in hand',
    spec: { kingdom: ['Recruiter', 'Smithy', 'Baker', 'Village', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory'] },
    steps: [
      'turns 8 Recruiter, Smithy, Silver',
      (d) => d.until((x) => x.name === 'Alice' && x.view.you.tokens?.villagers > 0, ['Recruiter', 'Smithy', 'Silver']),
      'until Alice Smithy+Smithy: Recruiter, Smithy, Silver',
      'play Smithy',
    ],
  },
  {
    name: 'coffers-spent',
    about: 'Coffers spent in the Buy phase: $ raised, the Coffers count down',
    spec: { kingdom: ['Baker', 'Candlestick Maker', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory'] },
    steps: ['turns 8 Baker, Silver', 'act', 'pick Spend a Coffers'],
  },
  {
    name: 'debt-owed',
    about: 'Debt owed in the Buy phase: buying blocked until it is paid',
    spec: { kingdom: ['Engineer', 'City Quarter', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory'] },
    steps: ['turns 2 Silver', 'turn Engineer', 'turn Silver', 'act'],
  },
  {
    name: 'debt-paid',
    about: 'Debt being paid off with $',
    spec: { kingdom: ['Engineer', 'City Quarter', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory'] },
    steps: ['turns 2 Silver', 'turn Engineer', 'turn Silver', 'act', 'pick Pay off', 'pick Pay off'],
  },
  {
    name: 'provinces-end',
    about: 'the end of a three-player game: Alice buys the last Province',
    players: 3,
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: [lastOne('Province', 8, ['Gold', 'Laboratory', 'Silver'])],
    after: ['buy Province'],
  },
  {
    name: 'colonies-end',
    about: 'the end of the game: Alice buys the last Colony',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'], colonies: true },
    steps: [lastOne('Colony', 11, ['Platinum', 'Gold', 'Laboratory', 'Silver'])],
    after: ['buy Colony'],
  },
  {
    name: 'resigned',
    about: 'the end of the game after Bob resigned',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 6 Gold, Laboratory, Silver'],
    after: ['resign Bob'],
  },
  {
    name: 'tie',
    about: 'the end of a tie: equal points and turns',
    // Every turn buys one card costing $0 until Curses, Ruins and Coppers are gone: 66
    // turns, Bob's last.
    spec: { kingdom: ['Marauder', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop'] },
    steps: ['turns 40 Curse, Abandoned Mine, Ruined Library, Ruined Market, Ruined Village, Survivors, Copper'],
    after: ['turns 40 Curse, Abandoned Mine, Ruined Library, Ruined Market, Ruined Village, Survivors, Copper'],
  },
  {
    name: 'study',
    about: 'the study view: a Kingdom card right-clicked',
    spec: { kingdom: ['Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop', 'Remodel'] },
    steps: ['turns 4 Silver', 'act'],
    show: [{ study: 'Laboratory' }],
  },
  {
    name: 'trash-window',
    about: 'the trash with cards in it, its window open',
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 4 Chapel, Silver', 'until Alice Chapel: Silver', 'prefer Estate, Copper', 'play Chapel', 'act'],
    show: [{ tab: 'Trash' }],
  },
  {
    name: 'mat-window',
    about: "a mat's window: Native Village's",
    spec: { kingdom: ['Native Village', 'Island', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory'] },
    steps: ['turns 12 Native Village, Island, Gold, Silver', 'act'],
    show: [{ at: '[data-anchor="zone:0:nativeVillage"]' }],
  },
  {
    name: 'picked-cards',
    about: 'cards picked while choosing: two of four for Chapel',
    spec: { kingdom: ['Cellar', 'Chapel', 'Workshop', 'Remodel', 'Pawn', 'Courtier', 'Sentry', 'Moat', 'Militia', 'Village'] },
    steps: ['turns 4 Remodel, Chapel', 'until Alice Chapel', 'play Chapel'],
    show: [{ click: 'Estate', hand: true }, { click: 'Copper', hand: true }],
  },

  // Player state and zones (parity round 3).
  {
    name: 'player-tokens',
    about: 'VP tokens and Debt on the players, the −1 Card and −$1 tokens taken, the Journey token turned over',
    spec: {
      kingdom: ['Monument', 'Goons', 'Engineer', 'Relic', 'Bridge Troll', 'Ranger', 'Village', 'Smithy', 'Market', 'Cellar'],
    },
    steps: [
      'turns 4 Monument, Ranger, Silver',
      'turns 6 Relic, Bridge Troll, Monument, Silver',
      'turn Engineer',
      (d) => d.until((x) => x.name === 'Alice' && attackTokens(x.view.you?.tokens), ['Relic', 'Bridge Troll', 'Silver']),
    ],
  },
  {
    name: 'boons-kept',
    about: 'a Boon kept until Clean-up (The Field\'s, Forest\'s or River\'s Gift)',
    spec: {
      kingdom: ['Bard', 'Blessed Village', 'Sacred Grove', 'Idol', 'Skulk', 'Werewolf', 'Village', 'Smithy', 'Market', 'Festival'],
    },
    steps: [
      'villages',
      'turns 8 Bard, Sacred Grove, Skulk, Silver',
      (d) => d.until((x) => x.name === 'Alice' && x.view.you?.boons?.length, ['Gold', 'Werewolf', 'Silver']),
    ],
  },
  {
    name: 'hexes',
    about: 'Hexes received: Deluded or Envious until the Buy phase, Miserable, a Plague Curse in hand',
    spec: {
      kingdom: ['Skulk', 'Cursed Village', 'Leprechaun', 'Werewolf', 'Tormentor', 'Village', 'Smithy', 'Market', 'Festival', 'Cellar'],
    },
    steps: [
      'villages',
      'turns 8 Skulk, Tormentor, Werewolf, Silver',
      (d) => d.until((x) => x.name === 'Alice' && x.view.turn.phase === 'action' && x.view.you?.states?.some((s) => s === 'Deluded' || s === 'Envious'), ['Gold', 'Skulk', 'Silver']),
    ],
  },
  {
    name: 'heirlooms',
    about: "every Heirloom in the starting deck, some in the first turn's hand",
    spec: {
      kingdom: ['Pooka', 'Secret Cave', 'Shepherd', 'Cemetery', 'Tracker', 'Pixie', 'Fool', 'Village', 'Smithy', 'Market'],
    },
    steps: [],
  },
  {
    name: 'set-aside',
    about: 'cards set aside by Prince, Summon, Cargo Ship, Crypt, Research, Haven, Gear, Church and Archive',
    spec: {
      kingdom: ['Crypt', 'Cargo Ship', 'Prince', 'Research', 'Haven', 'Gear', 'Church', 'Archive', 'Village', 'Smithy'],
      landscapes: ['Summon'],
    },
    steps: [
      'nights',
      'prefer Copper, Estate, Village, Smithy',
      'turns 8 Gold, Silver',
      'spree 14 Prince, Summon, Crypt, Research, Cargo Ship, Gold, Silver',
      (d) => d.until((x) => x.name === 'Alice' && x.view.lasting?.some((l) => l.card === 'Prince' && l.owner === 0), ['Summon', 'Crypt', 'Cargo Ship', 'Gold', 'Silver'], { spree: true }),
      'act',
    ],
  },
  {
    name: 'thin-decks',
    about: 'a big trash after Chapels, thin decks: an empty deck, discard pile or hand',
    spec: {
      kingdom: ['Chapel', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory', 'Workshop'],
    },
    steps: ['prefer Estate, Copper', 'turns 14 Chapel, Gold, Silver', 'act'],
  },
  {
    name: 'looked-at',
    about: "cards looked at off the deck: Fortune Hunter offering a Treasure among them",
    spec: {
      kingdom: ['Cage', 'Grotto', 'Fortune Hunter', 'Sailor', 'Shaman', 'Buried Treasure', 'Cabin Boy', 'Crew', 'Flagship', 'Harbor Village'],
    },
    steps: [
      'turns 6 Fortune Hunter, Silver',
      (d) => d.until((x) => x.name === 'Alice' && x.view.turn.phase === 'action' && x.view.you.hand.known['Fortune Hunter'] && x.view.you.deck.count >= 5, ['Fortune Hunter', 'Silver']),
      'play Fortune Hunter',
    ],
  },
  {
    name: 'champion',
    about: 'Travellers exchanged up to Champion and Teacher, staying in play',
    spec: {
      kingdom: ['Page', 'Peasant', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory'],
    },
    steps: ['villages', 'prefer Exchange', 'spree 40 Page, Peasant, Village', 'act'],
  },
  {
    name: 'throne-duration',
    about: 'Durations played by Throne Room and King\'s Court, kept in play with them into the next turn',
    spec: {
      kingdom: ['Throne Room', "King's Court", 'Caravan', 'Fishing Village', 'Wharf', 'Merchant Ship', 'Village', 'Smithy', 'Market', 'Cellar'],
      colonies: true,
    },
    steps: [
      'prefer Caravan, Fishing Village, Wharf, Merchant Ship',
      'turns 10 King\'s Court, Throne Room, Caravan, Wharf, Silver',
      'until Alice Throne Room+Caravan: Throne Room, Caravan, Fishing Village, Silver',
      'play Throne Room',
      'pick Caravan',
      'turn Silver',
      'turn Silver',
    ],
  },
  {
    name: 'mat-counters',
    about: "Pirate Ship's coins, Sinister Plot's tokens and the Trade Route mat",
    spec: {
      kingdom: ['Pirate Ship', 'Trade Route', 'Village', 'Smithy', 'Market', 'Moat', 'Militia', 'Cellar', 'Festival', 'Laboratory'],
      landscapes: ['Sinister Plot'],
    },
    steps: ['prefer Trash', 'turns 16 Sinister Plot, Pirate Ship, Trade Route, Duchy, Silver, Estate', 'act'],
  },
  {
    name: 'nocturne-nights',
    about: 'Night cards played: Night Durations in play (Cobbler, Den of Sin, Guardian, Ghost Town, Raider), Vampire, Exorcist',
    spec: {
      kingdom: ['Cobbler', 'Crypt', 'Den of Sin', 'Guardian', 'Ghost Town', 'Raider', 'Vampire', 'Exorcist', 'Necromancer', 'Sacred Grove'],
    },
    steps: ['nights', 'prefer Copper, Estate', 'spree 16 Vampire, Raider, Crypt, Sacred Grove, Den of Sin, Necromancer, Guardian, Ghost Town, Exorcist, Cobbler', 'act'],
  },
  {
    name: 'renaissance-played',
    about: 'Renaissance cards played and Projects bought: Canal lowering costs, Acting Troupe, Silk Merchant, Spices',
    spec: {
      kingdom: ['Acting Troupe', 'Cargo Ship', 'Research', 'Silk Merchant', 'Spices', 'Ducat', 'Lackeys', 'Sculptor', 'Villain', 'Seer'],
      landscapes: ['Canal', 'Exploration', 'Guildhall', 'Silos'],
    },
    steps: ['turn Exploration', 'turn Exploration', 'turns 4 Canal, Guildhall, Silos, Silver', 'turns 12 Spices, Seer, Silk Merchant, Acting Troupe, Villain, Research, Lackeys, Silver', 'act'],
  },
  {
    name: 'menagerie-played',
    about: 'Menagerie: variable costs on the piles (Destrier, Fisherman, Wayfarer, Animal Fair), Horses gained, Barge and Gatekeeper in play',
    spec: {
      kingdom: ['Barge', 'Cavalry', 'Coven', 'Destrier', 'Fisherman', 'Gatekeeper', 'Groom', 'Paddock', 'Wayfarer', 'Animal Fair'],
      landscapes: ['Ride', 'Bargain', 'Way of the Worm', 'Enclave'],
    },
    steps: ['prefer Horse', 'spree 16 Gatekeeper, Barge, Cavalry, Coven, Paddock, Groom, Ride, Silver', 'act'],
  },
  {
    name: 'plunder-durations',
    about: 'Plunder Durations in play, Traits on piles, Events (Deliver, Foray)',
    spec: {
      kingdom: ['Abundance', 'Cutthroat', 'Enlarge', 'Frigate', 'Gondola', 'Landing Party', 'Longship', 'Quartermaster', 'Rope', 'Secluded Shrine'],
      landscapes: ['Deliver', 'Foray', 'Hasty', 'Patient'],
    },
    steps: ['spree 16 Longship, Frigate, Cutthroat, Quartermaster, Enlarge, Rope, Gondola, Secluded Shrine, Abundance, Silver', 'act'],
  },
  {
    name: 'plunder-more',
    about: 'Plunder: Siren, Stowaway, Taskmaster, Trickster, Wealthy Village, Mapmaker played; Tireless and Fated',
    spec: {
      kingdom: ['Siren', 'Stowaway', 'Taskmaster', 'Trickster', 'Wealthy Village', 'Mapmaker', 'Buried Treasure', 'Cabin Boy', 'Crew', 'Grotto'],
      landscapes: ['Tireless', 'Fated', 'Invasion', 'Prosper'],
    },
    steps: ['villages', 'spree 16 Crew, Wealthy Village, Taskmaster, Siren, Trickster, Grotto, Stowaway, Cabin Boy, Mapmaker, Buried Treasure, Silver', 'act'],
  },
  {
    name: 'rising-sun-shadows',
    about: 'Shadow cards in the deck and played, Samurai and Snake Witch, a Prophecy',
    spec: {
      kingdom: ['Alley', 'Fishmonger', 'Ninja', 'Ronin', 'Tanuki', 'Samurai', 'Snake Witch', 'Artist', 'Craftsman', 'Gold Mine'],
      landscapes: ['Credit', 'Biding Time'],
    },
    steps: ['villages', 'spree 14 Samurai, Ninja, Ronin, Tanuki, Snake Witch, Alley, Fishmonger, Silver', 'act'],
  },
  {
    name: 'rising-sun-debt',
    about: 'Rising Sun debt: Change, Litter, Root Cellar, Imperial Envoy, Daimyo bought and played; Debt owed',
    spec: {
      kingdom: ['Change', 'Daimyo', 'Imperial Envoy', 'Litter', 'Root Cellar', 'Village', 'Smithy', 'Market', 'Festival', 'Laboratory'],
      landscapes: ['Continue', 'Harsh Winter'],
    },
    steps: ['villages', 'turns 12 Daimyo, Imperial Envoy, Litter, Root Cellar, Change, Silver', 'act'],
  },
  {
    name: 'adventures-played',
    about: 'Adventures: Miser\'s mat, Swamp Hag, Wine Merchant, Giant and the Journey token, Events putting tokens on piles',
    spec: {
      kingdom: ['Giant', 'Miser', 'Swamp Hag', 'Wine Merchant', 'Storyteller', 'Magpie', 'Messenger', 'Artificer', 'Port', 'Treasure Trove'],
      landscapes: ['Ferry', 'Plan', 'Pathfinding', 'Training'],
    },
    steps: ['turns 8 Pathfinding, Training, Plan, Ferry, Silver', 'turns 12 Giant, Swamp Hag, Treasure Trove, Wine Merchant, Miser, Port, Magpie, Silver', 'act'],
  },
  {
    name: 'adventures-attacks',
    about: 'Adventures: Bridge Troll, Caravan Guard, Raze, Ranger, Lost City, Relic played; Ball, Borrow, Save, Raid',
    spec: {
      kingdom: ['Bridge Troll', 'Caravan Guard', 'Raze', 'Ranger', 'Lost City', 'Relic', 'Haunted Woods', 'Village', 'Smithy', 'Market'],
      landscapes: ['Ball', 'Borrow', 'Save', 'Raid'],
    },
    steps: ['villages', 'turns 12 Bridge Troll, Relic, Lost City, Caravan Guard, Ranger, Raze, Silver', 'turn Raid', 'turn Raid', 'act'],
  },
  {
    name: 'nocturne-collect',
    about: 'Nocturne cards played over a game, Night cards at night: Conclave, Cobbler, Den of Sin, Raider, Werewolf, Tormentor, Changeling',
    spec: {
      kingdom: ['Conclave', 'Cobbler', 'Den of Sin', 'Raider', 'Werewolf', 'Tormentor', 'Sacred Grove', 'Faithful Hound', 'Changeling', 'Monastery'],
    },
    steps: ['villages', 'nights', 'turns 6 Gold, Silver', (d) => collect(d, 20, ['Conclave', 'Cobbler', 'Den of Sin', 'Raider', 'Werewolf', 'Tormentor', 'Sacred Grove', 'Faithful Hound', 'Monastery']), 'act'],
  },
  {
    name: 'renaissance-collect',
    about: 'Renaissance cards played over a game, Projects bought (Piazza, Canal, Guildhall, Exploration)',
    spec: {
      kingdom: ['Mountain Village', 'Patron', 'Old Witch', 'Improve', 'Experiment', 'Priest', 'Scholar', 'Inventor', 'Lackeys', 'Villain'],
      landscapes: ['Piazza', 'Canal', 'Guildhall', 'Exploration'],
    },
    steps: ['villages', (d) => collect(d, 18, ['Canal', 'Piazza', 'Mountain Village', 'Patron', 'Old Witch', 'Improve', 'Experiment', 'Priest', 'Scholar', 'Inventor', 'Lackeys', 'Villain', 'Guildhall']), 'act'],
  },
  {
    name: 'menagerie-collect',
    about: 'Menagerie cards played over a game: Village Green, Barge, Falconer, Hostelry, Mastermind, Sanctuary, Scrap, Sheepdog, Destrier, Coven; Ways and Events',
    spec: {
      kingdom: ['Village Green', 'Barge', 'Falconer', 'Hostelry', 'Mastermind', 'Sanctuary', 'Scrap', 'Sheepdog', 'Destrier', 'Coven'],
      landscapes: ['Way of the Camel', 'Way of the Worm', 'Banish', 'Delay'],
    },
    steps: ['villages', (d) => collect(d, 18, ['Village Green', 'Barge', 'Falconer', 'Hostelry', 'Mastermind', 'Sanctuary', 'Scrap', 'Sheepdog', 'Destrier', 'Coven']), 'act'],
  },
  {
    name: 'plunder-collect',
    about: 'Plunder cards played over a game: Wealthy Village, Gondola, Frigate, Landing Party, Quartermaster, Siren, Stowaway, Trickster, Enlarge, Abundance',
    spec: {
      kingdom: ['Wealthy Village', 'Gondola', 'Frigate', 'Landing Party', 'Quartermaster', 'Siren', 'Stowaway', 'Trickster', 'Enlarge', 'Abundance'],
      landscapes: ['Prosper', 'Prepare', 'Cursed', 'Tireless'],
    },
    steps: ['villages', (d) => collect(d, 18, ['Wealthy Village', 'Gondola', 'Frigate', 'Landing Party', 'Quartermaster', 'Siren', 'Stowaway', 'Trickster', 'Enlarge', 'Abundance']), 'act'],
  },
  {
    name: 'rising-sun-collect',
    about: 'Rising Sun cards played over a game: Shadows, Debt costs, Samurai; an Event and a Prophecy',
    spec: {
      kingdom: ['Alley', 'Artist', 'Craftsman', 'Fishmonger', 'Gold Mine', 'Imperial Envoy', 'Ronin', 'Root Cellar', 'Samurai', 'Tanuki'],
      landscapes: ['Foresight', 'Kind Emperor'],
    },
    steps: ['villages', (d) => collect(d, 22, ['Samurai', 'Alley', 'Fishmonger', 'Ronin', 'Tanuki', 'Artist', 'Craftsman', 'Gold Mine', 'Imperial Envoy', 'Root Cellar']), 'act'],
  },
  {
    name: 'adventures-collect',
    about: 'Adventures cards played over a game: Artificer, Giant, Magpie, Messenger, Port, Storyteller, Treasure Trove, Wine Merchant, Lost City, Swamp Hag',
    spec: {
      kingdom: ['Artificer', 'Giant', 'Magpie', 'Messenger', 'Port', 'Storyteller', 'Treasure Trove', 'Wine Merchant', 'Lost City', 'Swamp Hag'],
      landscapes: ['Ball', 'Borrow', 'Pilgrimage', 'Travelling Fair'],
    },
    steps: ['villages', (d) => collect(d, 18, ['Lost City', 'Giant', 'Swamp Hag', 'Artificer', 'Magpie', 'Messenger', 'Port', 'Storyteller', 'Treasure Trove', 'Wine Merchant', 'Pilgrimage']), 'act'],
  },
];

// Plays until one `card` is left and Alice, in her Buy phase with her Treasures played,
// has $`price` for it.
function lastOne(card, price, buys) {
  const left = (d) => d.view.supply.find((p) => p.card === card)?.count;
  return async (driver) => {
    const treasures = (d) => d.decision.options.some((o) => o.action === 'play' && driver.is(o.card, 'Treasure'));
    await driver.until((d) => d.name === 'Alice' && left(d) <= 1, [card, ...buys], { spree: true });
    await driver.until(
      (d) => d.name === 'Alice' && d.decision.phase === 'buy' && !treasures(d) && d.view.turn.coins >= price,
      buys,
    );
  };
}


function attackTokens(t) {
  return t?.minusCard && t?.minusCoin;
}

/// `n` turns, each buying the first card of `cards` its player doesn't have yet (else a
/// Silver): a game that plays many different cards.
async function collect(d, n, cards) {
  for (let i = 0; i < n; i++) {
    const x = await d.deciding();
    if (!x) return;
    const owned = x.view.you?.owned?.known ?? {};
    await d.turn([...cards.filter((c) => !owned[c]), 'Silver']);
  }
}

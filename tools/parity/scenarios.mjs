// The positions both clients are compared at. Each is a lobby table (`spec`, dealt with
// the run's fixed seed) and the steps that lead from the start to the position (see
// drive.mjs). Only cards the dominion.games transport supports can be used: see
// `GET /api/official/coverage`.
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
];

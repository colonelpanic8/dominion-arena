// Card coverage: a game per batch of ten Kingdom cards, each player buying one of each
// (dearest first) and playing what it draws, with a few landscapes, an Ally and a
// Prophecy on the table. Ways and Allies are used whenever they can be (`prefer`).
import { buyEach, playAsking } from '../scenarios.mjs';

/// Turns, each buying the dearest card of `once` its player hasn't bought yet, else a
/// Gold or Silver.
function buyDearest(n, once, prefer = []) {
  return async (d) => {
    const { json } = await d.arena.api('GET', '/cards');
    const cost = new Map(json.map((c) => [c.name, c.cost ? c.cost.coins + 2 * c.cost.potions + c.cost.debt : 0]));
    const sorted = [...once].sort((a, b) => cost.get(b) - cost.get(a));
    await buyEach(n, sorted, ['Gold', 'Silver'], {}, prefer)(d);
  };
}

/// Turns until Alice starts one.
async function aliceNext(d) {
  for (let i = 0; i < 4; i++) {
    const x = await d.deciding();
    if (!x || (x.name === 'Alice' && x.decision.kind === 'turn')) return;
    await d.turn(['Gold', 'Silver']);
  }
}

const BUYABLE = new Set(['event', 'project']);

/// A batch: `kingdom` and `landscapes` (Events and Projects bought too), `n` turns.
function batch(name, about, kingdom, landscapes = [], { n = 32, colonies = false, prefer = [], skip = [], first = [] } = {}) {
  return {
    name,
    about,
    spec: { kingdom, landscapes, ...(colonies ? { colonies: true } : {}) },
    steps: [
      ...first,
      async (d) => {
        const { json } = await d.arena.api('GET', '/cards');
        const kind = new Map(json.map((c) => [c.name, c.kind]));
        const once = [...kingdom, ...landscapes.filter((l) => BUYABLE.has(kind.get(l)))].filter((c) => !skip.includes(c));
        const ways = landscapes.filter((l) => kind.get(l) === 'way' || kind.get(l) === 'ally');
        await buyDearest(n, once, [...ways, ...prefer])(d);
      },
      aliceNext,
      'act',
    ],
  };
}

const K = {
  base: ['Bandit', 'Bureaucrat', 'Council Room', 'Gardens', 'Merchant', 'Mine', 'Moneylender', 'Poacher', 'Witch', 'Workshop'],
  intrigue: ['Baron', 'Conspirator', 'Courtyard', 'Duke', 'Farm', 'Ironworks', 'Mill', 'Replace', 'Shanty Town', 'Steward'],
  seaside: ['Astrolabe', 'Blockade', 'Corsair', 'Monkey', 'Pirate', 'Sailor', 'Sea Chart', 'Sea Witch', 'Tide Pools', 'Tactician'],
  seaside2: ['Bazaar', 'Cutpurse', 'Explorer', 'Salvager', 'Sea Hag', 'Smugglers', 'Treasure Map', 'Upgrade', 'Squire', 'Warehouse'],
  alchemy: ['Familiar', 'Herbalist', "Philosopher's Stone", 'Transmute', 'Vineyard', 'Alchemist', 'Apprentice', 'Golem', 'University', 'Scrying Pool'],
  prosperity: ['Anvil', 'Bank', 'Collection', 'Hoard', 'Mint', 'Counting House', 'Rabble', 'Mountebank', 'Goons', "Worker's Village"],
  prosperity2: ['Bishop', 'City', 'Expand', 'Forge', 'Grand Market', "King's Court", 'Peddler', 'Monument', 'Venture', 'Quarry'],
  cornucopia: ['Hamlet', 'Harvest', 'Horn of Plenty', 'Hunting Party', 'Jester', 'Menagerie', 'Remake', 'Tournament', 'Fairgrounds', 'Joust'],
  guilds: ['Butcher', 'Candlestick Maker', 'Doctor', 'Farrier', 'Infirmary', 'Merchant Guild', 'Plaza', 'Soothsayer', 'Stonemason', 'Taxman'],
  hinterlands: ['Berserker', 'Border Village', 'Cache', 'Cauldron', 'Crossroads', 'Develop', 'Duchess', 'Farmland', "Fool's Gold", 'Haggler'],
  hinterlands2: ['Ill-Gotten Gains', 'Jack of All Trades', 'Margrave', 'Noble Brigand', 'Nomad Camp', 'Oasis', 'Scheme', 'Silk Road', 'Souk', 'Spice Merchant'],
  hinterlands3: ['Stables', 'Trader', 'Trail', 'Tunnel', 'Weaver', 'Wheelwright', 'Nomads', 'Tragic Hero', 'Goatherd', 'Hunting Lodge'],
  darkAges: ['Bandit Camp', 'Beggar', 'Feodum', 'Forager', 'Graverobber', 'Hermit', 'Pillage', 'Rats', 'Rebuild', 'Urchin'],
  menagerie: ['Cardinal', 'Displace', 'Kiln', 'Snowy Village', 'Animal Fair', 'Bounty Hunter', 'Fisherman', 'Wayfarer', 'Livery', 'Supplies'],
  empires: ['Chariot Race', 'City Quarter', 'Encampment', 'Forum', 'Overlord', 'Royal Blacksmith', 'Temple', 'Villa', 'Wild Hunt', 'Castles'],
  plunder: ['Crucible', 'Figurine', 'First Mate', "King's Cache", 'Maroon', 'Mining Road', 'Pendant', 'Pilgrim', 'Silver Mine', 'Swamp Shacks'],
  plunder2: ['Tools', 'Cage', 'Flagship', 'Harbor Village', 'Shaman', 'Grotto', 'Fortune Hunter', 'Crew', 'Buried Treasure', 'Cabin Boy'],
  risingSun: ['Aristocrat', 'Rice', 'Rustic Village', 'Poet', 'Change', 'Litter', 'Gold Mine', 'Imperial Envoy', 'Craftsman', 'Root Cellar'],
  renaissance: ['Ducat', 'Hideout', 'Sculptor', 'Swashbuckler', 'Mountain Village', 'Patron', 'Improve', 'Experiment', 'Lackeys', 'Cargo Ship'],
  nocturne: ['Blessed Village', 'Druid', 'Idol', 'Leprechaun', 'Pixie', 'Pooka', 'Secret Cave', 'Shepherd', 'Tracker', 'Sacred Grove'],
  nocturne2: ['Cemetery', "Devil's Workshop", 'Crypt', 'Ghost Town', 'Raider', 'Tormentor', 'Werewolf', 'Exorcist', 'Night Watchman', 'Conclave'],
  allies: ['Augurs', 'Clashes', 'Forts', 'Townsfolk', 'Capital City', 'Courier', 'Galleria', 'Hunter', 'Modify', 'Swap'],
  adventures: ['Transmogrify', 'Raze', 'Ranger', 'Port', 'Storyteller', 'Treasure Trove', 'Wine Merchant', 'Artificer', 'Dismantle', 'Walled Village'],
};

export const CARDS = [
  batch('base-play', 'Base: Bandit, Bureaucrat, Council Room, Witch, Mine and the rest bought and played; Sickness, an Ally', K.base, ['Alliance', 'Commerce', 'Circle of Witches', 'Sickness']),
  batch('intrigue-play', 'Intrigue: Baron, Courtyard, Mill, Replace, Steward and the rest bought and played; Progress, an Ally', K.intrigue, ['Demand', 'Desperation', 'City-state', 'Progress']),
  batch('seaside-play', "Seaside's second-edition Durations bought and played: Astrolabe, Blockade, Corsair, Monkey, Pirate, Sea Witch, Tide Pools", K.seaside, ['Enhance', 'Gamble', 'Coastal Haven', 'Bureaucracy']),
  batch('seaside-play-2', 'Seaside: Bazaar, Cutpurse, Explorer, Salvager, Sea Hag, Smugglers, Treasure Map bought and played', K.seaside2, ['Invest', 'March', "Crafters' Guild", 'Divine Wind']),
  batch('alchemy-play', 'Alchemy: Familiar, Herbalist, Philosopher\'s Stone, Transmute, Vineyard with Potions', K.alchemy, ['Populate', 'Pursue', 'Desert Guides', 'Growth'], { first: ['turns 4 Potion, Silver'] }),
  batch('prosperity-play-2', 'Prosperity: Anvil, Bank, Collection, Hoard, Mint, Counting House, Rabble, Mountebank, Goons, Worker\'s Village', K.prosperity, ['Reap', 'Seize the Day', 'Fellowship of Scribes', 'Panic'], { colonies: true }),
  batch('prosperity-play-3', "Prosperity: Bishop, City, Expand, Forge, Grand Market, King's Court, Peddler", K.prosperity2, ['Stampede', 'Toil', 'Forest Dwellers', 'Flourishing Trade'], { colonies: true }),
  batch('cornucopia-play', 'Cornucopia: Hamlet, Harvest, Horn of Plenty, Hunting Party, Jester, Menagerie, Remake, Tournament, Joust; two Ways', K.cornucopia, ['Way of the Butterfly', 'Way of the Chameleon', 'Gang of Pickpockets']),
  batch('guilds-play', 'Guilds: Butcher, Candlestick Maker, Doctor, Farrier, Infirmary, Merchant Guild, Plaza, Soothsayer, Stonemason, Taxman; two Ways', K.guilds, ['Way of the Frog', 'Way of the Goat', 'Island Folk']),
  batch('hinterlands-play', "Hinterlands: Berserker, Border Village, Cache, Cauldron, Crossroads, Develop, Duchess, Fool's Gold, Haggler; two Ways", K.hinterlands, ['Way of the Monkey', 'Way of the Mule', 'League of Bankers']),
  batch('hinterlands-play-2', 'Hinterlands: Ill-Gotten Gains, Jack of All Trades, Margrave, Noble Brigand, Oasis, Scheme, Souk, Spice Merchant; two Ways', K.hinterlands2, ['Way of the Otter', 'Way of the Owl', 'Market Towns']),
  batch('hinterlands-play-3', 'Hinterlands: Stables, Trader, Trail, Tunnel, Weaver, Wheelwright, Nomads; Tragic Hero, Goatherd, Hunting Lodge; two Ways', K.hinterlands3, ['Way of the Rat', 'Way of the Sheep', 'Mountain Folk', 'Rapid Expansion']),
  batch('dark-ages-play-3', 'Dark Ages: Bandit Camp, Beggar, Feodum, Forager, Graverobber, Hermit, Pillage, Rats, Rebuild, Urchin; two Ways', K.darkAges, ['Way of the Squirrel', 'Way of the Turtle', 'Order of Astrologers']),
  batch('menagerie-play', 'Menagerie: Cardinal, Displace, Kiln, Snowy Village, Animal Fair, Bounty Hunter, Fisherman, Wayfarer, Livery, Supplies', K.menagerie, ['Way of the Pig', 'Way of the Seal', 'Banish', 'Bargain', 'Order of Masons']),
  batch('empires-play-2', 'Empires: Chariot Race, City Quarter, Encampment, Forum, Overlord, Royal Blacksmith, Temple, Villa, Wild Hunt, Castles; Landmarks', K.empires, ['Bandit Fort', 'Fountain', 'Keep', 'Labyrinth', 'Conquest', 'Delve', 'Peaceful Cult']),
  batch('plunder-play-2', "Plunder's Treasures and Actions: Crucible, Figurine, First Mate, King's Cache, Maroon, Mining Road, Pendant, Pilgrim, Silver Mine, Swamp Shacks", K.plunder, ['Avoid', 'Bury', 'Journey', 'Fawning', 'Plateau Shepherds']),
  batch('plunder-play-3', 'Plunder: Tools, Cage, Flagship, Harbor Village, Shaman, Grotto, Fortune Hunter, Crew, Buried Treasure, Cabin Boy', K.plunder2, ['Launch', 'Maelstrom', 'Mirror', 'Friendly', "Trappers' Lodge"]),
  batch('rising-sun-play-2', 'Rising Sun: Aristocrat, Rice, Omens taking Sun off Enlightenment, Debt cards; Events', K.risingSun, ['Amass', 'Gather', 'Kintsugi', 'Enlightenment', "Woodworkers' Guild"]),
  batch('renaissance-play-2', 'Renaissance: Ducat, Hideout, Sculptor, Swashbuckler, Cargo Ship; Projects bought', K.renaissance, ['Capitalism', 'Citadel', 'City Gate', 'Crop Rotation', 'League of Shopkeepers']),
  batch('nocturne-play-2', 'Nocturne: Fate cards receiving Boons, Doom cards giving Hexes, Heirlooms, Secret Cave', K.nocturne, ['Fleet', 'Innovation', 'Sewers', 'Star Chart']),
  batch('nocturne-play-3', "Nocturne's Night cards played at night: Cemetery, Devil's Workshop, Crypt, Ghost Town, Raider, Exorcist", K.nocturne2, ['Wall', 'Tower', 'Triumphal Arch', 'Ritual', 'Salt the Earth'], { first: ['nights'] }),
  batch('allies-play-2', 'Allies: split piles bought and played (Augurs, Clashes, Forts, Townsfolk), Courier, Modify, Swap', K.allies, ['Wolf Den', 'Windfall', 'Inherited', 'Cave Dwellers']),
  batch('adventures-play-2', 'Adventures: Reserves called, Ranger, Raze, Port, Storyteller; Events bought; Dismantle, Walled Village', K.adventures, ['Mission', 'Scouting Party', 'Trade', 'Alms', 'Bonfire', 'Save']),
  {
    name: 'unplayed-opening-2',
    about: "cards and landscapes whose questions the transport doesn't play yet, at the start",
    spec: {
      kingdom: ['Watchtower', 'Farmhands', 'Scepter', 'Rice Broker', 'Tea House', 'Ghost Ship', 'Embassy', 'Haunted Woods', 'Sleigh', 'Stash'],
      landscapes: ['Road Network', 'Quest', 'Transport', 'Scrounge', 'Asceticism', 'Approaching Army', 'Family of Inventors'],
    },
    steps: [],
  },
  {
    name: 'mountebank-curse',
    about: "a question about a card from hand for another player's Attack: Mountebank's Curse to discard",
    spec: { kingdom: K.prosperity, colonies: true },
    steps: ['turns 6 Mountebank, Silver', 'until Bob Mountebank & Alice Curse: Mountebank, Silver', 'play Mountebank'],
  },
  {
    name: 'doctor-name',
    about: "naming a card: Doctor's",
    spec: { kingdom: K.guilds },
    steps: ['turns 6 Doctor, Silver', playAsking('Doctor', ['Doctor', 'Silver'])],
  },
  {
    name: 'rebuild-name',
    about: "naming a card: Rebuild's",
    spec: { kingdom: K.darkAges },
    steps: ['turns 6 Rebuild, Silver', playAsking('Rebuild', ['Rebuild', 'Silver'])],
  },
  {
    name: 'courtyard-topdeck',
    about: "a card from hand onto the deck: Courtyard",
    spec: { kingdom: K.intrigue },
    steps: ['turns 4 Courtyard, Silver', playAsking('Courtyard', ['Courtyard', 'Silver'])],
  },
  {
    name: 'blockade-gain',
    about: "gaining a card to set aside: Blockade",
    spec: { kingdom: K.seaside },
    steps: ['turns 4 Blockade, Silver', playAsking('Blockade', ['Blockade', 'Silver'])],
  },
  {
    name: 'mint-reveal',
    about: 'revealing a Treasure from hand: Mint',
    spec: { kingdom: K.prosperity, colonies: true },
    steps: ['turns 6 Mint, Silver', playAsking('Mint', ['Mint', 'Silver'])],
  },
];

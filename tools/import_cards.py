#!/usr/bin/env python3
"""One-off import of card data into data/cards/<set>.json.

Sources (not committed):
  - Dominion Online card objects (names, types, expansion, official text), as extracted by
    dominion-navigator: data/dominion-card-shaped-objects.json
  - sumpfork/dominiontabs card_db_src/cards_db.json (costs) and en_us/cards_en_us.json
    (text for first-edition cards Dominion Online no longer has)

Usage: import_cards.py <dominion-online.json> <cards_db.json> <cards_en_us.json> <out-dir>

After the import, data/cards/*.json is the source of truth and is edited by hand.
"""

import json
import re
import sys
from collections import defaultdict
from pathlib import Path


def norm(s):
    return re.sub(r"[^a-z0-9]", "", s.lower())


SETS = {
    "Base": "base",
    "Intrigue": "intrigue",
    "Seaside": "seaside",
    "Alchemy": "alchemy",
    "Prosperity": "prosperity",
    "Cornucopia & Guilds": "cornucopia_guilds",
    "Hinterlands": "hinterlands",
    "Dark Ages": "dark_ages",
    "Adventures": "adventures",
    "Empires": "empires",
    "Nocturne": "nocturne",
    "Renaissance": "renaissance",
    "Menagerie": "menagerie",
    "Allies": "allies",
    "Plunder": "plunder",
    "Rising Sun": "rising_sun",
    "Arcana": "arcana",
    "Promos": "promos",
}

REMOVED_TAGS = {
    "dominion1stEditionRemoved": "base",
    "intrigue1stEditionRemoved": "intrigue",
    "seaside1stEditionRemoved": "seaside",
    "prosperity1stEditionRemoved": "prosperity",
    "hinterlands1stEditionRemoved": "hinterlands",
    "cornucopia1stEditionRemoved": "cornucopia_guilds",
    "guilds1stEditionRemoved": "cornucopia_guilds",
}

SKIP = {
    "Back", "Black Market Pile", "Boon Discardpile", "Boon Drawpile", "Hex Discardpile",
    "Hex Drawpile", "Druid Boons", "MinusCard", "MinusCoin", "State Limbo", "unused",
    "Charlatan Curse", "Pig", "Prize Pile", "Rewards Pile", "Loot Pile",
}

RENAME = {
    "Augur Pile": "Augurs",
    "Clash Pile": "Clashes",
    "Fort Pile": "Forts",
    "Odyssey Pile": "Odysseys",
    "Townsfolk Pile": "Townsfolk",
    "Wizard Pile": "Wizards",
    "Ruin Pile": "Ruins",
    "Kings Castle": "King's Castle",
    "Witchs Apprentice": "Witch's Apprentice",
    "Witchs Tower": "Witch's Tower",
}

GROUPS = {
    "Knights": ["Dame Anna", "Dame Josephine", "Dame Molly", "Dame Natalie", "Dame Sylvia",
                "Sir Bailey", "Sir Destry", "Sir Martin", "Sir Michael", "Sir Vander"],
    "Castles": ["Humble Castle", "Crumbling Castle", "Small Castle", "Haunted Castle",
                "Opulent Castle", "Sprawling Castle", "Grand Castle", "King's Castle"],
    "Ruins": ["Abandoned Mine", "Ruined Library", "Ruined Market", "Ruined Village", "Survivors"],
    "Augurs": ["Herb Gatherer", "Acolyte", "Sorceress", "Sibyl"],
    "Clashes": ["Battle Plan", "Archer", "Warlord", "Territory"],
    "Forts": ["Tent", "Garrison", "Hill Fort", "Stronghold"],
    "Odysseys": ["Old Map", "Voyage", "Sunken Treasure", "Distant Shore"],
    "Townsfolk": ["Town Crier", "Blacksmith", "Miller", "Elder"],
    "Wizards": ["Student", "Conjurer", "Sorcerer", "Lich"],
    # Split piles: the first card is on top and names the pile.
    "Catapult": ["Catapult", "Rocks"],
    "Encampment": ["Encampment", "Plunder"],
    "Gladiator": ["Gladiator", "Fortune"],
    "Patrician": ["Patrician", "Emporium"],
    "Settlers": ["Settlers", "Bustling Village"],
    "Sauna": ["Sauna", "Avanto"],
}

COST_OVERRIDES = {
    **{k: 5 for k in GROUPS["Knights"]},
    "Sir Martin": 4,
    **dict(zip(GROUPS["Castles"], range(3, 11))),
    **{k: 0 for k in GROUPS["Ruins"]},
    **{k: 7 for k in ["Amphora", "Doubloons", "Endless Chalice", "Figurehead", "Hammer",
                      "Insignia", "Jewels", "Orb", "Prize Goat", "Puzzle Box", "Sextant",
                      "Shield", "Spell Scroll", "Staff", "Sword"]},
    "Farm": 6,
}

BASIC = {
    "Copper": (0, 1, 0), "Silver": (3, 2, 0), "Gold": (6, 3, 0), "Platinum": (9, 5, 0),
    "Estate": (2, 0, 1), "Duchy": (5, 0, 3), "Province": (8, 0, 6), "Colony": (11, 0, 10),
    "Curse": (0, 0, -1), "Potion": (4, 0, 0),
}

LANDSCAPE_TYPES = {"Event", "Landmark", "Project", "Way", "Ally", "Trait", "Prophecy", "Study"}
NON_CARD_TYPES = LANDSCAPE_TYPES | {"Boon", "Hex", "State", "Artifact", "Study"}


def plain_text(raw):
    """Dominion Online card markup to plain text."""
    if not raw:
        return ""
    t = raw
    t = re.sub(r"%%?\(([^)]*)\)%%?", r"(\1)", t)
    t = t.replace("%", "")
    t = t.replace("<sun>", "Sun")
    t = re.sub(r"\[!?(\d+)D\]", r"\1 Debt", t)
    t = t.replace("[D]", "Debt")
    t = t.replace("[P]", "Potion")
    t = re.sub(r"\[!?(\d+)\]", r"$\1", t)
    t = t.replace("[]", "$")
    t = re.sub(r"\{!?(-?\d+)\}", r"\1 VP", t)
    t = t.replace("{}", "VP")
    t = re.sub(r"\s*//\s*---\s*//\s*", "\n---\n", t)
    t = re.sub(r"\s*////\s*", "\n", t)
    t = re.sub(r"\s*//\s*", "//", t)
    parts = t.split("//")
    out = parts[0]
    for prev, cur in zip(parts, parts[1:]):
        out += ("\n" if is_bonus_line(prev) or is_bonus_line(cur) else " ") + cur
    t = out
    t = re.sub(r"[ \t]+", " ", t)
    t = t.replace("|", "")
    return "\n".join(line.strip() for line in t.strip().split("\n"))


def is_bonus_line(segment):
    """A segment printed on its own line: a bold bonus like "|+1 Card|" or "|+|$2"."""
    s = segment.split("\n")[-1].strip() if segment else ""
    return bool(re.fullmatch(r"\|[^|]*\|(\$\d+)?|\$\d+|\d+ VP", s))


VANILLA = [
    (r"\+(\d+) Cards?", "cards"),
    (r"\+(\d+) Actions?", "actions"),
    (r"\+(\d+) Buys?", "buys"),
    (r"\+\|?\[(\d+)\]", "coins"),
    (r"\+(\d+) Villagers?", "villagers"),
    (r"\+(\d+) Coffers?", "coffers"),
    (r"\+(\d+) Favors?", "favors"),
]


def vanilla(raw):
    """Bonuses on the card's leading lines, each line holding only bonuses."""
    plus, coins, vp = {}, 0, 0
    if not raw:
        return plus, coins, vp
    for line in re.split(r"//", raw):
        line = line.replace("|", "").strip()
        if not line:
            continue
        if m := re.fullmatch(r"\[!(\d+)\]", line):
            coins = int(m.group(1))
            continue
        if m := re.fullmatch(r"\{!(-?\d+)\}", line):
            vp = int(m.group(1))
            continue
        if line == "---":
            continue
        matched = False
        for pat, key in VANILLA:
            if m := re.fullmatch(pat.replace(r"\|?", ""), line):
                plus[key] = plus.get(key, 0) + int(m.group(1))
                matched = True
                break
        if not matched:
            break
    return plus, coins, vp


def parse_cost(t):
    if t is None:
        return None
    raw = t.get("cost", "")
    if raw == "" and not t.get("potcost") and not t.get("debtcost"):
        return None
    coins = int(re.sub(r"[^0-9]", "", raw) or 0)
    cost = {"coins": coins}
    if t.get("potcost"):
        cost["potions"] = int(t["potcost"])
    if t.get("debtcost"):
        cost["debt"] = int(t["debtcost"])
    if "*" in raw:
        cost["variable"] = True
    if "+" in raw:
        cost["overpay"] = True
    return cost


def tabs_text(desc):
    t = desc.replace("<line>", "\n---\n").replace("<n>", "\n").replace("<br>", "\n")
    t = re.sub(r"\+(\d+) Coins?", r"+$\1", t)
    t = re.sub(r"(\d+) Coins?", r"$\1", t)
    t = t.replace("<*COIN*>", "").replace("<*VP*>", " VP")
    return t.strip()


def main():
    do_path, tabs_path, en_path, out = sys.argv[1:5]
    objects = json.load(open(do_path))["cardShapedObjects"]
    tabs = json.load(open(tabs_path))
    en = json.load(open(en_path))
    by_norm = {norm(t["card_tag"]): t for t in tabs}
    group_of = {m: g for g, ms in GROUPS.items() for m in ms}

    cards = []
    for o in objects:
        name = o["name"]
        if name in SKIP or o["expansion"] is None or name.startswith("Card Of The"):
            continue
        name = RENAME.get(name, name)
        t = by_norm.get(norm(name))
        if t is not None and norm(t["card_tag"]) == norm(name) and " / " not in t["card_tag"]:
            name = t["card_tag"]
        types = list(o["types"])
        removed = False
        if o["expansion"] == "Deprecated":
            tags = t["cardset_tags"] if t else []
            sets = [REMOVED_TAGS[x] for x in tags if x in REMOVED_TAGS]
            if not sets:
                sys.exit(f"no original set for deprecated {name}: {tags}")
            set_, removed = sets[0], True
        else:
            set_ = SETS[o["expansion"]]
        card = {"name": name, "set": set_, "types": types}
        if removed:
            card["removed"] = True
        if o["isBaseCard"]:
            card["basic"] = True
        if name in BASIC:
            c, coins, vp = BASIC[name]
            card["cost"] = {"coins": c}
            if coins:
                card["coins"] = coins
            if vp:
                card["vp"] = vp
            if name == "Potion":
                card["cost"] = {"coins": 4}
                card["potions"] = 1
        elif name in COST_OVERRIDES:
            card["cost"] = {"coins": COST_OVERRIDES[name]}
        elif "Randomizer" not in types:
            cost = parse_cost(t)
            if cost is not None:
                card["cost"] = cost
            elif not (set(types) & NON_CARD_TYPES) and set_ != "arcana":
                print(f"warning: no cost for {name}", file=sys.stderr)
        if name in group_of and group_of[name] != name:
            card["pile"] = group_of[name]
        plus, coins, vp = vanilla(o["text"])
        if plus:
            card["plus"] = plus
        if coins:
            card["coins"] = coins
        if vp:
            card["vp"] = vp
        card["text"] = plain_text(o["text"])
        cards.append(card)

    for t in tabs:
        tags = t["cardset_tags"]
        if "dominion1stEditionRemoved" in tags or "intrigue1stEditionRemoved" in tags:
            name = t["card_tag"]
            if name == "Harem":
                continue
            card = {"name": name, "set": REMOVED_TAGS[tags[-1]] if tags[-1] in REMOVED_TAGS else
                    ("base" if "dominion1stEdition" in tags else "intrigue"),
                    "types": t["types"], "removed": True, "cost": parse_cost(t),
                    "text": tabs_text(en[name]["description"])}
            plus = {}
            for line in card["text"].split("\n"):
                matched = False
                for pat, key in VANILLA:
                    pat = pat.replace(r"\|?\[(\d+)\]", r"\$(\d+)")
                    if m := re.fullmatch(pat, line.strip()):
                        plus[key] = plus.get(key, 0) + int(m.group(1))
                        matched = True
                if not matched:
                    break
            if plus:
                card["plus"] = plus
            cards.append(card)

    for c in cards:
        types = set(c["types"])
        not_supply = "(This is not in the Supply.)" in c["text"]
        c["kingdom"] = (
            not c.get("basic")
            and "pile" not in c
            and not (types & NON_CARD_TYPES)
            and not not_supply
            and not (types & {"Heirloom", "Shelter", "Ruins", "Prize", "Reward", "Loot",
                               "Spirit", "Zombie"})
            and c["name"] != "Ruins"
        )

    names = defaultdict(list)
    for c in cards:
        names[norm(c["name"])].append(c["name"])
    dups = {k: v for k, v in names.items() if len(v) > 1}
    if dups:
        sys.exit(f"duplicate names: {dups}")

    by_set = defaultdict(list)
    for c in cards:
        by_set[c["set"]].append(c)
    Path(out).mkdir(parents=True, exist_ok=True)
    for s, cs in by_set.items():
        cs.sort(key=lambda c: (not c.get("basic"), c.get("removed", False), c["name"]))
        with open(Path(out) / f"{s}.json", "w") as f:
            f.write("[\n")
            f.write(",\n".join("  " + json.dumps(c, ensure_ascii=False) for c in cs))
            f.write("\n]\n")
        kingdom = sum(c["kingdom"] for c in cs)
        print(f"{s}: {len(cs)} objects, {kingdom} kingdom")


if __name__ == "__main__":
    main()

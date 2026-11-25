// Steers a game to a scenario's position through the HTTP API, as its seats.
//
// Steps, each answered by whichever seat is deciding:
//   "play X"      play X from hand            "buy X"     buy X (a card, Event, ...)
//   "treasures"   play every Treasure in hand  "end"       end the current phase
//   "pick X, Y"   choose the options naming X and Y (a card or words of the label)
//   "none"        choose nothing               "default"   the least the decision allows
//   "turn A, B"   a whole turn: Actions, Treasures, then the first of A, B, ... it can
//                 buy, answering anything asked on the way with "default"
//   "turns N A, B"  N turns of "turn A, B"
//   "until P X+Y & Q Z: A, B"  turns of "turn A, B" until P (Alice or Bob) starts a
//                 turn holding X and Y, while Q holds Z
//   "turns" stops early when the game ends
export class Driver {
  constructor(arena, game, seats) {
    this.arena = arena;
    this.game = game;
    this.seats = seats; // name -> session cookie
  }

  async init() {
    const { json } = await this.arena.api('GET', '/cards');
    this.types = new Map(json.map((c) => [c.name, c.types]));
  }

  is(card, type) {
    return (this.types.get(card) ?? []).includes(type);
  }

  async deciding() {
    for (const [name, cookie] of Object.entries(this.seats)) {
      const { view } = await this.arena.view(this.game, cookie);
      if (view.decision) return { name, cookie, view, decision: view.decision };
    }
    return null;
  }

  async answer(d, indices) {
    await this.arena.answer(this.game, d.cookie, { decision: d.decision.id, indices });
  }

  async fallback(d) {
    const dec = d.decision;
    if (dec.kind === 'turn') {
      const end = dec.options.find((o) => o.action === 'end-phase');
      if (end) return this.answer(d, [end.index]);
    }
    if (dec.nameAny) {
      return this.arena.answer(this.game, d.cookie, { decision: dec.id, name: 'Copper' });
    }
    const n = Math.max(0, dec.min ?? 0);
    return this.answer(d, dec.options.slice(0, n).map((o) => o.index));
  }

  async run(steps) {
    for (const step of steps) await this.step(step);
  }

  async step(step) {
    const [verb, ...rest] = step.split(' ');
    const arg = rest.join(' ');
    const list = arg ? arg.split(',').map((s) => s.trim()).filter(Boolean) : [];
    const d = await this.deciding();
    if (!d && verb === 'turns') return;
    if (!d) throw new Error(`"${step}": nobody is deciding`);
    const opts = d.decision.options ?? [];
    const find = (pred, what) => {
      const o = opts.find(pred);
      if (!o) throw new Error(`"${step}": ${d.name} has no option to ${what} (${opts.map((o) => o.label).join(' | ')})`);
      return o;
    };
    switch (verb) {
      case 'play':
        return this.answer(d, [find((o) => o.action === 'play' && o.card === arg, `play ${arg}`).index]);
      case 'buy':
        return this.answer(d, [find((o) => o.action === 'buy' && o.card === arg, `buy ${arg}`).index]);
      case 'end':
        return this.answer(d, [find((o) => o.action === 'end-phase', 'end the phase').index]);
      case 'treasures':
        return this.treasures(d.name);
      case 'pick': {
        const picked = list.map((want) =>
          find((o) => o.card === want || (o.label ?? '').toLowerCase().includes(want.toLowerCase()), `pick ${want}`).index,
        );
        return this.answer(d, picked);
      }
      case 'none':
        return this.answer(d, []);
      case 'default':
        return this.fallback(d);
      case 'turn':
        return this.turn(list);
      case 'turns': {
        const [n, ...first] = arg.split(' ');
        const buys = first.join(' ').split(',').map((s) => s.trim()).filter(Boolean);
        for (let i = 0; i < Number(n) && (await this.deciding()); i++) await this.turn(buys);
        return;
      }
      case 'until':
        return this.until(arg);
      default:
        throw new Error(`unknown step "${step}"`);
    }
  }

  async treasures(who) {
    for (;;) {
      const d = await this.deciding();
      if (!d || d.name !== who || d.decision.kind !== 'turn') return;
      const o = d.decision.options.find((o) => o.action === 'play' && this.is(o.card, 'Treasure'));
      if (!o) return;
      await this.answer(d, [o.index]);
    }
  }

  async holds(who, cards) {
    const { view } = await this.arena.view(this.game, this.seats[who]);
    const hand = { ...(view.you?.hand?.known ?? {}) };
    return cards.every((c) => (hand[c] = (hand[c] ?? 0) - 1) >= 0);
  }

  async until(arg) {
    const [cond, buyList = ''] = arg.split(':');
    const groups = cond.split('&').map((g) => {
      const [who, cards] = g.trim().split(/ (.*)/);
      return { who, cards: cards.split('+').map((s) => s.trim()) };
    });
    const buys = buyList.split(',').map((s) => s.trim()).filter(Boolean);
    for (let i = 0; i < 80; i++) {
      const d = await this.deciding();
      if (!d) throw new Error(`until ${arg}: nobody is deciding`);
      if (d.name === groups[0].who && d.decision.kind === 'turn') {
        let all = true;
        for (const g of groups) all &&= await this.holds(g.who, g.cards);
        if (all) return;
      }
      await this.turn(buys);
    }
    throw new Error(`until ${arg}: never happened`);
  }

  /// One turn of whoever is deciding now.
  async turn(buys) {
    const first = await this.deciding();
    if (!first) throw new Error('turn: nobody is deciding');
    const who = first.name;
    const turnNo = first.view.turn?.number;
    for (let guard = 0; guard < 200; guard++) {
      const d = await this.deciding();
      if (!d) return;
      if (d.name !== who && d.decision.kind === 'turn') return;
      if (d.name === who && d.decision.kind === 'turn' && d.view.turn?.number !== turnNo) return;
      if (d.decision.kind !== 'turn') {
        await this.fallback(d);
        continue;
      }
      const opts = d.decision.options;
      const action = opts.find((o) => o.action === 'play' && this.is(o.card, 'Action'));
      if (d.decision.phase === 'action' && action) {
        await this.answer(d, [action.index]);
        continue;
      }
      const treasure = opts.find((o) => o.action === 'play' && this.is(o.card, 'Treasure'));
      if (d.decision.phase === 'buy' && treasure) {
        await this.answer(d, [treasure.index]);
        continue;
      }
      const buy = d.decision.phase === 'buy' && buys.map((b) => opts.find((o) => o.action === 'buy' && o.card === b)).find(Boolean);
      if (buy) {
        await this.answer(d, [buy.index]);
        buys = [];
        continue;
      }
      await this.fallback(d);
    }
    throw new Error(`turn of ${who} did not end`);
  }
}

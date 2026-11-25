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
//   "spree N A, B"  N turns buying as many of A, B, ... as each can, in that order
//   "act"         play Actions, then Treasures, and stop before buying
//   "villages"    from now on, turns play the Actions giving the most +Actions first
//   "prefer X, Y" from now on, answer other questions with the options naming X or Y
//                 where there are any ("prefer" alone: the least again)
// A step may also be a function of the driver.
export class Driver {
  constructor(arena, game, seats) {
    this.arena = arena;
    this.game = game;
    this.seats = seats; // name -> session cookie
    this.prefer = [];
  }

  async init() {
    const { json } = await this.arena.api('GET', '/cards');
    this.types = new Map(json.map((c) => [c.name, c.types]));
    this.actions = new Map(json.map((c) => [c.name, Number(/\+(\d+) Actions?/.exec(c.text)?.[1] ?? 0)]));
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
    const opts = dec.options ?? [];
    const liked = this.prefer.flatMap((want) => opts.filter((o) => names(o, want)));
    if (liked.length) {
      const max = dec.max ?? opts.length;
      const picked = [...new Set(liked)].slice(0, max);
      for (const o of opts) if (picked.length < n && !picked.includes(o)) picked.push(o);
      return this.answer(d, picked.map((o) => o.index));
    }
    return this.answer(d, opts.slice(0, n).map((o) => o.index));
  }

  async run(steps) {
    for (const step of steps) await this.step(step);
  }

  async step(step) {
    if (typeof step === 'function') return step(this);
    const [verb, ...rest] = step.split(' ');
    const arg = rest.join(' ');
    const list = arg ? arg.split(',').map((s) => s.trim()).filter(Boolean) : [];
    const d = await this.deciding();
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
        return this.answer(d, [find((o) => buying(o) && o.card === arg, `buy ${arg}`).index]);
      case 'end':
        return this.answer(d, [find((o) => o.action === 'end-phase', 'end the phase').index]);
      case 'treasures':
        return this.treasures(d.name);
      case 'pick': {
        const picked = list.map((want) => find((o) => names(o, want), `pick ${want}`).index);
        return this.answer(d, picked);
      }
      case 'none':
        return this.answer(d, []);
      case 'default':
        return this.fallback(d);
      case 'turn':
        return this.turn(list);
      case 'turns':
      case 'spree': {
        const [n, ...first] = arg.split(' ');
        const buys = first.join(' ').split(',').map((s) => s.trim()).filter(Boolean);
        for (let i = 0; i < Number(n); i++) await this.turn(buys, verb === 'spree');
        return;
      }
      case 'act':
        return this.act();
      case 'villages':
        this.villagesFirst = true;
        return;
      case 'prefer':
        this.prefer = list;
        return;
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

  /// Plays the deciding seat's Actions, then its Treasures, and stops before it buys.
  async act() {
    return this.turn([], false, (d) => d.decision.phase === 'buy' && !d.decision.options.some((o) => o.action === 'play' && this.is(o.card, 'Treasure')));
  }

  /// Turns of "turn A, B" (or "spree") until `stop` holds before an answer of a turn, at
  /// most `limit` turns.
  async until(stop, buys, { spree = false, limit = 100 } = {}) {
    for (let i = 0; i < limit; i++) if (await this.turn(buys, spree, stop)) return;
    throw new Error(`no position met the condition in ${limit} turns`);
  }

  /// One turn of whoever is deciding now. Returns true if `stop` stopped it.
  async turn(buys, spree = false, stop = null) {
    const first = await this.deciding();
    if (!first) throw new Error('turn: nobody is deciding');
    const who = first.name;
    const turnNo = first.view.turn?.number;
    for (let guard = 0; guard < 200; guard++) {
      const d = await this.deciding();
      if (!d) return false;
      if (d.name !== who && d.decision.kind === 'turn') return false;
      if (d.name === who && d.decision.kind === 'turn' && d.view.turn?.number !== turnNo) return false;
      if (stop && d.decision.kind === 'turn' && stop(d)) return true;
      if (d.decision.kind !== 'turn') {
        await this.fallback(d);
        continue;
      }
      const opts = d.decision.options;
      const actions = opts.filter((o) => o.action === 'play' && this.is(o.card, 'Action'));
      const action = this.villagesFirst
        ? actions.reduce((best, o) => (best && this.actions.get(best.card) >= this.actions.get(o.card) ? best : o), null)
        : actions[0];
      if (d.decision.phase === 'action' && action) {
        await this.answer(d, [action.index]);
        continue;
      }
      const treasure = opts.find((o) => o.action === 'play' && this.is(o.card, 'Treasure'));
      if (d.decision.phase === 'buy' && treasure) {
        await this.answer(d, [treasure.index]);
        continue;
      }
      const buy = d.decision.phase === 'buy' && buys.map((b) => opts.find((o) => buying(o) && o.card === b)).find(Boolean);
      if (buy) {
        await this.answer(d, [buy.index]);
        if (!spree) buys = [];
        continue;
      }
      await this.fallback(d);
    }
    throw new Error(`turn of ${who} did not end`);
  }
}

function names(o, want) {
  return o.card === want || (o.label ?? '').toLowerCase().includes(want.toLowerCase());
}

function buying(o) {
  return o.action === 'buy' || o.action === 'buy-landscape';
}

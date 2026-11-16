use super::MAX_BATCH_BYTES;
use crate::{
    Error, Result,
    model::*,
    questions as q, transport as t,
    wire::{MAX_COLLECTION, MAX_SAFE_LONG, MAX_STRING_BYTES, Reader, Writer},
};

// Ordinal counts from the hash-pinned bundle, asserted by reverse.mjs.
const CARD_MODES: i32 = 221;
const GAME_BUTTONS: i32 = 8;
const DECLINE_BUTTONS: i32 = 71;
const QUESTION_IDS: i32 = 736;
const ABILITY_NAMES: i32 = 403;

fn invalid(message: &str) -> Error {
    Error::Protocol(message.into())
}
struct Output {
    writer: Writer,
    size: usize,
}
impl Output {
    fn new() -> Self {
        Self {
            writer: Writer::new(),
            size: 0,
        }
    }
    fn reserve(&mut self, n: usize) -> Result<()> {
        self.size = self
            .size
            .checked_add(n)
            .filter(|n| *n <= MAX_BATCH_BYTES)
            .ok_or_else(|| invalid("server batch exceeds size limit"))?;
        Ok(())
    }
    fn count(&mut self, n: usize) -> Result<()> {
        if n > MAX_COLLECTION {
            return Err(invalid("oversized wire collection"));
        }
        (n as i32).write(self)
    }
    fn ordinal(&mut self, value: i32, bound: i32) -> Result<()> {
        if !(0..bound).contains(&value) {
            return Err(invalid("invalid enum ordinal"));
        }
        value.write(self)
    }
    fn fields<T: Write>(&mut self, values: impl ExactSizeIterator<Item = T>) -> Result<()> {
        self.count(values.len())?;
        for value in values {
            value.write(self)?;
        }
        Ok(())
    }
}
trait Write {
    fn write(&self, w: &mut Output) -> Result<()>;
}
impl<T: Write + ?Sized> Write for &T {
    fn write(&self, w: &mut Output) -> Result<()> {
        (*self).write(w)
    }
}
impl Write for i32 {
    fn write(&self, w: &mut Output) -> Result<()> {
        w.reserve(4)?;
        w.writer.int(*self);
        Ok(())
    }
}
impl Write for i64 {
    fn write(&self, w: &mut Output) -> Result<()> {
        if !(-MAX_SAFE_LONG..=MAX_SAFE_LONG).contains(self) {
            return Err(invalid("unsafe JavaScript long"));
        }
        w.reserve(8)?;
        w.writer.long(*self);
        Ok(())
    }
}
impl Write for f64 {
    fn write(&self, w: &mut Output) -> Result<()> {
        if !self.is_finite() {
            return Err(invalid("non-finite wire double"));
        }
        w.reserve(8)?;
        w.writer.double(*self);
        Ok(())
    }
}
impl Write for bool {
    fn write(&self, w: &mut Output) -> Result<()> {
        w.reserve(1)?;
        w.writer.boolean(*self);
        Ok(())
    }
}
impl Write for String {
    fn write(&self, w: &mut Output) -> Result<()> {
        if self.len() > MAX_STRING_BYTES {
            return Err(invalid("oversized wire string"));
        }
        w.reserve(4 + self.len())?;
        w.writer.string(self);
        Ok(())
    }
}
impl<T: Write> Write for Vec<T> {
    fn write(&self, w: &mut Output) -> Result<()> {
        w.fields(self.iter())
    }
}
impl<A: Write, B: Write> Write for (A, B) {
    fn write(&self, w: &mut Output) -> Result<()> {
        self.0.write(w)?;
        self.1.write(w)
    }
}
impl Write for Option<i32> {
    fn write(&self, w: &mut Output) -> Result<()> {
        if self.is_some_and(|value| value < 0) {
            return Err(invalid("negative optional enum ordinal"));
        }
        self.unwrap_or(-1).write(w)
    }
}
macro_rules! structure {
    ($name:ty; $($field:ident),* $(,)?) => { impl Write for $name {
        fn write(&self,w:&mut Output)->Result<()> { $(self.$field.write(w)?;)* Ok(()) }
    } };
}
structure!(CardFrequency; card_name, frequency);
structure!(Reminder; card_name, blinking, frequency);
structure!(PileMarker; kind, zone_index);
structure!(Player; player_id, card_frequencies, reminders, bought_projects);
structure!(TurnDescription; owner_id, turn_number, turn_type, controller_id);
structure!(Cost; coins, potions, debt);
structure!(CostReduction; generic, by_type, by_card, cost_override);
structure!(CardMove; from_zone, to_zone, before, after, movement_type, animation_class);
structure!(CounterChange; index, new_value);
structure!(PileUpdate; index, top_card_id);
structure!(Shuffle; owner, include_discard);
structure!(Inspection; player, cards, from_zone, to_zone, is_public, linked_to_question);
structure!(TokenChange; token, zone, is_flipped);
structure!(ZoneCreated; index, owner, kind, created_by);
structure!(ZoneDestroyed; index);
structure!(Rename; id, card_name);
structure!(BehaviourChange; id, card_name);
structure!(PilesStatus; draw_index, draw_size, discard_index, discard_size, top_card_id);
structure!(RemindersChange; player, reminders);
structure!(ImageChange; id, card_name);
structure!(Association; card, linked_cards);
structure!(CardStatus; card, stays_out, played_card_lists, set_aside_card_lists, properties);
structure!(ProjectBought; player, project);
structure!(ChangeCardTypes; cards, added, removed);
structure!(PileReorder; pile, cards);
structure!(ProjectRemoved; player, project);
structure!(CardTypeChange; id, added);
structure!(TemporaryEffects; renames, behaviour_changes, image_changes, card_type_changes);
structure!(AutoplaySetting; card_name, value);
structure!(TimerSettings; settings, force_resign, preset);
structure!(TableOptions; players_see_spectator_chat, timer_settings);
structure!(Snapshot; reconnect, game_id, state, autoplay_settings, table_options, events);
structure!(Ticker; entry, is_start_game);
structure!(Score; total_points, used_turns, parts);
structure!(PlayerResult; player_id, rank, score, card_names, resign_index, resignation_type);
structure!(GameResult; table_id, game_id, rating_type, empty_piles, players, auto_continue);
impl Write for GameState {
    fn write(&self, w: &mut Output) -> Result<()> {
        if self.card_names.len() != self.pile_names.len()
            || self.card_names.len() != self.card_counts.len()
        {
            return Err(invalid("allocation parallel arrays differ"));
        }
        let mut offset = 0usize;
        for ((name, pile), count) in self
            .card_names
            .iter()
            .zip(&self.pile_names)
            .zip(&self.card_counts)
        {
            let n = usize::try_from(*count).map_err(|_| invalid("negative allocation count"))?;
            if n > MAX_COLLECTION
                || offset
                    .checked_add(n)
                    .is_none_or(|end| end > MAX_COLLECTION || end > self.cards.len())
            {
                return Err(invalid("oversized/missing card allocation"));
            }
            for (i, card) in self.cards[offset..offset + n].iter().enumerate() {
                if card.id != (offset + i) as i32
                    || card.card_name != *name
                    || card.pile_name != *pile
                {
                    return Err(invalid("cards disagree with allocation arrays"));
                }
            }
            offset += n;
        }
        if offset != self.cards.len() {
            return Err(invalid("extra allocated cards"));
        }
        self.players.write(w)?;
        self.card_names.write(w)?;
        self.pile_names.write(w)?;
        self.card_counts.write(w)?;
        w.fields(self.zones.iter().map(|z| z.index))?;
        w.fields(self.zones.iter().map(|z| z.kind))?;
        w.fields(self.zones.iter().map(|z| z.pile_name))?;
        w.fields(self.zones.iter().map(|z| z.owner))?;
        w.fields(self.zones.iter().map(|z| &z.cards))?;
        w.fields(self.zones.iter().map(|z| z.created_by))?;
        w.fields(self.zones.iter().map(|z| z.attached_trait))?;
        w.fields(self.zones.iter().map(|z| z.is_visible))?;
        self.pile_markers.write(w)?;
        w.fields(self.tokens.iter().map(|z| z.kind))?;
        w.fields(self.tokens.iter().map(|z| z.owner))?;
        w.fields(self.tokens.iter().map(|z| z.zone))?;
        w.fields(self.tokens.iter().map(|z| z.is_flipped))?;
        w.fields(self.counters.iter().map(|z| z.index))?;
        w.fields(self.counters.iter().map(|z| z.kind))?;
        w.fields(self.counters.iter().map(|z| z.owner))?;
        w.fields(self.counters.iter().map(|z| z.value))?;
        w.fields(self.counters.iter().map(|z| z.zone))?;
        w.fields(self.counters.iter().map(|z| z.card))?;
        w.fields(self.counters.iter().map(|z| z.is_visible))?;
        self.turn.write(w)?;
        self.cost_reduction.write(w)?;
        self.temporary_effects.write(w)?;
        self.associations.write(w)?;
        self.card_statuses.write(w)
    }
}
impl Write for Event {
    fn write(&self, w: &mut Output) -> Result<()> {
        macro_rules! event { ($($variant:ident=$tag:literal),*) => { match self {
            $(Self::$variant(v)=>{ $tag.write(w)?;v.write(w) }),*
        } } }
        event!(
            CardMove = 0,
            CounterChange = 1,
            PileUpdate = 2,
            TurnDescription = 3,
            Shuffle = 4,
            Inspection = 5,
            CostReduction = 6,
            TokenChange = 7,
            ZoneCreated = 8,
            ZoneDestroyed = 9,
            Rename = 10,
            BehaviourChange = 11,
            PilesStatus = 12,
            RemindersChange = 13,
            ImageChange = 14,
            Association = 15,
            CardStatus = 16,
            ProjectBought = 17,
            ChangeCardTypes = 18,
            PileReorder = 19,
            ProjectRemoved = 20
        )
    }
}
fn depth(level: usize) -> Result<()> {
    if level > 48 {
        Err(invalid("nested question/story limit exceeded"))
    } else {
        Ok(())
    }
}
fn arguments(values: &[q::LogArgument], w: &mut Output, level: usize) -> Result<()> {
    w.count(values.len())?;
    for v in values {
        argument(v, w, level)?;
    }
    Ok(())
}
fn ability(v: &q::Ability, w: &mut Output, question: bool, level: usize) -> Result<()> {
    depth(level)?;
    w.ordinal(v.description_type, 3)?;
    w.ordinal(
        v.name,
        if v.description_type == 0 {
            25
        } else {
            ABILITY_NAMES
        },
    )?;
    v.association.write(w)?;
    if v.description_type == 2 {
        arguments(&v.arguments, w, level + 1)?;
    } else if !v.arguments.is_empty() {
        return Err(invalid("non-contextual ability has arguments"));
    }
    if question {
        v.optional.write(w)?;
        v.copy_index.write(w)?;
        v.copy_total.write(w)?;
    }
    Ok(())
}
fn argument(v: &q::LogArgument, w: &mut Output, level: usize) -> Result<()> {
    use q::LogArgument::*;
    depth(level)?;
    match v {
        CardNames(v) => {
            0.write(w)?;
            w.count(v.len())?;
            for c in v {
                w.ordinal(c.name, 881)?;
                c.frequency.write(w)?;
            }
        }
        Player(v) => {
            1.write(w)?;
            v.write(w)?;
        }
        Zone(v) => {
            2.write(w)?;
            v.write(w)?;
        }
        OwnZone { player, zone } => {
            3.write(w)?;
            player.write(w)?;
            zone.write(w)?;
        }
        Number(v) => {
            4.write(w)?;
            v.write(w)?;
        }
        Counter(v) => {
            5.write(w)?;
            v.write(w)?;
        }
        Turn {
            owner,
            number,
            kind,
            controller,
        } => {
            6.write(w)?;
            owner.write(w)?;
            number.write(w)?;
            w.ordinal(*kind, 10)?;
            controller.write(w)?;
        }
        Amount {
            coins,
            potions,
            debt,
        } => {
            7.write(w)?;
            coins.write(w)?;
            potions.write(w)?;
            debt.write(w)?;
        }
        Ability(v) => {
            8.write(w)?;
            ability(v, w, false, level + 1)?;
        }
        Event {
            kind,
            ability: a,
            arguments: args,
        } => {
            9.write(w)?;
            w.ordinal(*kind, 3)?;
            ability(a, w, false, level + 1)?;
            arguments(args, w, level + 1)?;
        }
        Card(v) => {
            10.write(w)?;
            v.write(w)?;
        }
        SingularCard(v) => {
            11.write(w)?;
            v.write(w)?;
        }
        Token(v) => {
            12.write(w)?;
            v.write(w)?;
        }
        DirectionalZone { direction, zone } => {
            13.write(w)?;
            w.ordinal(*direction, 2)?;
            zone.write(w)?;
        }
        Players(v) => {
            14.write(w)?;
            v.write(w)?;
        }
        MetaGame(v) => {
            15.write(w)?;
            v.game_id.write(w)?;
            v.rated.write(w)?;
            v.card_pool_level.write(w)?;
            v.levels.write(w)?;
            v.likelihoods.write(w)?;
            w.ordinal(v.timer_preset, 8)?;
        }
    }
    Ok(())
}
impl Write for q::LogArgument {
    fn write(&self, w: &mut Output) -> Result<()> {
        argument(self, w, 0)
    }
}
impl Write for q::Story {
    fn write(&self, w: &mut Output) -> Result<()> {
        w.ordinal(self.question_id, QUESTION_IDS)?;
        arguments(&self.arguments, w, 0)?;
        self.context.is_some().write(w)?;
        if let Some(v) = &self.context {
            argument(v, w, 0)?;
        }
        Ok(())
    }
}
structure!(q::QuestionDescription; question_type,association,story,short_story);
fn decline_button(value: i32, w: &mut Output) -> Result<()> {
    if value == -1 {
        value.write(w)
    } else {
        w.ordinal(value, DECLINE_BUTTONS)
    }
}
fn question(v: &q::Question, w: &mut Output, level: usize) -> Result<()> {
    use q::QuestionKind::*;
    depth(level)?;
    let tag = match v.kind {
        Choice { .. } => 0,
        Number { .. } => 1,
        Complex(_) => 2,
        Delayed { .. } => 3,
        Name => 4,
    };
    tag.write(w)?;
    v.description.write(w)?;
    match &v.kind {
        Choice {
            min,
            max,
            elements,
            decline,
            defaults,
            affected,
        } => {
            if *min < 0 || max < min {
                return Err(invalid("invalid choice bounds"));
            }
            min.write(w)?;
            max.write(w)?;
            element(elements, w, level)?;
            decline_button(*decline, w)?;
            defaults.write(w)?;
            affected.write(w)?;
        }
        Number {
            min,
            max,
            default,
            decline,
        } => {
            min.write(w)?;
            max.write(w)?;
            default.write(w)?;
            decline_button(*decline, w)?;
        }
        Complex(children) => {
            w.count(children.len())?;
            for child in children {
                question(child, w, level + 1)?;
            }
        }
        Delayed { decline } => decline_button(*decline, w)?,
        Name => {}
    }
    Ok(())
}
fn element(v: &q::QuestionElements, w: &mut Output, level: usize) -> Result<()> {
    use q::QuestionElements::*;
    w.count(v.len())?;
    if v.is_empty() {
        return Ok(());
    }
    match v {
        Cards(v) => {
            0.write(w)?;
            for x in v {
                x.write(w)?;
            }
        }
        Abilities(v) => {
            1.write(w)?;
            for x in v {
                ability(x, w, true, level + 1)?;
            }
        }
        ExtraTurns(v) => {
            2.write(w)?;
            for x in v {
                x.owner.write(w)?;
                x.number.write(w)?;
                w.ordinal(x.kind, 10)?;
                x.controller.write(w)?;
            }
        }
        Zones(v) => {
            3.write(w)?;
            for x in v {
                x.write(w)?;
            }
        }
        Modes(v) => {
            4.write(w)?;
            for x in v {
                w.ordinal(*x, CARD_MODES)?;
            }
        }
        Names(v) => {
            5.write(w)?;
            for x in v {
                w.ordinal(*x, 881)?;
            }
        }
        Buttons(v) => {
            6.write(w)?;
            for x in v {
                w.ordinal(*x, GAME_BUTTONS)?;
            }
        }
        Cleanup(v) => {
            7.write(w)?;
            for x in v {
                w.count(x.names.len())?;
                for name in &x.names {
                    w.ordinal(*name, 881)?;
                }
                x.is_optional.write(w)?;
                x.allows_reordering.write(w)?;
                x.is_enabled.write(w)?;
            }
        }
        BuyableCards(v) => {
            8.write(w)?;
            for x in v {
                x.write(w)?;
            }
        }
        ExplainedCards(v) => {
            9.write(w)?;
            for (id, value, kind) in v {
                id.write(w)?;
                value.write(w)?;
                if !(-1..=0).contains(kind) {
                    return Err(invalid("unknown card explanation"));
                }
                kind.write(w)?;
            }
        }
    }
    Ok(())
}
impl Write for q::Question {
    fn write(&self, w: &mut Output) -> Result<()> {
        self.index.write(w)?;
        question(self, w, 0)
    }
}
impl Write for LogEntry {
    fn write(&self, w: &mut Output) -> Result<()> {
        match self {
            Self::Named {
                name,
                depth,
                arguments,
            } => {
                0.write(w)?;
                name.write(w)?;
                depth.write(w)?;
                arguments.write(w)
            }
            Self::Decision {
                decision_index,
                player,
                decision,
                autoplayed,
            } => {
                1.write(w)?;
                decision_index.write(w)?;
                player.write(w)?;
                decision.write(w)?;
                autoplayed.write(w)
            }
        }
    }
}
impl Write for ScorePart {
    fn write(&self, w: &mut Output) -> Result<()> {
        self.card_name.write(w)?;
        self.points.write(w)?;
        self.frequency.write(w)?;
        self.reserved.write(w)?;
        if let LogEntry::Named {
            name,
            depth,
            arguments,
        } = &self.explanation
        {
            name.write(w)?;
            depth.write(w)?;
            arguments.write(w)
        } else {
            Err(invalid("score explanation must be named"))
        }
    }
}
structure!(t::NamedId; id,name);
structure!(t::TimerSettings; parameters,force_resign,preset);
impl Write for t::Group {
    fn write(&self, w: &mut Output) -> Result<()> {
        match self {
            Self::Nobody => 0.write(w),
            Self::Everybody => 1.write(w),
            Self::FriendsOf(p) => {
                2.write(w)?;
                p.write(w)
            }
            Self::Players(ps) => {
                3.write(w)?;
                ps.write(w)
            }
        }
    }
}
impl Write for t::RequiredSlot {
    fn write(&self, w: &mut Output) -> Result<()> {
        match self {
            Self::Card(v) => {
                0.write(w)?;
                w.ordinal(*v, 881)
            }
            Self::Expansion(v) => {
                1.write(w)?;
                w.ordinal(*v, 20)
            }
            Self::LandscapeTypes(v) => {
                2.write(w)?;
                v.write(w)
            }
        }
    }
}
impl Write for t::SpecialKingdom {
    fn write(&self, w: &mut Output) -> Result<()> {
        w.ordinal(self.colonies, 3)?;
        w.ordinal(self.shelters, 3)?;
        w.count(self.setup.len())?;
        for (id, values) in &self.setup {
            w.ordinal(*id, 13)?;
            if [1, 2, 4, 5, 10].contains(id) {
                values.write(w)?;
            } else {
                if values.len() != 1 {
                    return Err(invalid("setup rule requires one name"));
                }
                values[0].write(w)?;
            }
        }
        self.trait_targets.write(w)
    }
}
impl Write for t::TableRule {
    fn write(&self, w: &mut Output) -> Result<()> {
        use t::RuleValue::*;
        self.id.write(w)?;
        match (&self.value, self.id) {
            (Boolean(v), 0 | 13 | 19 | 20 | 23) => v.write(w),
            (Ints(v), 1 | 3 | 12 | 21) => v.write(w),
            (Required(v), 2) => v.write(w),
            (Group(v), 4 | 5 | 17) => v.write(w),
            (Double(v), 6 | 7) => v.write(w),
            (Seed { enabled, seed }, 8) => {
                enabled.write(w)?;
                seed.write(w)
            }
            (Int(v), 9 | 10 | 15 | 22) => v.write(w),
            (Special(v), 11) => v.write(w),
            (PlayerOrder(v), 14) => v.write(w),
            (Timer(v), 16) => v.write(w),
            (
                Replay {
                    game_id,
                    decision,
                    players,
                },
                18,
            ) => {
                game_id.write(w)?;
                decision.write(w)?;
                players.write(w)
            }
            (Text(v), 24) => v.write(w),
            _ => Err(invalid("table rule id/value mismatch")),
        }
    }
}
impl Write for t::TableDetails {
    fn write(&self, w: &mut Output) -> Result<()> {
        if self.ready.len() != self.players.len() {
            return Err(invalid("table player/ready arrays differ"));
        }
        self.table_id.write(w)?;
        self.host_id.write(w)?;
        self.players.write(w)?;
        self.ready.write(w)?;
        self.bots.write(w)?;
        self.spectators.write(w)?;
        self.reconnecting.write(w)?;
        self.rules.write(w)?;
        self.division.write(w)?;
        self.match_score.is_some().write(w)?;
        if let Some(score) = &self.match_score {
            self.match_games.write(w)?;
            score.host_wins.write(w)?;
            score.opponent_wins.write(w)?;
            score.host_name.write(w)?;
            score.opponent_name.write(w)?;
            score.winner.is_some().write(w)?;
            if let Some(v) = score.winner {
                v.write(w)?;
            }
            score.completed.write(w)?;
            score.host_id.write(w)?;
            score.opponent_id.is_some().write(w)?;
            if let Some(v) = score.opponent_id {
                v.write(w)?;
            }
        } else if !self.match_games.is_empty() {
            return Err(invalid("match games without score"));
        }
        self.tournament.write(w)
    }
}

impl Write for t::Relationships {
    fn write(&self, w: &mut Output) -> Result<()> {
        self.following.write(w)?;
        self.friends.write(w)?;
        self.followers.write(w)?;
        self.blacklisted.write(w)
    }
}
impl Write for t::Preference {
    fn write(&self, w: &mut Output) -> Result<()> {
        use t::PreferenceValue::*;
        w.ordinal(self.id, 52)?;
        match &self.value {
            Text(v) => {
                0.write(w)?;
                v.write(w)
            }
            Int(v) => {
                1.write(w)?;
                v.write(w)
            }
            Ownership { expansions, levels } => {
                2.write(w)?;
                expansions.write(w)?;
                levels.write(w)
            }
            Cards { list_id, cards } => {
                3.write(w)?;
                list_id.write(w)?;
                cards.write(w)
            }
            Boolean(v) => {
                4.write(w)?;
                v.write(w)
            }
            Matching {
                bot,
                human,
                max_rating,
                min_rating,
                preferences,
                timers,
            } => {
                5.write(w)?;
                bot.write(w)?;
                human.write(w)?;
                max_rating.write(w)?;
                min_rating.write(w)?;
                preferences.write(w)?;
                timers.write(w)
            }
            Autoplay(v) => {
                6.write(w)?;
                v.write(w)
            }
            Ints(v) => {
                7.write(w)?;
                v.write(w)
            }
            Deleted => 8.write(w),
        }
    }
}
structure!(t::TimerUpdate; player,time,running,free_time,maximum);
structure!(t::ancillary::AvatarBadge; player_id,avatar,country);
impl Write for t::LoginSuccess {
    fn write(&self, w: &mut Output) -> Result<()> {
        self.player_id.write(w)?;
        self.username.write(w)?;
        self.reconnecting.write(w)?;
        self.previous_reconnects.write(w)?;
        self.preferences.write(w)?;
        self.session_id().to_owned().write(w)?;
        self.unread_mail.write(w)?;
        self.relationships.write(w)?;
        self.card_levels.write(w)?;
        self.timer_presets.write(w)?;
        self.feature_words.write(w)
    }
}
impl Write for t::TableSummary {
    fn write(&self, w: &mut Output) -> Result<()> {
        self.table_id.write(w)?;
        self.host.write(w)?;
        self.players.write(w)?;
        self.bots.write(w)?;
        self.spectators.write(w)?;
        self.min_players.write(w)?;
        self.max_players.write(w)?;
        self.observable.write(w)?;
        self.joinable.write(w)?;
        w.ordinal(self.status, 5)?;
        match (self.status, self.start_time) {
            (2, Some(v)) => v.write(w),
            (2, None) => Err(invalid("playing summary needs start time")),
            (_, None) => Ok(()),
            _ => Err(invalid("start time outside playing summary")),
        }
    }
}
fn message(v: &t::Message, w: &mut Output) -> Result<()> {
    use t::Message::*;
    match v {
        Chat {
            receiver,
            sender,
            text,
        } => {
            0.write(w)?;
            receiver.write(w)?;
            sender.write(w)?;
            text.write(w)
        }
        Failed(v) => {
            1.write(w)?;
            let id = t::FAILURES
                .iter()
                .position(|x| *x == v.reason)
                .ok_or_else(|| invalid("unknown failure reason"))?;
            (id as i32).write(w)?;
            v.card.unwrap_or(-1).write(w)
        }
        Login(v) => {
            2.write(w)?;
            v.write(w)
        }
        Tables(v) => {
            3.write(w)?;
            v.write(w)
        }
        RuleChanged { table_id, rule } => {
            4.write(w)?;
            table_id.write(w)?;
            Write::write(rule, w)
        }
        HostChanged { table_id, host } => {
            5.write(w)?;
            table_id.write(w)?;
            host.write(w)
        }
        PlayerJoined {
            table_id,
            player,
            is_player,
        } => {
            6.write(w)?;
            table_id.write(w)?;
            player.write(w)?;
            is_player.write(w)
        }
        PlayerLeft {
            table_id,
            player_id,
        } => {
            7.write(w)?;
            table_id.write(w)?;
            player_id.write(w)
        }
        PlayerStatus {
            player_id,
            table_id,
            status,
        } => {
            8.write(w)?;
            player_id.write(w)?;
            table_id.write(w)?;
            w.ordinal(*status, 6)
        }
        GameStarted { table_id, game_id } => {
            9.write(w)?;
            table_id.write(w)?;
            game_id.write(w)
        }
        TableJoined { table, show } => {
            10.write(w)?;
            table.write(w)?;
            show.write(w)
        }
        TableRemoved => 11.write(w),
        RoleChanged {
            table_id,
            player_id,
            is_player,
            ready,
        } => {
            12.write(w)?;
            table_id.write(w)?;
            player_id.write(w)?;
            is_player.write(w)?;
            ready.write(w)
        }
        TableDetails(v) => {
            13.write(w)?;
            v.write(w)
        }
        GameFinished {
            table,
            result,
            continue_allowed,
            match_completed,
        } => {
            14.write(w)?;
            table.write(w)?;
            result.write(w)?;
            continue_allowed.write(w)?;
            match_completed.write(w)
        }
        Succeeded(v) => {
            15.write(w)?;
            w.ordinal(*v, 11)
        }
        Preferences(v) => {
            16.write(w)?;
            v.write(w)
        }
        Relationships(v) => {
            19.write(w)?;
            v.write(w)
        }
        BotAdded { table_id, bot } => {
            22.write(w)?;
            table_id.write(w)?;
            bot.write(w)
        }
        TableTerminated(v) => {
            26.write(w)?;
            v.write(w)
        }
        PlayerReplaced { table_id, old, new } => {
            27.write(w)?;
            table_id.write(w)?;
            old.write(w)?;
            new.write(w)
        }
        PlayerDisconnected { table_id, player } => {
            28.write(w)?;
            table_id.write(w)?;
            player.write(w)
        }
        PlayerReconnected { table_id, player } => {
            29.write(w)?;
            table_id.write(w)?;
            player.write(w)
        }
        Event(v) => {
            32.write(w)?;
            v.write(w)
        }
        Pong => 36.write(w),
        Question(v) => {
            37.write(w)?;
            v.write(w)
        }
        Snapshot(v) => {
            38.write(w)?;
            v.write(w)
        }
        ReconnectInstruction(v) => {
            39.write(w)?;
            v.write(w)
        }
        TimeoutInstruction => 40.write(w),
        Timers(v) => {
            41.write(w)?;
            v.write(w)
        }
        FormResult {
            form,
            succeeded,
            error,
        } => {
            44.write(w)?;
            form.write(w)?;
            succeeded.write(w)?;
            error.is_some().write(w)?;
            if let Some(error) = error {
                error.write(w)?;
            }
            Ok(())
        }
        AnimationCompensation => 45.write(w),
        Ancillary(t::ancillary::Message::PlayerAvatars(v)) => {
            56.write(w)?;
            v.write(w)
        }
        Ancillary(t::ancillary::Message::MailSummaries(v)) if v.is_empty() => {
            23.write(w)?;
            0.write(w)
        }
        Ancillary(t::ancillary::Message::Mails(v)) if v.is_empty() => {
            24.write(w)?;
            0.write(w)
        }
        Ancillary(t::ancillary::Message::Leaderboard(v)) if v.is_empty() => {
            25.write(w)?;
            0.write(w)
        }
        Ancillary(t::ancillary::Message::StatsViews(v)) if v.is_empty() => {
            59.write(w)?;
            0.write(w)
        }
        _ => Err(Error::Unsupported("server message has no encoder".into())),
    }
}
pub(super) fn batch(mut counter: i32, messages: &[t::Message]) -> Result<(Vec<u8>, i32)> {
    if messages.len() > MAX_COLLECTION {
        return Err(invalid("too many messages"));
    }
    let mut out = Output::new();
    for v in messages {
        let next = counter
            .checked_add(1)
            .ok_or_else(|| invalid("message counter overflow"))?;
        counter.write(&mut out)?;
        message(v, &mut out)?;
        counter = next;
    }
    let bytes = out.writer.finish();
    let mut r = Reader::new(&bytes);
    for _ in messages {
        r.int()?;
        t::parse_message(&mut r)?;
    }
    r.finish()?;
    Ok((bytes, counter))
}

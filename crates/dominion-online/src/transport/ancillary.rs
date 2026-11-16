//! Ancillary payloads of the pinned official protocol.
use super::{NamedId, array, checked};
use crate::{Result, wire::Reader};
use serde::{Deserialize, Serialize};
fn strings(r: &mut Reader<'_>) -> Result<Vec<String>> {
    array(r, |r| r.string())
}
macro_rules! payload {
    ($name:ident { $($field:ident: $ty:ty = $read:expr),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        pub struct $name { $(pub $field: $ty),* }
        impl $name { pub fn parse(r: &mut Reader<'_>) -> Result<Self> {
            Ok(Self { $($field: ($read)(r)?),* })
        } }
    };
}

payload!(Price {
    price_category: i32 = |r| checked(r, 5, "PriceCategories"),
    amount_in_dollar_cents: i32 = Reader::int
});

payload!(MailSummary {
    message_id: i64 = Reader::long,
    status: i32 = |r| checked(r, 3, "MailStati"),
    subject: String = Reader::string,
    date: i64 = Reader::long,
    permanent: bool = Reader::boolean
});

payload!(Mail {
    message_id: i64 = Reader::long,
    receiver: String = Reader::string,
    sender: String = Reader::string,
    subject: String = Reader::string,
    body: String = Reader::string,
    send_date: i64 = Reader::long,
    status: i32 = |r| checked(r, 3, "MailStati"),
    permanent: bool = Reader::boolean
});

payload!(HeadToHead {
    found: bool = Reader::boolean,
    opponent: NamedId = NamedId::parse,
    wins: i32 = Reader::int,
    losses: i32 = Reader::int,
    ties: i32 = Reader::int,
    expected_score: f64 = Reader::double
});

payload!(PlayerProfile {
    player: NamedId = NamedId::parse,
    editable: bool = Reader::boolean,
    discord: String = Reader::string,
    country: String = Reader::string,
    rating_type: i32 = |r| checked(r, 4, "RatingTypes"),
    rating_history: Vec<PlayerRatingPoint> = |r| array(r, PlayerRatingPoint::parse),
    summary: PlayerSummary = PlayerSummary::parse,
    period: PeriodSummary = PeriodSummary::parse,
    avatar: String = Reader::string,
    title: String = Reader::string
});

payload!(CardStats {
    version: String = Reader::string,
    min_level: f64 = Reader::double,
    player_name: String = Reader::string,
    levels: Vec<StatsLevel> = |r| array(r, StatsLevel::parse),
    event_type_keys: Vec<String> = strings,
    variants: Vec<CardStatFilterVariant> = |r| array(r, CardStatFilterVariant::parse),
    view_enabled: bool = Reader::boolean,
    recent_from: String = Reader::string,
    recent_through: String = Reader::string
});

payload!(CardPerTurn {
    min_level: f64 = Reader::double,
    player_name: String = Reader::string,
    card_id: i32 = Reader::int,
    variants: Vec<CardPerTurnVariant> = |r| array(r, CardPerTurnVariant::parse),
    gains_faster_than_board: Vec<CardCooccurrence> = |r| array(r, CardCooccurrence::parse),
    gains_slower_than_board: Vec<CardCooccurrence> = |r| array(r, CardCooccurrence::parse),
    all_players_substitute: bool = Reader::boolean
});

payload!(Scorigami {
    since_day: String = Reader::string,
    through_day: String = Reader::string,
    total_games: i64 = Reader::long,
    cells: Vec<ScorigamiCell> = |r| array(r, ScorigamiCell::parse)
});

payload!(StatsProfiles {
    player_name: String = Reader::string,
    since_day: String = Reader::string,
    through_day: String = Reader::string,
    type_keys: Vec<String> = strings,
    playstyle_keys: Vec<String> = strings,
    profiles: Vec<StatsProfile> = |r| array(r, StatsProfile::parse),
    card_stats: Vec<StatsCardStatVariant> = |r| array(r, StatsCardStatVariant::parse),
    player_since_day: String = Reader::string,
    player_through_day: String = Reader::string
});

payload!(StatsComparison {
    subject_player: String = Reader::string,
    subject_level: f64 = Reader::double,
    reference_player: String = Reader::string,
    reference_level: f64 = Reader::double,
    variants: Vec<StatsComparisonVariant> = |r| array(r, StatsComparisonVariant::parse)
});

payload!(StatsView {
    name: String = Reader::string,
    config: String = Reader::string
});

payload!(PlayerRatingPoint {
    date_millis: i64 = Reader::long,
    level: f64 = Reader::double,
    skill: f64 = Reader::double,
    deviation: f64 = Reader::double,
    games: i32 = Reader::int
});

payload!(ResultOpponent {
    player: NamedId = NamedId::parse,
    rank: i32 = Reader::int,
    score: i32 = Reader::int,
    resigned: bool = Reader::boolean
});

payload!(BonusCodeValidated {
    player_id: i32 = Reader::int,
    code: String = Reader::string,
    bought_expansion: i32 = |r| checked(r, 20, "Expansions"),
    expansion_list: Vec<i32> = Reader::ints
});

payload!(PurchaseResult {
    payment_status: i32 = Reader::int
});

payload!(PlayerSummary {
    wins: i32 = Reader::int,
    losses: i32 = Reader::int,
    ties: i32 = Reader::int,
    best_level: f64 = Reader::double,
    best_level_date_millis: i64 = Reader::long,
    best_rank: i32 = Reader::int,
    best_rank_date_millis: i64 = Reader::long,
    current_rank: i32 = Reader::int,
    current_level: f64 = Reader::double,
    best_win_streak: i32 = Reader::int,
    best_win_streak_start_millis: i64 = Reader::long,
    best_win_streak_end_millis: i64 = Reader::long,
    best_loss_streak: i32 = Reader::int,
    best_loss_streak_start_millis: i64 = Reader::long,
    best_loss_streak_end_millis: i64 = Reader::long,
    best_tie_streak: i32 = Reader::int,
    best_tie_streak_start_millis: i64 = Reader::long,
    best_tie_streak_end_millis: i64 = Reader::long,
    current_skill: f64 = Reader::double,
    current_deviation: f64 = Reader::double
});

payload!(PeriodSummary {
    available: bool = Reader::boolean,
    games: i32 = Reader::int,
    results: i32 = Reader::int,
    score: f64 = Reader::double,
    expected_score: f64 = Reader::double,
    distinct_opponents: i32 = Reader::int,
    opponent_skill_min: f64 = Reader::double,
    opponent_skill_max: f64 = Reader::double,
    start_level: f64 = Reader::double,
    start_skill: f64 = Reader::double,
    start_deviation: f64 = Reader::double,
    start_games: i32 = Reader::int,
    new_level: f64 = Reader::double,
    new_skill: f64 = Reader::double,
    new_deviation: f64 = Reader::double,
    new_games: i32 = Reader::int,
    period_start_millis: i64 = Reader::long,
    period_date_millis: i64 = Reader::long
});

payload!(StatsLevel {
    min_level: f64 = Reader::double
});

payload!(CardStatFilterVariant {
    filter_key: String = Reader::string,
    rows: Vec<CardStatRow> = |r| array(r, CardStatRow::parse)
});

payload!(CardPerTurnVariant {
    filter_key: String = Reader::string,
    total: Vec<i32> = Reader::ints,
    winner: Vec<i32> = Reader::ints,
    loser: Vec<i32> = Reader::ints,
    games_active: Vec<i32> = Reader::ints
});

payload!(CardCooccurrence {
    card_id: i32 = Reader::int,
    games: i32 = Reader::int,
    gains_with_co_card: f64 = Reader::double,
    card_multiplier: f64 = Reader::double,
    board_multiplier: f64 = Reader::double
});

payload!(ScorigamiCell {
    winner_score: i32 = Reader::int,
    loser_score: i32 = Reader::int,
    game_count: i32 = Reader::int,
    first_day_offset: i32 = Reader::int,
    last_day_offset: i32 = Reader::int,
    first_winner: String = Reader::string,
    first_loser: String = Reader::string,
    last_winner: String = Reader::string,
    last_loser: String = Reader::string
});

payload!(StatsProfile {
    min_level: f64 = Reader::double,
    player_name: String = Reader::string,
    reference_min_level: f64 = Reader::double,
    provinces_games: i64 = Reader::long,
    three_piles_games: i64 = Reader::long,
    resigned_games: i64 = Reader::long,
    variants: Vec<StatsProfileVariant> = |r| array(r, StatsProfileVariant::parse)
});

payload!(StatsCardStatVariant {
    min_level: f64 = Reader::double,
    filter_key: String = Reader::string,
    outcome: String = Reader::string,
    stats: Vec<StatsCardStatList> = |r| array(r, StatsCardStatList::parse)
});

payload!(StatsComparisonVariant {
    filter_key: String = Reader::string,
    outcome: String = Reader::string,
    favorites: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse),
    least_favorites: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse),
    openers: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse),
    least_openers: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse)
});

payload!(CardStatRow {
    card_id: i32 = Reader::int,
    games_in_kingdom: i32 = Reader::int,
    avg_turns: f64 = Reader::double,
    avg_vp: f64 = Reader::double,
    resign_pct: f64 = Reader::double,
    banned_pct: f64 = Reader::double,
    dislike_pct: f64 = Reader::double,
    fpa_pct: f64 = Reader::double,
    over_gained: f64 = Reader::double,
    skill_index: f64 = Reader::double,
    gain1_plus_pct: f64 = Reader::double,
    opened_pct: f64 = Reader::double,
    piled_pct: f64 = Reader::double,
    black_market_win_pct: f64 = Reader::double,
    gain_win_pct: f64 = Reader::double,
    totals: Vec<i32> = Reader::ints,
    winners: Vec<i32> = Reader::ints,
    losers: Vec<i32> = Reader::ints,
    winner_games: i32 = Reader::int,
    loser_games: i32 = Reader::int,
    gain1_plus_pct_winner: f64 = Reader::double,
    gain1_plus_pct_loser: f64 = Reader::double,
    opened_pct_winner: f64 = Reader::double,
    opened_pct_loser: f64 = Reader::double,
    opened_win_pct: f64 = Reader::double,
    purchased_win_pct: f64 = Reader::double,
    received_win_pct: f64 = Reader::double,
    skip_win_pct: f64 = Reader::double
});

payload!(StatsProfileVariant {
    filter_key: String = Reader::string,
    outcome: String = Reader::string,
    games: i64 = Reader::long,
    gains: Vec<f64> = |r| array(r, |r| r.double()),
    playstyle: Vec<f64> = |r| array(r, |r| r.double()),
    favorites: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse),
    least_favorites: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse),
    openers: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse),
    least_openers: Vec<StatsFavoriteCard> = |r| array(r, StatsFavoriteCard::parse),
    coins_per_turn: Vec<f64> = |r| array(r, |r| r.double()),
    actions_per_turn: Vec<f64> = |r| array(r, |r| r.double()),
    trashes_per_turn: Vec<f64> = |r| array(r, |r| r.double()),
    plays_per_turn: Vec<f64> = |r| array(r, |r| r.double()),
    games_active: Vec<i32> = Reader::ints,
    provinces_games: i64 = Reader::long,
    three_piles_games: i64 = Reader::long,
    resigned_games: i64 = Reader::long
});

payload!(StatsCardStatList {
    key: String = Reader::string,
    most: Vec<StatsCardStat> = |r| array(r, StatsCardStat::parse),
    least: Vec<StatsCardStat> = |r| array(r, StatsCardStat::parse)
});

payload!(StatsFavoriteCard {
    card_id: i32 = Reader::int,
    ratio: f64 = Reader::double,
    rate: f64 = Reader::double,
    reference_rate: f64 = Reader::double,
    games: i64 = Reader::long
});

payload!(StatsCardStat {
    card_id: i32 = Reader::int,
    value: f64 = Reader::double
});
payload!(Record {
    wins: i32 = Reader::int,
    losses: i32 = Reader::int,
    ties: i32 = Reader::int,
    best_win_streak: i32 = Reader::int,
    best_win_streak_start: i32 = Reader::int,
    best_win_streak_end: i32 = Reader::int,
    best_loss_streak: i32 = Reader::int,
    best_loss_streak_start: i32 = Reader::int,
    best_loss_streak_end: i32 = Reader::int,
    best_tie_streak: i32 = Reader::int,
    best_tie_streak_start: i32 = Reader::int,
    best_tie_streak_end: i32 = Reader::int
});
payload!(LeaderboardEntry {
    player: NamedId = NamedId::parse,
    rank: i32 = Reader::int,
    level: f64 = Reader::double,
    level_change: f64 = Reader::double,
    skill: f64 = Reader::double,
    deviation: f64 = Reader::double,
    volatility: f64 = Reader::double,
    converted_skill: f64 = Reader::double,
    converted_deviation: f64 = Reader::double,
    game_count: i32 = Reader::int,
    record: Record = Record::parse
});
payload!(ProfileResult {
 game_id:i64=Reader::long, time_millis:i64=Reader::long,
 opponents:Vec<ResultOpponent>=|r|array(r,ResultOpponent::parse),
 my_rank:i32=Reader::int,my_score:i32=Reader::int,rated:bool=Reader::boolean,
 my_level:f64=Reader::double,opponent_level:f64=Reader::double,expected_score:f64=Reader::double,
 kingdom:Vec<i32>=Reader::ints,players:Vec<NamedId>=|r|array(r,NamedId::parse),my_resigned:bool=Reader::boolean
});
payload!(TournamentTable {
    summary: super::TableSummary = super::TableSummary::parse,
    host: NamedId = NamedId::parse,
    opponent: NamedId = NamedId::parse,
    host_division: String = Reader::string,
    opponent_division: String = Reader::string,
    kind: i32 = |r| checked(r, 2, "TournamentTypes"),
    host_wins: f64 = Reader::double,
    opponent_wins: f64 = Reader::double
});
payload!(OpenMatch {
    match_id: i32 = Reader::int,
    created_at: i64 = Reader::long,
    status: i32 = Reader::int,
    opponent_name: String = Reader::string,
    games_played: i32 = Reader::int
});
fn optional_int(r: &mut Reader<'_>) -> Result<Option<i32>> {
    if r.boolean()? {
        Ok(Some(r.int()?))
    } else {
        Ok(None)
    }
}
payload!(TournamentGames {
 table_id:i64=Reader::long,game_ids:Vec<i64>=|r|array(r, |r| r.long()),
 host_wins:f64=Reader::double,opponent_wins:f64=Reader::double,
 host_name:String=Reader::string,opponent_name:String=Reader::string,
 match_winner_id:Option<i32>=optional_int,match_completed:bool=Reader::boolean,
 host_id:i32=Reader::int,opponent_id:Option<i32>=optional_int
});
payload!(AvatarBadge {
    player_id: i32 = Reader::int,
    avatar: String = Reader::string,
    country: String = Reader::string
});

#[derive(Clone, Debug)]
pub enum Message {
    PurchasePrice {
        package: i32,
        end_date: i64,
        purchase_date: i64,
        prices: Vec<Price>,
    },
    PaymentRedirect(String),
    AutomatchFound {
        table: super::TableDetails,
        is_bot_game: bool,
    },
    MailSummaries(Vec<MailSummary>),
    Mails(Vec<Mail>),
    Leaderboard(Vec<(i32, Vec<LeaderboardEntry>)>),
    BonusCodeValidated(BonusCodeValidated),
    PurchaseResult(PurchaseResult),
    ServerStateNotice,
    OpenMatches {
        table_id: i64,
        matches: Vec<OpenMatch>,
    },
    TournamentTables(Vec<TournamentTable>),
    TournamentGames(TournamentGames),
    MatchChat {
        table_id: i64,
        log: String,
    },
    CardStats(CardStats),
    CardPerTurn(CardPerTurn),
    CardStatsUpToDate {
        min_level: f64,
        player_name: String,
        levels: Vec<StatsLevel>,
        view_enabled: bool,
    },
    Scorigami(Scorigami),
    PlayerProfile(PlayerProfile),
    HeadToHead(HeadToHead),
    PlayerAvatars(Vec<AvatarBadge>),
    PlayerResults {
        rating_type: i32,
        results: Vec<ProfileResult>,
    },
    PlayerSearch {
        found: bool,
        player: NamedId,
    },
    StatsViews(Vec<StatsView>),
    StatsProfiles(StatsProfiles),
    StatsComparison(StatsComparison),
}
impl Message {
    pub fn parse(opcode: i32, r: &mut Reader<'_>) -> Result<Self> {
        Ok(match opcode {
            17 => Self::PurchasePrice {
                package: r.int()?,
                end_date: r.long()?,
                purchase_date: r.long()?,
                prices: array(r, Price::parse)?,
            },
            18 => Self::PaymentRedirect(r.string()?),
            21 => Self::AutomatchFound {
                table: super::TableDetails::parse(r)?,
                is_bot_game: r.boolean()?,
            },
            23 => Self::MailSummaries(array(r, MailSummary::parse)?),
            24 => Self::Mails(array(r, Mail::parse)?),
            25 => Self::Leaderboard(array(r, |r| {
                Ok((
                    checked(r, 4, "RatingTypes")?,
                    array(r, LeaderboardEntry::parse)?,
                ))
            })?),
            30 => Self::BonusCodeValidated(BonusCodeValidated::parse(r)?),
            31 => Self::PurchaseResult(PurchaseResult::parse(r)?),
            42 => Self::ServerStateNotice,
            46 => Self::OpenMatches {
                table_id: r.long()?,
                matches: array(r, OpenMatch::parse)?,
            },
            47 => Self::TournamentTables(array(r, TournamentTable::parse)?),
            48 => Self::TournamentGames(TournamentGames::parse(r)?),
            49 => Self::MatchChat {
                table_id: r.long()?,
                log: r.string()?,
            },
            50 => Self::CardStats(CardStats::parse(r)?),
            51 => Self::CardPerTurn(CardPerTurn::parse(r)?),
            52 => Self::CardStatsUpToDate {
                min_level: r.double()?,
                player_name: r.string()?,
                levels: array(r, StatsLevel::parse)?,
                view_enabled: r.boolean()?,
            },
            53 => Self::Scorigami(Scorigami::parse(r)?),
            54 => Self::PlayerProfile(PlayerProfile::parse(r)?),
            55 => Self::HeadToHead(HeadToHead::parse(r)?),
            56 => Self::PlayerAvatars(array(r, AvatarBadge::parse)?),
            57 => Self::PlayerResults {
                rating_type: checked(r, 4, "RatingTypes")?,
                results: array(r, ProfileResult::parse)?,
            },
            58 => Self::PlayerSearch {
                found: r.boolean()?,
                player: NamedId::parse(r)?,
            },
            59 => Self::StatsViews(array(r, StatsView::parse)?),
            60 => Self::StatsProfiles(StatsProfiles::parse(r)?),
            61 => Self::StatsComparison(StatsComparison::parse(r)?),
            _ => return Err(crate::Error::Unsupported(format!("server opcode {opcode}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        transport::{Decoder, Message as ServerMessage},
        wire::Writer,
    };
    #[derive(Deserialize)]
    struct Fixture {
        opcode: i32,
        hex: String,
    }
    #[test]
    fn upstream_generated_messages_consume_exactly_and_reject_every_truncated_prefix() {
        let fixtures: Vec<Fixture> =
            serde_json::from_str(include_str!("../../tests/fixtures/ancillary.json")).unwrap();
        for fixture in fixtures {
            let payload: Vec<u8> = fixture
                .hex
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            let mut r = Reader::new(&payload);
            let parsed = Message::parse(fixture.opcode, &mut r)
                .unwrap_or_else(|e| panic!("opcode {}: {e}", fixture.opcode));
            r.finish().unwrap();
            assert!(!format!("{parsed:?}").is_empty());
            for end in 0..payload.len() {
                assert!(
                    Message::parse(fixture.opcode, &mut Reader::new(&payload[..end])).is_err(),
                    "opcode {} prefix {end}",
                    fixture.opcode
                );
            }
            let mut header = Writer::new();
            header.int(0);
            header.int(fixture.opcode);
            let mut frame = header.finish();
            frame.extend(payload);
            let mut following = Writer::new();
            following.int(1);
            following.int(36);
            frame.extend(following.finish());
            let decoded = Decoder::new().decode(&frame).unwrap();
            assert!(matches!(
                decoded.as_slice(),
                [ServerMessage::Ancillary(_), ServerMessage::Pong]
            ));
        }
    }
}

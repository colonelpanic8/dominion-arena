//! What the game page says about who plays a game and how it runs, from a response's
//! `meta`: the account on each player's bar, the deciding seat's time, how the game
//! ended. Pure.

use dominion_protocol::ResultPlayer;
use dominion_protocol::lobby::{GameMeta, UserKind};

/// The account playing a seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatAccount {
    pub name: String,
    pub kind: UserKind,
    pub rating: Option<i32>,
    /// Someone is connected for the seat.
    pub connected: bool,
}

impl SeatAccount {
    /// What a bar says beside the name: "1512", "guest", "bot 1490".
    pub fn tag(&self) -> String {
        let kind = match self.kind {
            UserKind::Registered => None,
            UserKind::Guest => Some("guest".to_string()),
            UserKind::Bot => Some("bot".to_string()),
        };
        let rating = self.rating.map(|r| r.to_string());
        kind.into_iter().chain(rating).collect::<Vec<_>>().join(" ")
    }

    /// The same in a sentence, for a tooltip.
    pub fn title(&self) -> String {
        let kind = match self.kind {
            UserKind::Registered => "",
            UserKind::Guest => ", a guest",
            UserKind::Bot => ", a bot",
        };
        let rating = self
            .rating
            .map_or(String::new(), |r| format!(", rated {r}"));
        let here = if self.connected {
            "connected"
        } else {
            "not connected"
        };
        format!("{}{kind}{rating}: {here}", self.name)
    }
}

/// The account playing `seat`, if one does (a seat played by token has none).
pub fn account(meta: &GameMeta, seat: u8) -> Option<SeatAccount> {
    let i = usize::from(seat);
    let user = meta.users.get(i)?.as_ref()?;
    Some(SeatAccount {
        name: user.name.clone(),
        kind: user.kind,
        rating: user.rating,
        connected: meta.connected.get(i).copied().unwrap_or(true),
    })
}

/// A seat's own time is shown once this little is left.
pub const LOW_SECONDS: u64 = 60;

/// The deciding seat's time running out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forfeit {
    pub seat: u8,
    /// It is the seat shown.
    pub yours: bool,
    /// Nobody is connected for it.
    pub left: bool,
    pub seconds: u64,
}

/// What to say about the deciding seat's time at `now` (milliseconds since the epoch;
/// 0 when the clock can't be read): always once another player has left, else only when
/// little is left.
pub fn forfeit(
    meta: &GameMeta,
    deciding: Option<u8>,
    seat: Option<u8>,
    now: u64,
) -> Option<Forfeit> {
    let d = deciding?;
    let at = meta.forfeit_at?;
    if now == 0 {
        return None;
    }
    let yours = seat == Some(d);
    let left = !yours && !meta.connected.get(usize::from(d)).copied().unwrap_or(true);
    let seconds = at.saturating_sub(now).div_ceil(1000);
    (left || seconds <= LOW_SECONDS).then_some(Forfeit {
        seat: d,
        yours,
        left,
        seconds,
    })
}

/// Seconds as `m:ss`.
pub fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl Forfeit {
    /// The notice, calling the seat `name`.
    pub fn text(&self, name: &str) -> String {
        let time = clock(self.seconds);
        if self.yours {
            format!("You forfeit in {time}")
        } else if self.left {
            format!("{name} has left: forfeits in {time}")
        } else {
            format!("{name} forfeits in {time}")
        }
    }
}

/// Who ended the game early and how, a line each: `ended` is the meta's `resigned` or
/// `abandoned` (without it, a player out of the game resigned).
pub fn ended_lines(players: &[ResultPlayer], ended: Option<&str>) -> Vec<String> {
    let how = match ended {
        Some("abandoned") => "has forfeited: away, or too long over a decision",
        _ => "has resigned",
    };
    players
        .iter()
        .filter(|p| p.resigned)
        .map(|p| format!("{} {how}.", p.name))
        .collect()
}

/// Why the game ended, in a sentence: the view's reason, naming whoever ended it early.
pub fn end_reason(
    reason: Option<&str>,
    players: &[ResultPlayer],
    ended: Option<&str>,
) -> Option<String> {
    let out: Vec<&ResultPlayer> = players.iter().filter(|p| p.resigned).collect();
    match (reason, out.as_slice()) {
        (Some("resigned"), [p]) if ended == Some("abandoned") => {
            Some(format!("{} forfeited.", p.name))
        }
        (Some("resigned"), [p]) => Some(format!("{} resigned.", p.name)),
        (r, _) => r.map(super::wording::end_reason),
    }
}

/// The log's first lines, as dominion.games heads a game's log.
pub fn log_header(game_id: Option<&str>, rated: bool, timed: bool) -> [String; 2] {
    let rated = if rated { "rated" } else { "unrated" };
    let game = match game_id {
        Some(id) => format!("Game {id}, {rated}."),
        None => format!("Game, {rated}."),
    };
    let timer = if timed { "Timer: On" } else { "Timer: Off" };
    [game, timer.into()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_protocol::lobby::UserRef;

    fn user(name: &str, kind: UserKind, rating: Option<i32>) -> Option<UserRef> {
        Some(UserRef {
            name: name.into(),
            kind,
            rating,
            owner: None,
        })
    }

    fn meta() -> GameMeta {
        GameMeta {
            timed: true,
            users: vec![
                user("Ada", UserKind::Registered, Some(1512)),
                user("Robo", UserKind::Bot, Some(1490)),
                None,
            ],
            connected: vec![true, false, false],
            forfeit_at: Some(600_000),
            ..GameMeta::default()
        }
    }

    #[test]
    fn a_bar_names_the_kind_and_rating_of_an_account_and_nothing_for_a_token_seat() {
        let m = meta();
        assert_eq!(account(&m, 0).unwrap().tag(), "1512");
        let bot = account(&m, 1).unwrap();
        assert_eq!(bot.tag(), "bot 1490");
        assert!(!bot.connected);
        assert_eq!(bot.title(), "Robo, a bot, rated 1490: not connected");
        assert_eq!(account(&m, 2), None);
        assert_eq!(account(&m, 9), None);
        let guest = SeatAccount {
            name: "Guest 3".into(),
            kind: UserKind::Guest,
            rating: None,
            connected: true,
        };
        assert_eq!(guest.tag(), "guest");
    }

    #[test]
    fn a_player_who_left_is_counted_down_at_once_and_others_only_near_the_end() {
        let m = meta();
        let gone = forfeit(&m, Some(1), Some(0), 400_500).unwrap();
        assert!(gone.left && !gone.yours);
        assert_eq!(gone.seconds, 200);
        assert_eq!(gone.text("Robo"), "Robo has left: forfeits in 3:20");
        // Connected with time to spare: nothing; under a minute: the time.
        assert_eq!(forfeit(&m, Some(0), Some(1), 400_000), None);
        let low = forfeit(&m, Some(0), Some(1), 555_000).unwrap();
        assert_eq!(low.text("Ada"), "Ada forfeits in 0:45");
        // The seat shown is here, whatever the server last saw of its connection.
        assert_eq!(forfeit(&m, Some(1), Some(1), 400_000), None);
        let mine = forfeit(&m, Some(1), Some(1), 590_000).unwrap();
        assert_eq!(mine.text("Robo"), "You forfeit in 0:10");
        // Past the time it stays at zero until the server ends the game.
        assert_eq!(forfeit(&m, Some(1), None, 700_000).unwrap().seconds, 0);
        // No one deciding, an untimed game, or no clock: nothing.
        assert_eq!(forfeit(&m, None, Some(0), 400_000), None);
        assert_eq!(forfeit(&m, Some(1), Some(0), 0), None);
        let untimed = GameMeta {
            forfeit_at: None,
            ..m
        };
        assert_eq!(forfeit(&untimed, Some(1), Some(0), 400_000), None);
    }

    fn result(name: &str, rank: u32, resigned: bool) -> ResultPlayer {
        ResultPlayer {
            index: (rank - 1) as u8,
            name: name.into(),
            vp: 3,
            turns: 4,
            rank,
            resigned,
        }
    }

    #[test]
    fn an_early_end_names_who_ended_it() {
        let players = [result("Ada", 1, false), result("Bo", 2, true)];
        assert_eq!(
            ended_lines(&players, Some("resigned")),
            ["Bo has resigned."]
        );
        assert_eq!(
            ended_lines(&players, Some("abandoned")),
            ["Bo has forfeited: away, or too long over a decision."]
        );
        assert_eq!(
            end_reason(Some("resigned"), &players, Some("abandoned")).unwrap(),
            "Bo forfeited."
        );
        assert_eq!(
            end_reason(Some("resigned"), &players, None).unwrap(),
            "Bo resigned."
        );
        assert_eq!(
            end_reason(Some("provinces-gone"), &players[..1], None).unwrap(),
            "The Provinces ran out."
        );
        assert!(ended_lines(&players[..1], None).is_empty());
    }
}

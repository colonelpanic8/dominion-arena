//! Glicko-2 ratings, updated after every rated game: a game is one rating period for its
//! players, and a rating grows less certain by the day it isn't played.

use serde::{Deserialize, Serialize};

const SCALE: f64 = 173.7178;
/// How much volatility may change in one game.
const TAU: f64 = 0.5;
const MAX_DEVIATION: f64 = 350.0;
const DAY: f64 = 86_400.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    pub rating: f64,
    pub deviation: f64,
    pub volatility: f64,
    /// When it last changed, in seconds since the Unix epoch.
    pub at: u64,
}

impl Default for Rating {
    fn default() -> Rating {
        Rating {
            rating: 1500.0,
            deviation: MAX_DEVIATION,
            volatility: 0.06,
            at: 0,
        }
    }
}

fn g(phi: f64) -> f64 {
    1.0 / (1.0 + 3.0 * phi * phi / (std::f64::consts::PI * std::f64::consts::PI)).sqrt()
}

impl Rating {
    /// The deviation as it is at `now`: a day unplayed is a rating period without games.
    pub fn deviation_at(&self, now: u64) -> f64 {
        let days = if self.at == 0 {
            0.0
        } else {
            now.saturating_sub(self.at) as f64 / DAY
        };
        self.aged(days, self.volatility)
    }

    fn aged(&self, periods: f64, volatility: f64) -> f64 {
        let phi = self.deviation / SCALE;
        ((phi * phi + periods * volatility * volatility).sqrt() * SCALE).min(MAX_DEVIATION)
    }

    /// The rating after one game at `now` against `opponents`, each with the score
    /// against them: 1 for a win, 0.5 for a draw, 0 for a loss.
    pub fn after(&self, opponents: &[(Rating, f64)], now: u64) -> Rating {
        let days = if self.at == 0 {
            0.0
        } else {
            now.saturating_sub(self.at) as f64 / DAY
        };
        self.update(opponents, days.max(1.0), now)
    }

    fn update(&self, opponents: &[(Rating, f64)], periods: f64, now: u64) -> Rating {
        if opponents.is_empty() {
            return *self;
        }
        let mu = (self.rating - 1500.0) / SCALE;
        let phi = self.deviation / SCALE;
        let mut v_inv = 0.0;
        let mut sum = 0.0;
        for (o, score) in opponents {
            let (mu_j, phi_j) = ((o.rating - 1500.0) / SCALE, o.deviation_at(now) / SCALE);
            let e = 1.0 / (1.0 + (-g(phi_j) * (mu - mu_j)).exp());
            v_inv += g(phi_j) * g(phi_j) * e * (1.0 - e);
            sum += g(phi_j) * (score - e);
        }
        let v = 1.0 / v_inv;
        let delta = v * sum;
        let a = (self.volatility * self.volatility).ln();
        let f = |x: f64| {
            let ex = x.exp();
            ex * (delta * delta - phi * phi - v - ex) / (2.0 * (phi * phi + v + ex).powi(2))
                - (x - a) / (TAU * TAU)
        };
        let mut lo = a;
        let mut hi = if delta * delta > phi * phi + v {
            (delta * delta - phi * phi - v).ln()
        } else {
            let mut k = 1.0;
            while f(a - k * TAU) < 0.0 {
                k += 1.0;
            }
            a - k * TAU
        };
        let (mut f_lo, mut f_hi) = (f(lo), f(hi));
        for _ in 0..100 {
            if (hi - lo).abs() <= 1e-6 {
                break;
            }
            let mid = lo + (lo - hi) * f_lo / (f_hi - f_lo);
            let f_mid = f(mid);
            if f_mid * f_hi <= 0.0 {
                (lo, f_lo) = (hi, f_hi);
            } else {
                f_lo /= 2.0;
            }
            (hi, f_hi) = (mid, f_mid);
        }
        let volatility = (lo / 2.0).exp();
        let phi_star = (phi * phi + periods * volatility * volatility).sqrt();
        let phi_new = 1.0 / (1.0 / (phi_star * phi_star) + 1.0 / v).sqrt();
        Rating {
            rating: 1500.0 + SCALE * (mu + phi_new * phi_new * sum),
            deviation: (phi_new * SCALE).min(MAX_DEVIATION),
            volatility,
            at: now,
        }
    }
}

/// Everyone's rating after a game they finished in `ranks` (1 is best; equal ranks
/// tied): each player met every other.
pub fn after_game(ratings: &[Rating], ranks: &[u32], now: u64) -> Vec<Rating> {
    (0..ratings.len())
        .map(|p| {
            let opponents: Vec<(Rating, f64)> = (0..ratings.len())
                .filter(|&q| q != p)
                .map(|q| {
                    let score = match ranks[p].cmp(&ranks[q]) {
                        std::cmp::Ordering::Less => 1.0,
                        std::cmp::Ordering::Equal => 0.5,
                        std::cmp::Ordering::Greater => 0.0,
                    };
                    (ratings[q], score)
                })
                .collect();
            ratings[p].after(&opponents, now)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rated(rating: f64, deviation: f64) -> Rating {
        Rating {
            rating,
            deviation,
            volatility: 0.06,
            at: 0,
        }
    }

    #[test]
    fn glickmans_example() {
        let opponents = [
            (rated(1400.0, 30.0), 1.0),
            (rated(1550.0, 100.0), 0.0),
            (rated(1700.0, 300.0), 0.0),
        ];
        let r = rated(1500.0, 200.0).update(&opponents, 1.0, 0);
        assert!((r.rating - 1464.06).abs() < 0.01, "{r:?}");
        assert!((r.deviation - 151.52).abs() < 0.01, "{r:?}");
        assert!((r.volatility - 0.05999).abs() < 0.00001, "{r:?}");
    }

    #[test]
    fn a_game_moves_the_winner_up_and_the_loser_down_and_both_grow_surer() {
        let new = Rating::default();
        let after = after_game(&[new, new], &[1, 2], 1000);
        assert!(after[0].rating > 1500.0 && after[1].rating < 1500.0);
        assert!((after[0].rating - 1500.0 + after[1].rating - 1500.0).abs() < 1e-6);
        assert!(after.iter().all(|r| r.deviation < 350.0 && r.at == 1000));
        let tied = after_game(&[new, new], &[1, 1], 1000);
        assert!((tied[0].rating - 1500.0).abs() < 1e-6);
        // Three players: the middle one beat one and lost to one.
        let three = after_game(&[new, new, new], &[1, 2, 3], 1000);
        assert!(three[0].rating > three[1].rating && three[1].rating > three[2].rating);
        assert!((three[1].rating - 1500.0).abs() < 1e-6);
    }

    #[test]
    fn a_rating_grows_less_certain_unplayed() {
        let r = Rating {
            at: 1000,
            ..rated(1600.0, 60.0)
        };
        assert!((r.deviation_at(1000) - 60.0).abs() < 1e-9);
        let year = r.deviation_at(1000 + 365 * 86_400);
        assert!(year > 150.0 && year < 350.0, "{year}");
        assert_eq!(r.deviation_at(1000 + 100 * 365 * 86_400), 350.0);
    }
}

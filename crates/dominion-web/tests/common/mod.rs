//! Helpers shared by the tests that drive real games.

use dominion_engine::Card;
use dominion_protocol::api::{CardCost, CardInfo};
use dominion_web::cards::CardDb;

/// What `GET /api/cards` serves, built from the engine's card table.
pub fn card_db() -> CardDb {
    CardDb::new(
        Card::ALL
            .iter()
            .map(|&c| {
                let d = c.def();
                CardInfo {
                    name: d.name.into(),
                    set: d.set.to_string(),
                    types: c.type_names().into_iter().map(String::from).collect(),
                    kind: format!("{:?}", c.kind()).to_lowercase(),
                    cost: d.cost.map(|c| CardCost {
                        coins: c.coins.into(),
                        potions: c.potions.into(),
                        debt: c.debt.into(),
                    }),
                    variable_cost: d.variable_cost,
                    overpay: d.overpay,
                    coins: d.coins.into(),
                    potions: d.potions.into(),
                    vp: d.vp.into(),
                    kingdom: d.kingdom,
                    basic: d.basic,
                    removed: d.removed,
                    pile: d.pile.map(|p| p.name().into()),
                    text: d.text.into(),
                }
            })
            .collect(),
    )
}

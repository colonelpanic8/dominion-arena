//! Print an initial agent view and card reference for a chosen kingdom.
use dominion_agent::{Table, glossary_for, text};
use dominion_engine::{Card, GameConfig, PlayerId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help") {
        println!(
            "Usage: cargo run -p dominion-agent --example view -- CARD [CARD ...]\nQuote names containing spaces. Generates a two-player game with seed 11."
        );
        return Ok(());
    }
    let cards = args
        .iter()
        .map(|name| Card::from_name(name).ok_or_else(|| format!("Unknown card: {name}")))
        .collect::<Result<Vec<_>, _>>()?;
    let table = Table::new(
        "example",
        GameConfig::new(2, cards, 11),
        vec!["Ada".into(), "Bo".into()],
    )?;
    let view = table.view(PlayerId(0));
    println!(
        "{}\n\n{}",
        text::reference(&view, &glossary_for(&view)),
        text::render(&view)
    );
    Ok(())
}

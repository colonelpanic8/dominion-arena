//! Counting things in text.

/// `n` of a thing: "1 card", "2 cards", "0 cards".
pub fn amount(n: impl Into<i64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{n} {}", if n.abs() == 1 { one } else { many })
}

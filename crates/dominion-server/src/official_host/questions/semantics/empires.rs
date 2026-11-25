//! Question bindings for this set's cards (see `semantics::SETS`).
use super::{Binding, ProjectionError, WireCatalog};
use dominion_engine::{Card, Decision, View};

pub(super) fn binding(
    _view: &View<'_>,
    _d: &Decision,
    _c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    None
}

pub(super) fn mode(_card: Card, _mode: u8) -> Option<&'static str> {
    None
}

pub(super) fn ability(_card: Card, _ability: u8) -> Option<&'static str> {
    None
}

pub(super) fn symbols(_card: Card) -> &'static [(&'static str, &'static str)] {
    &[]
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(_card: Card) -> Option<&'static [&'static str]> {
    None
}

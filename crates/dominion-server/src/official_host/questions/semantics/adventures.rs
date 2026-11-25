//! Question bindings for this set's cards (see `semantics::SETS`).
use super::{Binding, Choice, ProjectionError, SetExtras, WireCatalog};
use dominion_engine::{Card, Decision, Occasion, Trigger, View};
use dominion_online::questions::LogArgument;

pub(super) const EXTRAS: SetExtras = SetExtras {
    choice,
    ability_arguments,
    decline,
};

fn choice(_view: &View<'_>, _d: &Decision) -> Option<Choice> {
    None
}

fn decline(_d: &Decision) -> Option<&'static str> {
    None
}

fn ability_arguments(
    _view: &View<'_>,
    _t: &Trigger,
    _occasion: Option<&Occasion>,
    _c: &WireCatalog,
) -> Option<Result<Vec<LogArgument>, ProjectionError>> {
    None
}

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

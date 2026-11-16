# Card knowledge

`Mind` is the shared inference core used by `dominion-engine` and `dominion-online`.
It has no dependency on either backend or on the card catalog. A mind represents one
viewer's observations, using opaque instance handles, card identities, and locations.
It keeps groups of indistinguishable copies, candidate multisets, and correlations
between subsets. Its summary reports guaranteed identities per zone, uncertainty
across zones, and known ownership. It does not track deck order or probabilities.

The adapters own visibility:

- `dominion-engine/src/knowledge.rs` translates engine traces into observations for
  each player and the public watcher. It retains the existing serialized shape of
  games' knowledge, including groups, families, and pending hidden movements.
- `dominion-online/src/knowledge.rs` assigns synthetic slots to observed cards and
  anonymous counts. Only disclosed wire identities can be associated with a slot;
  shuffles revoke those associations. The reducer handles protocol messages and
  projects the summary to `dominion/agent-view@2`.

The official adapter reserves a non-card symbol for an unidentified identity. This
preserves known composition when known cards mix with unknown ones. A partially
identified family is conservatively merged: an unknown card could be another copy
of a named candidate, so revealing it must not falsely determine a different subset.
Candidate groups containing unidentified identities are not exported as exact
candidate multisets; guaranteed counts and known ownership are still reported.

Replacement snapshots start fresh knowledge from visible information. Unexpected
zone-count corrections preserve guaranteed zone counts and unaffected observed copy
identities, but drop correlations. Neither operation infers identities from hidden
allocation IDs or claims to recover missing history.

The engine's knowledge tests exercise traces, serialization, random games, and pairs
of worlds with identical observations. The official adapter tests anonymous moves,
reveals, shuffles, ownership transfers, replacement snapshots, and random partial
observations against an independent physical-card model.

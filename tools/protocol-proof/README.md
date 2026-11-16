# Official-protocol reference fixtures

These tools execute selected parsers and serializers from a separately obtained,
hash-verified public client 2.3.7 bundle in a Node VM. No network APIs, real
credentials, DOM, or browser session are provided. They generate synthetic wire
fixtures, not recordings of account traffic. The upstream bundles are never
copied into this repository.

Place the public `dominion-webclient-head-2.3.7.min.js` and
`dominion-webclient-body-2.3.7.min.js` in a directory, then:

```sh
npm ci --prefix tools/protocol-proof --ignore-scripts
node tools/protocol-proof/generate.mjs /path/to/bundles
node tools/protocol-proof/outgoing.mjs /path/to/bundles
nix develop -c cargo test -p dominion-online
```

`generate.mjs` asks each ancillary upstream parser to consume deterministic
synthetic fields, records the corresponding bytes with the upstream Writer,
then checks those bytes again with the upstream Reader and parser. Both values
of optional booleans are exercised and nested arrays contain an element. The
Rust tests require exact consumption, every truncated prefix to fail, and a
following PONG to remain aligned. The fixtures cover the 25 ancillary opcodes;
the core game/session codecs also have focused Rust fixtures.

`outgoing.mjs` invokes all 68 upstream serializers against a fake sender. The
Rust tests compare all 68 corresponding typed commands byte for byte. No
command is transmitted, including purchases, session removal and other account
changes. These codecs are not exposed as arbitrary MCP commands.

The examples prove field layouts and dispatch coverage. They do not establish
server acceptance of every possible field value, every expansion interaction,
or every account/lobby workflow. Some upstream serializers themselves reject
operations (restricted admissions and undo table settings); those are not
invented by the adapter.

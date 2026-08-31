# stele

**Authored-intent-vs-extraction reconciliation over a semantically-typed knowledge graph, degrading to plain AGENTS.md.**

*stele* /ˈstiːli/ — an inscribed standing stone that publicly declares the law. The territory is checked against it.

A Rust single-binary that compiles a typed knowledge graph from three authored sources — the file tree, comment anchors, and typed blocks in AGENTS.md files — reconciles it against a derived graph extracted from the code, asserts the two against each other in CI (both directions: an undeclared dependency means the code broke the signature; an unbacked declaration means the doc lied), serves the graph to agents as queries, and materializes standard-compliant AGENTS.md files so every harness works with zero engine.

**Status: v1 released and self-hosting.** SPEC Draft 0.9 is the contract; the Rust engine (`stele-cli`, binary `stele`) implements it and runs on this repository (`.stele/graph.lock` is committed, CI runs `stele check` + `stele emit --check`), with the EXAMPLE.md failure gallery as its integration-test oracle. The full surface is shipped: the six-class assertion suite, `emit` with byte-identity checking, the query verbs, and MCP `stele serve` (JSON-RPC 2.0 over stdio exposing the eight read verbs as tools) — the complete degradation ladder, files → CLI → MCP. Binary releases (v0.2.0) ship per-platform from the tag-triggered release workflow, and the first external adoption is live: a real umbrella repo runs a pinned `stele check` + `stele emit --check` gate in CI. Draft 0.9 adds **undercover mode** (SPEC §3.5): private single-user graphs for a lone operator on a shared repo — the whole graph lives untracked at the parent of the git common dir, materializing one `CLAUDE.local.md` shim while `git status` stays clean. (2026-07-21)

| artifact | what it is |
| --- | --- |
| [GUIDE.md](./GUIDE.md) | the five-minute adoption path — install, scaffold, anchor, build, enforce, query |
| [SPEC.md](./SPEC.md) | the v1 specification — model, sources, assertion suite, query surface, process contract |
| [EXAMPLE.md](./EXAMPLE.md) | worked example on a fictional monorepo: every artifact, an agent session with token accounting, a CI failure gallery |
| [research/report.md](./research/report.md) | the evidence base: cited state-of-the-art survey the design decisions trace to |
| [research/claims.md](./research/claims.md) | claim ledger C1–C26 — every `[Cn]` citation in the spec resolves here |
| [research/findings/](./research/findings/) | primary-source research notes, including the prior-art sweep that positions the design |
| [research/wild-ideas.md](./research/wild-ideas.md) | the ideation corpus the design was selected from (three visions; v2 extension points derive from it) |

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/install.sh | bash
```

Or from source: `cargo install --git https://github.com/jgeschwendt/stele stele-cli`. Then see [GUIDE.md](./GUIDE.md) — `stele init` to scaffold, `stele build` + `stele emit` to compile and materialize the graph, `stele check` in CI to enforce it.

## Why this exists (the one-paragraph version)

Measured evidence says coding agents don't benefit from prose overviews of a codebase — even human-written ones — and are actively harmed by stale or redundant context. What does transmit value: concrete tooling commands, non-derivable invariants and hazards, and navigation structure delivered lazily at the point of relevance. Every existing tool either *generates* docs from code (unverified, drifting) or *checks* declared architecture forward-only (never coupled to the docs agents read). stele occupies the empty intersection: the docs are a typed, queryable graph compiled from authored intent, verified against the extracted truth of the code, in CI, both directions — and they degrade to plain markdown files any harness can read. The full argument, with citations, is in [research/report.md](./research/report.md).

## License

Copyright Joshua Geschwendt.

Licensed under the [PolyForm Noncommercial License
1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0) — the full text is in
[LICENSE.md](./LICENSE.md). Any noncommercial purpose is permitted; commercial use
requires a separate license from the author.

External contributions are not accepted — the chain of title stays with one owner.

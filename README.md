# stele

**Authored-intent-vs-extraction reconciliation over a semantically-typed knowledge graph, degrading to plain AGENTS.md.**

*stele* /ˈstiːli/ — an inscribed standing stone that publicly declares the law. The territory is checked against it.

A Rust single-binary that compiles a typed knowledge graph from three authored sources — the file tree, comment anchors, and typed blocks in AGENTS.md files — reconciles it against a derived graph extracted from the code, asserts the two against each other in CI (both directions: an undeclared dependency means the code broke the signature; an unbacked declaration means the doc lied), serves the graph to agents as queries, and materializes standard-compliant AGENTS.md files so every harness works with zero engine.

**Status: v1 implemented, self-hosting.** SPEC Draft 0.8 is the contract; the Rust engine (`stele-cli`, binary `stele`) implements it, runs on this repository (`.stele/graph.lock` is committed, CI runs `stele check` + `stele emit --check`), and the EXAMPLE.md failure gallery is its integration-test oracle. MCP `stele serve` (SPEC §5.2) is shipped: a blocking JSON-RPC 2.0 stdio server (MCP protocol 2025-11-25) exposing the eight read verbs as tools; the root AGENTS.md engine lines describe the full degradation ladder — files → CLI → MCP. (2026-07-20)

| artifact | what it is |
| --- | --- |
| [SPEC.md](./SPEC.md) | the v1 specification — model, sources, assertion suite, query surface, process contract |
| [EXAMPLE.md](./EXAMPLE.md) | worked example on a fictional monorepo: every artifact, an agent session with token accounting, a CI failure gallery |
| [research/report.md](./research/report.md) | the evidence base: cited state-of-the-art survey the design decisions trace to |
| [research/claims.md](./research/claims.md) | claim ledger C1–C26 — every `[Cn]` citation in the spec resolves here |
| [research/findings/](./research/findings/) | primary-source research notes, including the prior-art sweep that positions the design |
| [research/wild-ideas.md](./research/wild-ideas.md) | the ideation corpus the design was selected from (three visions; v2 extension points derive from it) |

## Why this exists (the one-paragraph version)

Measured evidence says coding agents don't benefit from prose overviews of a codebase — even human-written ones — and are actively harmed by stale or redundant context. What does transmit value: concrete tooling commands, non-derivable invariants and hazards, and navigation structure delivered lazily at the point of relevance. Every existing tool either *generates* docs from code (unverified, drifting) or *checks* declared architecture forward-only (never coupled to the docs agents read). stele occupies the empty intersection: the docs are a typed, queryable graph compiled from authored intent, verified against the extracted truth of the code, in CI, both directions — and they degrade to plain markdown files any harness can read. The full argument, with citations, is in [research/report.md](./research/report.md).

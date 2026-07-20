# stele

**Authored-intent-vs-extraction reconciliation over a semantically-typed knowledge graph, degrading to plain AGENTS.md.**

*stele* /ˈstiːli/ — an inscribed standing stone that publicly declares the law. The territory is checked against it.

A Rust single-binary that compiles a typed knowledge graph from three authored sources — the file tree, comment anchors, and typed blocks in AGENTS.md files — reconciles it against a derived graph extracted from the code, asserts the two against each other in CI (both directions: an undeclared dependency means the code broke the signature; an unbacked declaration means the doc lied), serves the graph to agents as queries, and materializes standard-compliant AGENTS.md files so every harness works with zero engine.

**Status: specification.** Draft 0.7, adversarially refined to convergence. Implementation has not started.

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

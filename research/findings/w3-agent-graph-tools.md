# W3 — Tools that serve a CODE GRAPH to AI coding agents (2025–2026)

Prior-art sweep for the proposed `agraph` design (Rust single-binary; compiles a _typed, authored_ knowledge graph from file tree + comment anchors + typed blocks in AGENTS.md; root node = initialContext; agents unfold nodes via GraphQL-flavored query tool-calls with typed fields commands/invariants/hazards/edges and C4 altitudes as node kinds; self-asserts in CI; materializes plain AGENTS.md as no-engine fallback).

Method: 8 web searches, 5 pages fetched in full (2 comparison/landscape, 3 primary repos). Date of sweep: 2026-07-19.

**Confidence note:** star counts and token-reduction figures below are vendor/author-stated and unverified. Several breakout repos (codegraph, gitnexus, codebase-memory-mcp) show suspiciously fast star growth and self-published arXiv papers; treat "47k stars in 5 months" and "99% fewer tokens / 120x" as marketing until independently confirmed. Their _existence, architecture, and feature surface_ are well-attested across multiple independent write-ups.

---

## The single most important finding

**`codebase-memory-mcp` (DeusData) is a near-complete implementation of our design's convergent surface** — it independently arrived at almost every structural choice we made _except the authored-first premise_. It is the primary thing to differentiate against.

- Source: https://github.com/DeusData/codebase-memory-mcp (GitHub README), site https://deusdata.github.io/codebase-memory-mcp/, paper https://arxiv.org/html/2603.27277v1 — repo, MIT, ~33k stars (claimed).
- **Graph contents:** nodes `Project, Package, Folder, File, Module, Class, Function, Method, Interface, Enum, Type, Route, Resource`; edges `CALLS, IMPORTS, DEFINES, IMPLEMENTS, INHERITS, HTTP_CALLS, ASYNC_CALLS, EMITS, LISTENS_ON, DATA_FLOWS, SIMILAR_TO`.
- **Derived, not authored** — "multi-pass indexing using tree-sitter AST analysis plus semantic type resolution (Hybrid LSP)". This is the axis on which we differ.
- **Query surface:** 15 MCP tools (semantic/BM25/structural search, tracing, arch analysis, raw **Cypher**) + a **CLI** mode (`codebase-memory-mcp cli search_graph '...'`) + optional 3D graph UI at :9749. No GraphQL; no typed intent fields (invariants/hazards).
- **Progressive disclosure: YES** — `get_architecture` returns "languages, packages, entry points, routes, hotspots, boundaries, layers, and clusters in a single call" as a root overview that unfolds into deeper queries. This is exactly our root-node/initialContext idea, minus the authored typed fields.
- **Emits agent-instruction files: YES** — installer "generates AGENTS.md, SKILL.md, and durable context hooks for 43 client surfaces … tiered Scout/Verify/Auditor profiles." But AGENTS.md here is an _output pointer to the MCP server_, not the _authored source_ the graph compiles from — inverted from us.
- **CI self-verification: PARTIAL / DIFFERENT** — "CodeQL SAST — blocks release pipeline if any open alerts remain" + VirusTotal binary scanning. This verifies _the tool's own binary_, NOT the graph's assertions against the code. No reconciliation of declared-vs-extracted edges. Our CI self-assertion contract remains unclaimed.
- **Distribution:** written in **C (88.7%) / C++**, "single static binary for macOS, Linux, and Windows … Zero dependencies." Directly parallels our Rust-single-binary choice (we are not first to "single native binary code-graph over MCP"; we would be first in Rust, which is not a differentiator).

**Verdict:** closest overall. Misses three conjuncts: (1) authored/typed-intent graph — it's purely AST-derived; (2) typed semantic fields + C4 altitudes as first-class node kinds; (3) graph-self-assertion in CI (its CI is binary-security only).

---

## Landscape comparison (rywalker.com/research/code-intelligence-tools, pub 2026-03-15, upd 2026-06-11)

An 18-tool survey — best single map of the category. Its own summary: **"None of the tools explicitly document AGENTS.md/CLAUDE.md emission, CI self-verification mechanisms, or progressive disclosure beyond incremental indexing."** (Note: this predates/omits codebase-memory-mcp, which does the first two.) MCP dominates transport (14/18); all graphs are **derived** from AST/LSP; none expose GraphQL or typed intent fields.

Tier-1 knowledge-graph engines:

- **CodeGraph** (colbymchenry) — https://github.com/colbymchenry/codegraph — TypeScript + embedded SQLite single-file; imports/calls/defs/extends, 21 langs; MCP, 8 agent integrations; file-watcher incremental sync; derived. "Local SQLite symbol/call graph over MCP." No authored fields, no AGENTS.md emission noted, no graph CI.
- **CodeGraph** (codegraph-ai) — https://github.com/codegraph-ai/CodeGraph — separate project; "42 MCP tools, 38 languages, VS Code ext, persistent memory layer." Derived semantic graph.
- **GitNexus** — TypeScript, LadybugDB (native/WASM, zero-server); 16 MCP tools + 7 resources + skills + Claude Code hooks; derived; noncommercial license. https://www.termdock.com/en/blog/gitnexus-code-intelligence-knowledge-graph
- **CodeGraphContext** — https://github.com/CodeGraphContext/CodeGraphContext — Python; pluggable backends (FalkorDB Lite/KuzuDB/Neo4j); MCP + CLI; re-index (not incremental); 22 langs.
- **Axon** — Python, WebGL viz; **stalled since 2026-03-25**.

Tier-2 symbol/semantic:

- **Serena** — Python, LSP-over-MCP, symbol retrieval + editing/refactor, 40+ langs. Live LSP (real-time), derived.
- **claude-context** — TS, hybrid BM25+vector over AST chunks, Merkle-tree incremental, Milvus/Zilliz.
- **grepai** — **Go, single binary, 100% local via Ollama**; call-graph tracing + semantic search; CLI daemon + MCP; "independently benchmarked 97% reduction in Claude Code input tokens." Another single-binary precedent (Go, not Rust).
- **Octocode MCP** — TS, 14 MCP tools, LSP nav + PR archaeology + GitHub multi-repo.
- **CodePathFinder** — Go, cross-file taint analysis, 211 security rules, CLI.
- **mcp-vector-search** — Python, LanceDB semantic + knowledge graph + complexity/dead-code.

Tier-3 context packing (not graphs but adjacent):

- **Repomix** — TS, XML-structured pack, tree-sitter ~70% token compression; **authored output**, flat (no graph). CLI + MCP.
- **code2prompt** — Rust, Handlebars templates; no release since 2025-12.
- **Aider repo-map** (below).

Tier-4 commercial/cloud: **Augment Context Engine** (closed, MCP GA 2026-02, "70%+ agent quality gains" claimed), **Sourcegraph Cody** (cloud, RAG over codebase), **DeepWiki** (AI-gen docs), **Greptile** (below).

---

## Individual tools requested

### Nuanced (nuanced-dev/nuanced-py)

- https://github.com/nuanced-dev/nuanced-py · https://docs.nuanced.dev/overview · MIT · Python. **ARCHIVED / read-only since 2026-03-05.**
- Enriched **Python** function call graphs, **derived via static analysis**; models "what calls what, and under what conditions" (execution paths, unlike static LSP symbol indexing).
- Query surface: CLI (`nuanced init`, `nuanced enrich`) + library API; no MCP/AGENTS.md/GraphQL. No CI self-verification. No progressive-disclosure root.
- **Verdict:** narrow (Python call graph only), derived, now dead. Confirms "enriched call graph for agents" is a validated _need_ but not a live competitor. Misses nearly every conjunct (authored, typed fields, CI, AGENTS.md, altitudes).

### potpie.ai (potpie-ai/potpie)

- https://github.com/potpie-ai/potpie · docs.potpie.ai · Apache-2.0 · Python (97%, some Rust/TS). Raised $2.2M ("knowledge graph for code"). Originally Neo4j property graph (files/functions/classes/imports/calls) + CrewAI/langgraph RAG agents.
- **Most interesting evolution: partially authored.** Current README shows `potpie record --type <type> --summary` to explicitly author "decisions, conventions, runbooks" alongside derived code/PR/issue indexing. Graph now spans code + workflow + team knowledge + Linear/Jira/Confluence.
- Query surface: CLI (`potpie search`, `potpie resolve`, `potpie graph …` low-level reads), web graph explorer (`potpie ui`); no GraphQL/Cypher exposed in README.
- Agent files: `potpie skills install --agent <agent>` "install or refresh Potpie guidance for an agent harness" — skill materialization, but no explicit AGENTS.md/CLAUDE.md.
- CI: `potpie doctor` = local diagnostics for "daemon, backend capabilities, and **skill drift**" — a drift check, but local, not CI graph-vs-code assertion.
- **Verdict:** closest on the _authored_ axis (typed `record` entries + drift detection), but it's an additive layer over a derived AST graph, not a graph _compiled from_ authored typed blocks; no C4 altitudes, no GraphQL typed fields, no bidirectional edge reconciliation in CI, no engine-free AGENTS.md fallback. Watch closely — converging toward us from the platform side.

### Greptile

- https://www.greptile.com/ · https://www.greptile.com/agent · cloud SaaS, closed.
- Builds a **Semantic Code Graph** before review: "every function, variable, class, file, and directory relates to every other" — import chains, inheritance, API call chains, cross-module deps. **Derived.**
- Used internally for **multi-hop PR investigation**; not exposed as a queryable MCP/GraphQL surface to third-party agents. v4 adds per-comment confidence scores.
- **Verdict:** graph is an internal engine for code review, not a served context layer. Confirms "derived semantic graph beats diff-only" but off-axis for us (not agent-facing, not authored, closed, no AGENTS.md/CI-assert/altitudes).

### Sourcegraph Cody / Amp

- Cody Free/Pro terminated 2025-07-23; individuals moved to **Amp** (agentic). Cody Enterprise-only (~$59/user/mo), "Code Graph technology" + Universal Codebase Context via embeddings + Sourcegraph search (v2026.1), RAG across repos.
- **Derived**, cloud/self-host, query via API + web; no GraphQL typed-intent surface, no authored graph, no AGENTS.md compilation, no graph-self-assert.
- **Verdict:** enterprise cross-repo derived-index + RAG. Scale story, not our authored/typed/portable story. Misses all distinctive conjuncts.

### Aider repo-map (viewed as a graph)

- https://aider.chat/2023/10/22/repomap.html · https://aider.chat/docs/repomap.html · Python, built into Aider (also ported: RepoMapper, and a "Repo Map MCP Server").
- Directed graph, files = nodes, references = edges; **tree-sitter extraction (derived)**; **NetworkX PageRank** with personalization on chat context; token-budgeted (`--map-tokens`, default 1k) — selects most-referenced identifiers.
- **Verdict:** the canonical "repo as ranked graph" precedent and the origin of _token-budgeted graph selection_ (a conjunct we share). But derived, ephemeral per-chat, no persistence, no authored fields, no MCP served surface, no CI, no AGENTS.md. Its PageRank + token-budget idea is worth borrowing for our node-ranking/exhaustiveness checks.

### code-graph-rag / Neo4j-based projects (the "GraphRAG over code" cluster)

- **Code-Graph-RAG** — tree-sitter multi-language → knowledge graph → NL query & edit via **MCP server**. Derived.
- **eric050828/graph-codebase-mcp** — Python AST → Neo4j + OpenAI embeddings, MCP; NL→Cypher. https://github.com/eric050828/graph-codebase-mcp
- **Abhishek-Aditya-bs/CodeGraph** — Neo4j entity graph + 3072-dim vector index, hybrid structural+semantic. https://github.com/Abhishek-Aditya-bs/CodeGraph
- **ChrisRoyse/CodeGraph** — https://github.com/ChrisRoyse/CodeGraph
- **Microsoft GraphRAG** (https://github.com/microsoft/graphrag) — general text GraphRAG, not code-specific; note "~$33K indexing cost for large datasets" drove the 2026 wave toward cheaper local variants. Memgraph "Graph-Code" demo shows the coding-assistant pattern.
- **Verdict (whole cluster):** all **derived** (AST→Neo4j/Kuzu/Falkor + embeddings), query via NL→Cypher over MCP, none authored, none with typed intent fields/C4 altitudes, none self-asserting graph-vs-code in CI, none materializing AGENTS.md as fallback. This is the crowded commodity center; our design is not competing here.

### GraphQL-specific surface

No code-graph tool found **exposes repo structure via GraphQL.** Query surfaces are: MCP tools (dominant), Cypher (Neo4j-backed ones), CLI, and NL→query. Our "GraphQL-flavored typed-field unfolding" appears **genuinely unoccupied** as a query ergonomic. (GitHub's own GraphQL API serves repo _metadata_, not a semantic code graph, and is out of scope.)

---

## Where the genuine gap is (per-conjunct)

| Conjunct of `agraph`                                                                                                                        | State of the art                                                                                                                                                                 | Gap?                                                                                                          |
| ------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Single native binary, local, MCP-served graph                                                                                               | **Occupied** — codebase-memory-mcp (C), grepai (Go), CodeGraph (TS+SQLite)                                                                                                       | No. Rust is cosmetic.                                                                                         |
| Compiles a structural graph (files/symbols/calls/imports)                                                                                   | **Fully commodity** — every tool                                                                                                                                                 | No.                                                                                                           |
| **Graph is AUTHORED** (typed blocks in AGENTS.md + comment anchors as the _source_)                                                         | **Nearly empty** — all derive from AST/LSP. Potpie's `record` and drift-check is the only partial precedent; codebase-memory-mcp _emits_ AGENTS.md but doesn't _compile from_ it | **YES — core gap.**                                                                                           |
| **Typed semantic fields** (commands/invariants/hazards/edges) + **C4 altitudes as node kinds**                                              | **Empty** — schemas are syntactic (Class/Function/Route), never intent/risk-typed; no C4-altitude node model found                                                               | **YES — sharpest differentiator.**                                                                            |
| Root node = initialContext, unfolded by query tool-calls (progressive disclosure)                                                           | **Occupied** — codebase-memory-mcp `get_architecture`; documented as a pattern (Ardalis, Medium)                                                                                 | No — but ours unfolds _authored typed fields_, not derived structure.                                         |
| GraphQL-flavored query ergonomic                                                                                                            | **Empty** — MCP/Cypher/NL only                                                                                                                                                   | YES (minor — ergonomic, not moat).                                                                            |
| **CI self-assertion** (declared-vs-extracted edges _both directions_, anchor liveness, token budgets, exhaustiveness, freshness watermarks) | **Empty** — codebase-memory-mcp CI = binary security (CodeQL/VirusTotal); potpie `doctor` = local drift only; aider = token budget only, no verification                         | **YES — strong gap.** The "doc proves itself against the code, bidirectionally, in CI" contract is unclaimed. |
| Materializes plain AGENTS.md as **no-engine fallback** (engine-optional portability)                                                        | **Empty in this direction** — others generate AGENTS.md as a _pointer to the required MCP server_; none treat flattened AGENTS.md as a first-class degrade-gracefully artifact   | **YES.**                                                                                                      |

**Net:** the convergent half of the design (single-binary local graph, MCP, progressive-disclosure root, token budgeting) is already well-built — codebase-memory-mcp is the reference implementation to study and out-position. The **defensible, unoccupied core** is the _authored-first, intent-typed, C4-altitude graph that self-asserts against the derived import graph in CI and degrades to plain AGENTS.md_. No surveyed tool holds more than one of those four; Potpie holds ~1.5 and is the fastest-converging threat. Prior art validates the demand without foreclosing the design.

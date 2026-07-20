# W3 — Existing Code Knowledge-Graph Infrastructure

Prior-art sweep for `agraph` (Rust single-binary that compiles a typed knowledge graph from a
repo: file tree + structured comment anchors + typed AGENTS.md blocks; root node = session
initialContext; agents unfold nodes via GraphQL-flavored query tool-calls; graph self-asserts in
CI; materializes plain AGENTS.md as no-engine fallback).

Scope of this file: **infrastructure that derives a queryable graph/fact DB from source code.**
Siblings own MCP agent tools, architecture linters, and comment grammars.

Method: 6 searches, 5 full-page fetches (official docs/repos), one backward chain (scope-graphs →
stack-graphs lineage). Dates are access dates (2026-07-19); quotes <15 words.

---

## Meta Glean

- Sources: [engineering.fb.com blog, 2024-12-19](https://engineering.fb.com/2024/12/19/developer-tools/glean-open-source-code-indexing/) (vendor blog); [github.com/facebookincubator/Glean](https://github.com/facebookincubator/Glean) (repo); [glean.software docs](https://glean.software/docs/angle/guide/) (official docs). Accessed 2026-07-19.
- **Graph contents:** typed, schema-defined _facts_ — declarations, references, type info,
  inheritance, call/xref relationships, per-symbol location. Language-specific schemas plus a
  derived language-neutral layer.
- **Query surface:** **Angle**, a Datalog/logic-style declarative query language (anagram of
  "Glean"). Access via shell (Docker), a server, and client libraries. RocksDB storage backend;
  basic lookups "in about a millisecond."
- **Derived deterministically?** Yes. Indexers use each language's native compiler to extract
  facts; **derived predicates** compute more facts either at query time or `stored` ahead of time.
  Deterministic given the same compiler inputs.
- **Agent/AI/doc-facing?** Yes — the strongest of the set. Blog explicitly cites automatic
  **documentation generation** from API/comment data and **RAG in AI coding assistants**, plus
  "diff sketches." This is the closest existing system to our thesis (graph → agent context).
- **License / embeddability:** BSD. Implemented in **Haskell** (+ Hack, C++). _Not_ embeddable in
  a Rust single binary — it's a server + RocksDB deployment, heavy runtime. Wrong shape for
  `agraph`'s portability constraint.
- **Verdict — STEAL (concepts), don't adopt.** Steal: derived-predicate model (declared facts +
  computed facts from one deterministic pass), the language-neutral derived layer, and the
  explicit "facts → agent RAG / doc-gen" framing as validation that our direction is real. Don't
  adopt the artifact: Haskell server + RocksDB is the opposite of a portable Rust binary with a
  plain-file fallback.

## Google Kythe

- Sources: [kythe.io storage model](https://kythe.io/docs/kythe-storage.html), [schema reference](https://kythe.io/docs/schema/), [overview](https://kythe.io/docs/kythe-overview.html) (official docs). Accessed 2026-07-19.
- **Graph contents:** directed graph of **nodes** (functions, vars, types) + **edges** (defines,
  refs, inheritance, overrides) + **anchors** tying nodes to source spans. A _fact_ = named
  bytestring on a node; `doc/uri` fact attaches external documentation to a node.
- **Query surface:** entry store (source/kind/target triples); CLI tools incl. `xrefs` for
  cross-references. Lower-level than Angle/QL — a storage+serving model, not a rich query language.
- **Derived deterministically?** Yes — per-language indexers run against compiler output; schema
  is "liberal and extensible," new node/edge kinds without a central authority.
- **Agent/AI/doc-facing?** Partial/latent: the `doc/uri` fact and doc extraction exist, but no
  agent-consumption story. Predates the LLM era; effectively unmaintained/dormant.
- **License / embeddability:** Apache-2.0. C++/Go/Java toolchain; Bazel-centric. Not a Rust
  embeddable and not a single binary.
- **Verdict — STEAL (schema philosophy), don't adopt.** Steal: the **anchor** primitive (graph
  node ↔ source span) directly models our "structured comment anchors," and the extensible
  node/edge-kind schema mirrors our C4-altitude node kinds + typed edges. Steal the `doc/uri`
  idea: a first-class edge from a code node to a doc node. Don't adopt: heavyweight,
  build-system-coupled, dormant.

## SCIP / LSIF (Sourcegraph)

- Sources: [github.com/sourcegraph/scip](https://github.com/sourcegraph/scip) (repo, Apache-2.0), [announcing SCIP blog](https://sourcegraph.com/blog/announcing-scip) (vendor blog), [scip-code.org](https://scip-code.org/). Accessed 2026-07-19.
- **Graph contents:** per-document **symbols**, **occurrences** (ranges), **documentation**
  strings, and **relationships** (implementations, type-defs). Human-readable string symbol IDs
  (`<scheme> <manager> <package> <version> <descriptor>`) — replaces LSIF's opaque numeric monikers.
- **Query surface:** SCIP is a **transmission format**, not a query engine — a Protobuf schema
  (`scip.proto`) that producers (indexers) emit and consumers (Sourcegraph) ingest. Powers
  go-to-def / find-refs / find-implementations. LSIF is the older graph-encoded predecessor SCIP
  replaces (SCIP is "easier to produce, easier to debug").
- **Derived deterministically?** Yes — compiler-backed indexers (e.g. rust-analyzer emits SCIP
  natively) or heuristic indexers; deterministic per indexer.
- **Agent/AI/doc-facing?** No direct agent story; carries doc strings but aimed at IDE nav UIs.
- **License / embeddability:** Apache-2.0. **Rich Rust bindings** exist; `rust-analyzer` emits
  SCIP. The _format_ is trivially embeddable; there is no engine to embed.
- **Verdict — ADOPT (as an interop format / import path).** SCIP is the industry-standard,
  Apache-licensed, Protobuf-defined interchange for symbol/occurrence/xref data with **Rust
  bindings and native rust-analyzer emission**. `agraph` should treat SCIP as an _input_ to
  populate the extracted-import/reference side of its "declared depends-edges vs extracted graph"
  self-assertion, rather than reinventing symbol resolution. Not our whole graph (no C4 altitudes,
  no typed AGENTS.md blocks), but the cheapest correct source of the code-fact half.

## GitHub stack-graphs (Rust)

- Sources: [github.com/github/stack-graphs README](https://github.com/github/stack-graphs/blob/main/README.md) (repo), [introducing stack graphs blog](https://github.blog/open-source/introducing-stack-graphs/) (vendor blog), [docs.rs/stack-graphs](https://docs.rs/stack-graphs/latest/stack_graphs/). Accessed 2026-07-19.
- **Graph contents:** name-binding graph for **name resolution** / code navigation — not a
  general fact DB. Based on Eelco Visser's **scope-graphs** framework (TU Delft) — the backward
  chain.
- **Query surface:** path-finding over the stack graph to resolve a reference to its definition;
  incremental (per-file shards stitch together).
- **Derived deterministically?** Yes, and notably **without tapping the build system** — rules are
  declarative, generated from tree-sitter parse. Efficient + incremental is the headline.
- **Agent/AI/doc-facing?** None in-repo. (Notably, OpenHands filed an issue exploring stack-graphs
  for agent "repo map / context" — external interest exists but unbuilt.)
- **License / embeddability:** **Dual Apache-2.0 / MIT, pure Rust, embeddable crate** — exactly
  our target shape. **BUT archived 2025-09-09: "no longer supported or updated by GitHub,"**
  recommends forking.
- **Verdict — STEAL (mechanism), adopt-with-eyes-open.** The **incremental, build-system-free,
  tree-sitter-driven** approach is precisely `agraph`'s desired posture, and it's Rust+MIT. Steal
  the incrementality model (per-file shards) for freshness watermarks. Risk: **deprecated/archived**
  — adopting the crate means owning a fork. Best used as a design reference and, if name
  resolution is needed, a vendored/forked dependency, not a live upstream.

## tree-sitter-graph (crate)

- Sources: [github.com/tree-sitter/tree-sitter-graph](https://github.com/tree-sitter/tree-sitter-graph/) (repo), [docs.rs/tree-sitter-graph](https://docs.rs/tree-sitter-graph/latest/tree_sitter_graph/), [reference](https://docs.rs/tree-sitter-graph/latest/tree_sitter_graph/reference/index.html). v0.12. Accessed 2026-07-19.
- **What it produces:** a **DSL for constructing arbitrary graph structures from a tree-sitter
  parse**. A `.tsg` file = **stanzas**: each is a tree-sitter query pattern + a statement block
  that creates nodes, links edges, and annotates both with **arbitrary attributes**. Explicitly
  _not_ limited to trees and _not_ required to line up with the syntax tree.
- **Query surface:** none — it's a _builder_, not a query engine. Output graph is consumed
  downstream (stack-graphs is the canonical consumer, via `tree-sitter-stack-graphs`).
- **Derived deterministically?** Yes — pure function of (parse tree, DSL rules).
- **Agent/AI/doc-facing?** No — pure infrastructure.
- **License / embeddability:** **Dual Apache-2.0 / MIT, Rust crate** (`tree-sitter-graph = "0.12"`),
  library or CLI. Actively the lower layer beneath stack-graphs. Ideal single-binary fit.
- **Verdict — ADOPT. This is very likely our extraction layer.** It is _exactly_ the primitive the
  design needs: deterministic, declarative extraction of an **arbitrary attributed graph** (not
  forced to mirror the AST) from tree-sitter parses, as an embeddable Rust+MIT crate. Our
  "structured comment anchors → typed nodes/edges with attributes (commands/invariants/hazards)"
  maps directly onto tsg stanzas + attributes. `agraph` can define `.tsg` stanzas to lift comment
  anchors and code structure into node/edge form, then layer its own typed schema + query surface
  (the GraphQL-flavored unfold) on top. Caveat: tree-sitter parses _code_, not AGENTS.md markdown —
  the AGENTS.md typed-block half needs a separate parser (markdown/YAML), so tsg covers the
  code+comment-anchor half only.

## CodeQL

- Sources: [codeql.github.com](https://codeql.github.com/), [about-codeql docs](https://codeql.github.com/docs/codeql-overview/about-codeql/), [github/codeql LICENSE](https://github.com/github/codeql/blob/main/LICENSE). Accessed 2026-07-19.
- **Graph contents:** a **relational database** of facts per language (each language has its own
  schema of relations); includes copied source + relational data. "Query code as though it were
  data."
- **Query surface:** **QL** — an object-oriented, Datalog-derived logic language ("resembles SQL
  with OO extensions"). Rich, mature, optimized for security dataflow queries.
- **Derived deterministically?** Yes — database built by a language extractor during (a trap of)
  compilation; deterministic per build.
- **Agent/AI/doc-facing?** Aimed at security researchers / code scanning (GHAS), not agents/docs.
- **License / embeddability:** **Split license — trap.** QL libraries/queries are **MIT**, but the
  **CodeQL engine is NOT open source**: free only for research/OSS; commercial proprietary use
  requires GHAS (per-committer). **Cannot embed the engine in a distributed Rust binary.**
- **Verdict — STEAL (query-model inspiration only), do NOT adopt.** The "code as a queryable
  database + declarative query language" thesis is validated here, and QL's schema-of-relations is
  worth studying for our typed-field query surface. But the **non-OSS engine license is
  disqualifying** for an embeddable single binary, and its dataflow-security focus is orthogonal to
  agent-context/doc generation. Reference, not dependency.

---

## Cross-cutting synthesis — where the genuine gap is

| System            | Graph =                     | Query engine      | Deterministic     | Agent/doc-facing       | Rust-embeddable / license          |
| ----------------- | --------------------------- | ----------------- | ----------------- | ---------------------- | ---------------------------------- |
| Glean             | typed facts + derived       | Angle (Datalog)   | yes               | **yes (RAG, doc-gen)** | no (Haskell server) / BSD          |
| Kythe             | nodes/edges/anchors + facts | store + xrefs CLI | yes               | latent (`doc/uri`)     | no (C++/Bazel) / Apache-2.0        |
| SCIP/LSIF         | symbols/occurrences/rels    | none (format)     | yes               | no                     | **yes (bindings)** / Apache-2.0    |
| stack-graphs      | name-binding graph          | path resolution   | yes (incremental) | no (external interest) | **yes** / MIT+Apache, **archived** |
| tree-sitter-graph | arbitrary attributed graph  | none (builder)    | yes               | no                     | **yes** / MIT+Apache, active       |
| CodeQL            | relational fact DB          | QL (OO Datalog)   | yes               | no                     | no (**engine not OSS**) / split    |

**What already exists (don't rebuild):**

1. **Deterministic extraction of an attributed graph from source** — solved cleanly and
   embeddably by **tree-sitter-graph** (+ stack-graphs for name resolution). This is `agraph`'s
   extraction layer; adopt it.
2. **A standard, Rust-friendly interchange for symbol/xref facts** — **SCIP** (rust-analyzer emits
   it). Adopt as the import path for the "extracted import graph" side of CI self-assertion.
3. **Derived-fact model + declarative query over code facts** — proven by Glean/CodeQL/Kythe.
   Steal the model; none is embeddable.
4. **The "graph → agent RAG / doc-gen" thesis** — only **Glean** has shipped this framing,
   validating the direction.

**The genuine gap `agraph` fills (no prior art):**

- **AGENTS.md typed blocks as first-class graph nodes.** Every system above extracts from _code_
  (via compiler or tree-sitter). None models **hand-authored, typed prose/config blocks**
  (commands/invariants/hazards) as nodes, nor a **file tree + comment anchors + AGENTS.md** in one
  graph. tree-sitter-graph covers code+comment anchors; the AGENTS.md half is unserved.
- **C4-altitude node kinds + a root `initialContext` node designed as an agent-unfold entry point.**
  Existing graphs are flat symbol/xref graphs; none has an intentional altitude hierarchy or a
  designated session-root node with progressive disclosure via query tool-calls.
- **The graph self-asserting in CI** — declared depends-edges vs extracted import graph (both
  directions), anchor liveness, per-harness token budgets, exhaustiveness, freshness watermarks.
  Kythe/SCIP produce facts; **none turns the fact-vs-declaration delta into a build gate.** This is
  the sharpest novelty: prior art _derives_ a graph; `agraph` _reconciles a declared graph against
  a derived one and fails CI on drift._
- **Plain-AGENTS.md materialization as a no-engine fallback.** Every system above requires its
  engine/server to be useful. A design goal of degrading to portable plain files has no analog.

**Net:** the extraction and interchange layers are commodity (tree-sitter-graph + SCIP, both
Rust+permissive). The **typed-authored-doc modeling, the agent-oriented altitude/root/unfold
surface, the CI reconciliation gate, and the plain-file fallback** are the unclaimed territory.
Build there; adopt below.

_(This file is not thin — all six systems + LSIF covered with primary sources.)_

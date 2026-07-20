# W3 — Schema-Constrained Documentation (typed doc content for machine consumption)

Research date: 2026-07-19. Method: 6 searches, 4 full fetches, one chain (search →
repo → SPEC.md). **Not thin** — the two named targets (Charter/ADF) are directly
on-point and richly documented; the graph angle turned up live competitors. Quotes
kept < 15 words.

**Bottom line up front.** Two answers to the key questions:

1. _Does anything enforce WHAT KINDS of content docs may contain (commands vs
   invariants vs prose) via schema?_ — **No.** The closest system (ADF) types the
   **shape** of a section body (text/list/map/metric) and its **weight**
   (load-bearing/advisory), but explicitly does **not** constrain semantics. Nobody
   ships a schema that says "this block is an invariant, that one is a command."
2. _Does anything render typed doc nodes into per-harness agent files?_ — **Yes,
   Charter/ADF does exactly this** (`compile --target claude|all` →
   CLAUDE.md/AGENTS.md/.cursorrules/GEMINI.md). This is the single largest overlap
   with the proposed `agraph`.

**Verdict on the framing question — "has anyone shipped typed-fields-as-curation-bar
for agent docs?":** _Partially, and only along one axis._ ADF ships **structural
typing + metric ceilings + token budgets as a CI-enforced bar**, and Graphify/codegraph
ship **extracted code graphs with EXTRACTED-vs-INFERRED edge provenance + drift
hooks**. Nobody has shipped the combination the design proposes: an **authored,
semantically-typed** doc graph (commands/invariants/hazards as distinct node kinds)
that **self-asserts declared-edges against the extracted import graph, both
directions**, in CI. That intersection is the genuine gap.

---

## Target 1 — Charter / ADF (the closest prior art) ★

- **URL:** https://github.com/Stackbilt-dev/charter · spec repo
  https://github.com/adf-spec/adf · SPEC.md
  https://raw.githubusercontent.com/adf-spec/adf/main/SPEC.md · landing
  https://agentdocsspec.com/ (NOTE: this domain is a _different_ project — see
  Target 5; do not conflate)
- **Source type:** GitHub repos + raw spec text (primary)
- **What it is:** Charter is a "local-first CLI for AI agent governance" whose format
  is **ADF = Attention-Directed Format** (the SPEC.md calls it "vendor-neutral
  specification for modular, progressively-disclosed AI agent context"). Small
  `.adf` modules in `.ai/`, loaded on demand by trigger keywords.

### The ADF schema, audited (how typed is it _really_)

**Directory / manifest layer (typed):**

- `.ai/manifest.adf` (module registry), `core.adf`, `state.adf`, on-demand modules.
- Manifest fields: `ROLE` (one-line text), `DEFAULT_LOAD` (list of module paths),
  `ON_DEMAND` (list of `path (Triggers on: kw1, kw2, …)`), `BUDGET.MAX_TOKENS`
  (a metric: `value / ceiling [unit]`).

**Section header grammar (typed, regex-strict):**

- Form: `[emoji]? KEY [weight]: value` where `KEY` matches `[A-Z][A-Z0-9_]*`,
  `weight` ∈ `[load-bearing]` | `[advisory]` (advisory = default).
- `[load-bearing]` = "binding constraint. Tools MUST surface violations."
- `[advisory]` = "guidance. Tools MUST NOT fail."

**Body-form classification — FOUR types, and this is the ceiling of its typing:**

- `text` (free-form / inline value), `list` (`- item`), `map` (`KEY: value`),
  `metric` (`key: value / ceiling [unit]`). Classification order: list → metric →
  map → text fallback.

**Metric ceilings (deterministic, CI-enforced):**

- Exact syntax: `key: value / ceiling [unit]`. Violation = `value > ceiling` inside
  a load-bearing section. Charter's pre-commit hooks "reject code that exceeds
  ceilings." Example seen: `entry_loc: 142 / 500 [lines]`.

**Trigger resolution grammar (deterministic, no LLM):**

- Lowercase keyword vs lowercase trigger; **exact equality OR prefix-stem match
  where prefix length ≥ 66% of full trigger**. (`"validat"` matches `"validation"`,
  7/10 = 70%.) "String matching only; no LLM call required."

**Token budgets:** first-class, per manifest (`BUDGET.MAX_TOKENS` as a metric with
ceiling). This directly prefigures the design's "token budgets per harness."

**Rendering into per-harness files (the big overlap):**

- `charter adf compile --target claude` → CLAUDE.md;
  `--target all --write` → CLAUDE.md + AGENTS.md + .cursorrules + GEMINI.md.
- Spec framing: "**ADF is source; vendor files are build artifacts**." Exactly the
  design's "materialize plain AGENTS.md as a no-engine fallback."

### The critical limitation (where the design still differs)

SPEC.md is explicit: it **does NOT constrain content semantics.** From the audit:
"Content kinds are presentational layers (text, list, map, metric) only. The
specification imposes no schema on _what_ instructions, rules, or invariants modules
may declare — only _how_ they're structured and when they load." Parsing "MUST be
tolerant… mixed content degrade[s] gracefully."

So ADF is **medium machine-checkability**: strict structural parse + strict metrics +
strict triggers; **loose on body semantics**. There is no `commands` vs `invariants`
vs `hazards` node kind, no graph, no edges, no C4 altitudes, no declared-vs-extracted
import cross-check. ADF loads modules by keyword; it does not link them into a typed
graph you unfold by query.

**Overlap map (ADF vs proposed agraph):**

| agraph plank                                     | ADF status                                  |
| ------------------------------------------------ | ------------------------------------------- |
| Typed blocks in AGENTS.md                        | Partial — shape/weight typed, semantics NOT |
| Token budgets per harness                        | ✔ `BUDGET.MAX_TOKENS` metric                |
| CI self-assertion (metric ceilings)              | ✔ pre-commit rejects > ceiling              |
| Materialize per-harness files                    | ✔ `compile --target all`                    |
| Typed node kinds (commands/invariants/hazards)   | ✘                                           |
| Knowledge graph + typed edges                    | ✘ (keyword load, not graph)                 |
| C4 altitudes as node kinds                       | ✘                                           |
| Declared-depends vs extracted-import cross-check | ✘                                           |
| Anchor liveness / freshness watermarks           | ✘                                           |

---

## Target 2 — Graphify / codegraph / create-context-graph (the GRAPH frontier) ★

- **URLs:** https://github.com/Graphify-Labs/graphify ·
  https://github.com/colbymchenry/codegraph ·
  https://github.com/neo4j-labs/create-context-graph ·
  https://graphify.net/graphify-claude-code-integration.html
- **Source type:** GitHub repos + vendor page
- **What they are:** Tools that build a **queryable code knowledge graph** for agents.
  "Parses… extracts entities like functions, classes, files, concepts… stores
  entities as nodes and relationships as labeled edges… exposes a query interface"
  (Graphify). codegraph: tree-sitter parse → symbols/edges/files in SQLite (FTS5),
  exposed to Claude Code/Cursor/Codex/opencode **over MCP**.
- **Two features that strongly prefigure the design:**
  - **Edge provenance:** "Each connection is tagged EXTRACTED (explicit in the
    source) or INFERRED (resolved by graphify)." — This is the design's
    declared-vs-extracted distinction, _but_ applied to a machine-extracted graph,
    not authored-vs-extracted reconciliation.
  - **Drift prevention via hooks:** graph "only refreshes when Claude touches a
    file… every terminal/IDE/other-tool commit drifts"; fix is post-commit/
    post-checkout hooks that rebuild after every commit/branch switch.
  - CLAUDE.md integration: "small marker-fenced CodeGraph section in the agent's
    instructions file (CLAUDE.md / AGENTS.md / GEMINI.md)."

**Where they differ from agraph:** these graphs are **extracted FROM code** (bottom-up,
tree-sitter). They are not an **authored, typed doc graph** where a human writes
`commands`/`invariants`/`hazards`/`edges` as first-class typed fields. There is no
CI assertion that a _human-declared_ depends-edge matches the extracted import graph
(both directions) — the graph _is_ the extraction; nothing is declared to check it
against. No C4 altitude typing, no per-harness token budgets, no plain-AGENTS.md
materialization as an engine-free fallback (they require the MCP server / SQLite db).

This is the closest living competitor to the graph half of the design, and the
cleanest place to point at the gap: **they extract; the design also asserts authored
intent against the extraction.**

---

## Target 3 — hcc (hierarchical-context-compressor), schema angle

- **URL:** https://github.com/reyavir/hierarchical-context-compressor
- **Source type:** GitHub repo
- **Schema verdict: essentially untyped.** HCC generates **markdown** doc maps via a
  three-phase LLM workflow. Output: root `agents.md` (ToC) + `llms.txt`, per-directory
  `AGENTS.md` "operational manuals." Sections are **prose headings** ("Setup &
  Commands", "Code Style & Patterns", "Implementation Details") — not schema fields.
- **The one hard constraint is a budget, not a type:** each generated `AGENTS.md`
  body (below `### Local Agent Context`) "is limited to **100 lines**"; phase 1 caps
  documented dirs at `--max-dirs` (default 15). "Prioritizes narrative concision over
  structured data typing."
- **Relevance:** confirms the hierarchical/compressed-root → unpack model in the
  wild, but as generated prose, not a typed graph. No content-kind schema.

---

## Target 4 — Backstage catalog-info.yaml (typed component metadata)

- **URLs:** https://backstage.io/docs/features/software-catalog/descriptor-format/ ·
  .../well-known-annotations/
- **Source type:** vendor docs (primary)
- **What's typed:** entity envelope is schema-validated — `apiVersion`, `kind`
  (Component/API/System/Resource/…), `metadata` (name/annotations), `spec`
  (`type`, `lifecycle`, `owner`, dependency edges like `dependsOn`, `providesApis`,
  `consumesApis`). Annotation key grammar is strict (lowercase-domain prefix ≤ 253
  chars, name `[a-zA-Z0-9]`+`[-_.]` ≤ 63).
- **The doc link is a POINTER, not typed content:** `backstage.io/techdocs-ref`
  "informs where TechDocs source content is stored"; value `dir:.` or a location
  URL. TechDocs itself is MkDocs markdown — **free prose behind a typed pointer.**
- **Relevance:** Backstage types the _metadata graph around_ a component (incl.
  `dependsOn` edges — the closest mainstream analogue to declared depends-edges) and
  links to docs; it does NOT type the doc _content_. This is the "typed catalog +
  untyped docs" split the design collapses. The `dependsOn`/`providesApis` fields are
  worth stealing as edge vocabulary; there is **no CI cross-check** of `dependsOn`
  against the real import graph in stock Backstage.

---

## Target 5 — Agent-Friendly Documentation Spec (agentdocsspec.com) — NOT a schema

- **URL:** https://agentdocsspec.com/ · CLI `afdocs`
- **Source type:** spec site
- **Verdict:** a **quality checklist**, not a content schema. "23 checks across 7
  categories" (Content Discoverability, Markdown Availability, …) assessing whether a
  docs _site_ serves agents. Recommends `llms.txt` "under 50K characters" and serving
  `.md` URLs. Explicitly **no typed fields/blocks, no schema, no per-harness
  generation.** Included to disambiguate from Charter's landing page (same "agent
  docs spec" phrasing, unrelated project).

---

## Target 6 — Adjacent typed-doc formats (breadth, lightly sourced)

- **MADR (Markdown ADR)** — https://adr.github.io/madr/ · https://github.com/adr/madr
  (2.1k★). YAML front matter + **named sections as convention**: context, decision
  drivers, considered options, decision outcome (nested consequences/confirmation),
  per-option pros/cons. Multiple template variants (full/minimal/bare). **Typing is
  by template, not enforced schema** — it's structured prose an author fills in; no
  validator rejects a missing "decision drivers." The closest doc-genre to typed
  _fields_ but enforcement is social, not machine. Good field vocabulary to mine.
- **OpenAPI / AsyncAPI** — the mature example of **fully typed, machine-validated API
  docs**: JSON-Schema-backed, linted (Spectral), rendered to many outputs. Proof that
  "typed doc → many renderings + CI validation" is a solved pattern _for API surface_;
  nobody has ported the rigor to _repo/agent_ docs. (Well-known; not re-fetched.)
- **CUE / Dhall** — typed config languages. CUE has first-class JSON Schema import/
  export and "native support for validating YAML, JSON…"; Dhall "is typed and rejects
  some configurations JSON would accept." These are the _engine_ one would validate a
  typed doc graph with, not doc formats themselves. Relevant as the validation
  substrate (agraph in Rust would reimplement a slice of this).
- **JSON-schema-validated frontmatter** — the dominant lightweight pattern (AGENTS.md
  v1.1 `description`/`tags` frontmatter; docs-site frontmatter linters). Types the
  _envelope_, never the body. Consistent with everything above.

---

## Synthesis — where the genuine gap is

Plotting all systems on two axes — **(a) how deeply is doc CONTENT typed** and
**(b) is it a GRAPH with CI-asserted authored edges** — every existing system sits in
one quadrant and the design targets the empty one:

- **Typed envelope, prose body:** Backstage, MADR, JSON-schema frontmatter, AGENTS.md
  v1.1. (Types metadata/links, not content-kinds.)
- **Typed API content, fully validated:** OpenAPI/AsyncAPI. (Solved — but API-only,
  not repo/agent docs, not a repo graph.)
- **Structurally typed + budgeted + per-harness compiled:** **Charter/ADF.** (Closest
  overall. Ships token budgets + metric ceilings + multi-harness compile as a CI bar.
  Missing: semantic node kinds, graph, edges, declared-vs-extracted cross-check.)
- **Extracted code graph, edge-provenance, drift hooks, MCP query:** **Graphify /
  codegraph.** (Closest on the graph half. Missing: authored typed doc content, C4
  altitudes, per-harness budgets, engine-free AGENTS.md fallback, authored-vs-extracted
  assertion — the graph _is_ the extraction, nothing is declared to check.)

**Nobody occupies the intersection the design claims:** an authored, semantically-typed
knowledge graph (commands/invariants/hazards/edges as typed fields, C4 altitudes as
node kinds) that (1) is unfolded by typed query, (2) **self-asserts declared
depends-edges against the extracted import graph in BOTH directions** in CI, (3)
enforces per-harness token budgets + anchor liveness + freshness watermarks, and
(4) materializes plain AGENTS.md as an engine-free portability fallback.

The two halves each exist in a shipped tool; **their union — authored typing crossed
with extraction reconciliation — is unclaimed.** The strongest "this already exists"
challenges to pre-empt: ADF (owns compile-to-harness + budgets + the CI-ceiling
pattern) and Graphify (owns the extracted graph + EXTRACTED/INFERRED provenance +
drift hooks). The design should cite both and position its novelty precisely as
**authored-intent-vs-extraction reconciliation over a semantically-typed graph**, not
as "the first typed agent docs" (ADF got there on structure) nor "the first agent
knowledge graph" (Graphify/codegraph got there on extraction).

# W3 — Structured comment grammars mined as data

Prior-art sweep for `agraph`: does any existing convention/tool treat specially-formatted
code comments as machine-readable **claims/identity** (this-is-landmark-X,
this-owns-invariant-Y) rather than description, round-trip them into an agent-facing
doc/graph, and/or **validate** the anchors in CI?

Scope: comment-as-data only. Method budget met (5 searches, 5 full fetches, one chain).

---

## 1. Anchor comments — the `AIDEV-NOTE` convention (2025 practitioner pattern)

**Origin (confirmed primary source):** Diwank Singh Tomer, "Field Notes From Shipping
Real Code With Claude," https://diwank.space/field-notes-from-shipping-real-code-with-claude
— published **2025-06-07** (blog post; also mirrored on LessWrong and #1 on Hacker News
`item?id=44211417`). This is the artifact everyone else cites; it is _not_ from Harper
Reed (his 2025-05-08 "Basic Claude Code" post has no anchor-comment section — checked, absent).

The convention (verbatim, quotes <15 words):

- Prefixes: "`AIDEV-NOTE:`, `AIDEV-TODO:`, or `AIDEV-QUESTION:` (all-caps prefix)".
- "Before scanning files, always first try to locate existing anchors `AIDEV-*`".
- "Update relevant anchors when modifying associated code."
- "Do not remove `AIDEV-NOTE`s without explicit human instruction."
- Add when code is "too complex, or very important, or confusing, or could have a bug."

**Character of the grammar:** DESCRIPTIVE / guidance breadcrumbs ("street signs within
the codebase"), aimed at both AI and humans. It marks _attention_ ("look here, this is
subtle"), **not identity or a typed claim**. There is:

- **No CI validation** of anchors (liveness, staleness) — confirmed absent in source.
- **No round-trip** into a doc/graph — grep at read-time is the entire mechanism.
- No linkage to git blame or issue trackers.

**Adoption / tooling built on it (all 2025):**

- `Filip-Podstavec/claude-leverage` — "AI-first" repo scaffolding: root+per-dir AGENTS.md,
  "AIDEV anchors throughout," ADRs, GLOSSARY.md, `architecture.yml`, and a
  `.claude-leverage-context-map.json` manifest. Notable: it pairs anchors with a JSON
  context map — the nearest thing found to a manifest-over-anchors, but it is a static
  hand-maintained map, not a compiled/validated graph.
- `ovidiuiliescu/AiComments` — "AI Comments" wrapper format: comments in a recognizable
  wrapper "easy for agents/tools to detect," carrying "intent, constraints, and invariants."
  Closer in spirit to typed claims but still descriptive; no compiler/validator surfaced.
- `wasabeef/agent-note` (DEV post 2024-05-18) — ADJACENT/boundary: stores the _why_ of
  AI edits in **git notes** (`refs/notes/agentnote`), not inline comments. Has an
  `agent-note why` line-blame CLI and a hidden PR "reviewer-context" comment for AI review
  tools. No inline anchor grammar; no CI validation of claims. Interesting as a
  round-trip-into-agent-facing-artifact precedent, but the artifact is git metadata, not a graph.

---

## 2. Annotation-to-spec compilers (comment → queryable artifact)

**swaggo/swag** (https://github.com/swaggo/swag) — compiles godoc-style "magic comments"
(`@Summary`, `@Description`, `@Tags`, `@Param`, `@Router`…) above Go handlers into an
OpenAPI/Swagger spec (`docs/docs.go`) via `swag init`. This IS "comment-as-data compiled
into a queryable artifact." But the extracted data is a **description of an API surface**,
not an invariant/claim, and the artifact is a spec file, not a knowledge graph. Widely
adopted (claimed ~75% of Go API devs, 2025 — treat figure as marketing).
Same family: rustdoc, JSDoc, doxygen (below) — all extract tagged comments into a doc site.

**doxygen** custom-tag extraction (https://www.doxygen.nl/manual/custcmd.html) — the most
graph-like of the doc generators:

- `ALIASES` config defines custom `\tag{arg}` commands.
- `\xrefitem` **aggregates every occurrence of a custom tag into a single cross-referenced
  list page** — i.e. it round-trips scattered comment tags into one queryable artifact
  (the `\todo`/`\bug`/`\test` lists work this way). Closest classic precedent to
  "collect anchors → materialize an index."
- Built-in **typed-claim tags** exist: `\invariant`, `\pre`, `\post`. These are the
  clearest prior art for "comment declaring a typed invariant" — but they render to prose
  in docs; nothing validates them against code, and they attach to a symbol, not a landmark identity.

---

## 3. Magic-comment linters / pragmas (typed directives, validated by tooling)

- ESLint directive comments (`// eslint-disable-next-line rule`), coverage pragmas
  (`/* istanbul ignore next */`, `# pragma: no cover`), type pragmas (`# type: ignore`).
  These ARE machine-read typed comments that change tool behavior — but each is a _local
  suppression directive_, not a claim about the code's role, and they are consumed inline,
  never aggregated into a graph.

## 4. TODO-comment miners — and the one that VALIDATES in CI

- **`presmihaylov/todocheck`** (https://github.com/presmihaylov/todocheck) — **the strongest
  hit for CI-validated comment claims.** "Static code analyser for annotated TODO comments."
  A TODO annotated with an issue id is a machine-readable _claim that the issue is open_;
  todocheck checks each against the tracker (GitHub/GitLab/Jira, `.todocheck.yaml`) and
  **fails CI** on: issue closed ("ERROR: Issue is closed"), issue nonexistent, or malformed/
  unannotated TODO. Purpose: "no half-baked issue closed with pending TODOs." Issue #57
  even proposes a bot to _reopen_ an issue when live TODOs still reference it — i.e.
  bidirectional consistency between comment and external state. This is exactly the
  "anchor liveness / declared-vs-actual, both directions" pattern `agraph` proposes, but
  scoped to one claim type (issue-open) against an external tracker, not repo-internal graph edges.
- Siblings: `Softwire/todo-checker` (TODOs ↔ Jira), Track-TODO GitHub Action, checkstyle
  `TodoComment`, "fail CI on TODO/FIXME." **`leasot`** — pure miner/reporter (extracts
  TODO/FIXME into a report); no validation of a claim.

---

## Verdict — closest prior art to a claims-not-descriptions comment grammar

No existing convention combines all three `agraph` properties (typed **identity/claim**
anchors + **graph** round-trip + **CI self-assertion**). The pieces exist separately:

| Property `agraph` wants                                           | Closest prior art                                                                                          | Gap                                                                                                            |
| ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Comment marks **identity** (this-is-landmark-X, owns-invariant-Y) | doxygen `\invariant`/`\pre`/`\post` (typed claim on a symbol); AiComments "invariants"                     | Attach to a symbol/prose, not a named landmark node; no query surface                                          |
| Comment carries a **validated claim**                             | **todocheck** (fails CI on closed/absent issue), todocheck-bot bidirectional reopen                        | Single claim type (issue-open) vs _external_ tracker; not repo-internal depends/import edges or freshness      |
| Anchors **round-trip into a queryable artifact/graph**            | doxygen `\xrefitem` list pages; swaggo → OpenAPI spec; claude-leverage `.claude-leverage-context-map.json` | Artifact is a doc/spec/static JSON, not a typed graph; no agent unfold/query semantics; map is hand-maintained |
| Anchors serve an **AI-agent** context surface                     | `AIDEV-NOTE` (grep-at-read), AGENTS.md ecosystem                                                           | Purely descriptive breadcrumbs; no liveness/staleness/token-budget CI; no compiled graph                       |

**Genuine gap for `agraph`:** the _claims-not-description_ framing plus **bidirectional
CI self-assertion of comment anchors against the extracted code graph** (declared edges vs
import graph, anchor liveness, freshness watermarks, token budgets) is unoccupied.
The nearest single precedent is **todocheck** — it proves the "comment as a claim the CI
must keep true, both directions" pattern is real and shipped — but only for issue-tracker
liveness, against an external system, with no graph and no landmark/identity semantics.
doxygen `\xrefitem` proves "aggregate tagged comments → one artifact" is old and solved;
swaggo proves "compile comments → queryable spec" is mainstream. None marks a node's
_identity/ownership_ or unfolds a typed graph for an agent.

## Why any thinness

Not thin on the practitioner + tooling axes (primary sources reached for AIDEV-NOTE origin,
todocheck, swaggo, doxygen). Lighter on academic formalisms (e.g. literate-programming /
"invariant annotation" research, JML/Javadoc-`@invariant` design-by-contract lineage) —
those are older prior art for _typed invariants in comments_ worth one more pass if the
design leans on the invariant-verification angle rather than the agent-graph angle.

## Leads (unfetched / worth a follow-up pass)

- JML (Java Modeling Language) / Design-by-Contract `@invariant @requires @ensures` — the
  formal-methods ancestor of typed-claim comments; verified by tools (ESC/Java, OpenJML).
- `Filip-Podstavec/claude-leverage` `.claude-leverage-context-map.json` — inspect schema;
  nearest manifest-over-anchors artifact.
- todocheck issue #57 (bot reopening issues from live TODOs) — bidirectional-assertion precedent.
- doxygen `\xrefitem` mechanics — the aggregate-tags-into-a-page pattern in detail.
- AGENTS.md ecosystem (agents.md standard) — the "materialize plain AGENTS.md fallback" baseline.
- `ovidiuiliescu/AiComments` wrapper format — the one convention explicitly claiming to carry "invariants."

## Sources (URL · date · type)

- diwank.space/field-notes-from-shipping-real-code-with-claude · 2025-06-07 · blog (PRIMARY origin of AIDEV-NOTE)
- news.ycombinator.com/item?id=44211417 · 2025 · HN discussion
- github.com/Filip-Podstavec/claude-leverage · 2025 · repo/README
- github.com/ovidiuiliescu/AiComments · 2025 · repo/README
- dev.to/wasabeef/introducing-agent-note-... · 2024-05-18 · blog (git-notes approach)
- github.com/swaggo/swag · repo/README · annotation→OpenAPI compiler
- github.com/presmihaylov/todocheck (+ issue #57) · repo · CI validator of TODO claims
- doxygen.nl/manual/custcmd.html · docs · ALIASES / \xrefitem / \invariant
- doc.rust-lang.org/rustdoc/lints.html + reference/attributes · docs · rustdoc custom tags
- harper.blog/2025/05/08/basic-claude-code · 2025-05-08 · blog (checked: NO anchor section)

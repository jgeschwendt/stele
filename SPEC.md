# stele — specification

Draft 0.9 · 2026-07-21 · status: refined — gigarefine converged at cap (5 passes, curve 33→21→11→5→3-trivial); 0.8 adds the `.steleignore` scan-exclusion decision from v1 self-hosting; 0.9 adds undercover mode — a private single-user graph for shared repos whose collaborators never see a stele artifact
Evidence citations `[Cn]` resolve to [`research/claims.md`](./research/claims.md); prior-art citations name the finding files in [`research/findings/`](./research/findings/).

## 0. Positioning

**stele is authored-intent-vs-extraction reconciliation over a semantically-typed knowledge graph, degrading to plain AGENTS.md.**

A Rust single-binary (`stele`) compiles a typed graph from three authored sources — the file tree, comment anchors, and typed blocks in AGENTS.md files — reconciles it against a derived graph extracted from the code, asserts the two against each other in CI, serves the graph to agents as queries, and materializes standard-compliant AGENTS.md files so every harness works with zero engine.

What it is **not** (each rejection is evidence- or market-bound):

- Not an embedding index — agentic search beats retrieval for code; near-random on localization [C5].
- Not a prose-overview generator — overviews are inert even human-written; generated restatement is net-negative [C7, C17].
- Not another derived-AST-graph-over-MCP server — that category is commodity (~18 tools; codebase-memory-mcp is the reference twin) [C23].
- Not a diagram tool — the graph is a contract and a query surface, not a picture [C14, C24].

## 1. Design constraints (binding, evidence-cited)

1. **The root is a router and a claims-carrier, never an overview.** Only non-derivable content: commands, invariants, hazards, typed pointers. The test for every authored token: _could the agent derive this by reading the repo?_ If yes, it is forbidden at the root [C7, C17].
2. **Commands are the highest-value payload.** Specific tooling/build/test instructions are the only content proven to change agent behavior (mentioned tool used 1.6–2.5×/instance vs ~0) [C17].
3. **Navigation favors recall; prose favors omission.** Missing the load-bearing edge is the dominant failure; redundant edges are tolerated. The inverse holds for prose [C6, C8].
4. **Staleness is net-negative on the measured evidence (harm measured on humans; agent transfer plausible, not yet measured), so freshness is enforced, not requested** [C6, C10]. Every claim is anchored and watermarked; CI fails on drift.
5. **Portability = explicit materialization.** Harness loading semantics diverge (Claude lazy subdirs; Codex static root→cwd chain, default 32 KiB limit) [C3, C4]. The committed AGENTS.md files are complete without the engine; the engine is progressive enhancement.
6. **One canonical home per fact.** The graph compiles from sources; every rendered artifact is a marked, checked projection. Duplication is drift [C13, house rule].
7. **Load-bearing content sits at the top of any materialized file** (position-in-context effects — the U-shape reported by Liu et al. 2024; note Chroma 2025 found no U-shape in its NIAH runs, so treat the front-load as the conservative bet, not a settled magnitude) [C15, C9].
8. **Depth follows the codebase's real structure.** C4's own guidance: most teams need only the first two levels; the schema must not force more [C14].

## 2. The model

### 2.1 Node kinds — C4 altitudes as types

| kind        | C4 level | one per…                     | typical id          |
| ----------- | -------- | ---------------------------- | ------------------- |
| `system`    | L1       | repo (exactly one, the root) | `/`                 |
| `container` | L2       | deployable/runnable unit     | `apps/web`          |
| `component` | L3       | module behind an interface   | `apps/web/lib/core` |

L4 (code) is deliberately **not a node kind** — the territory itself is the leaf, reached through anchors; code-level description is generated on demand, never authored [C14].

`component` nodes may nest. Node **ids are path-derived by default** (the directory that declares them), keeping the doc tree isomorphic to the code tree by construction [C14]. An explicit `id:` override exists for the rare non-path-shaped node.

Auxiliary node kinds: `adr` (one per decision record, immutable, numbered) and `anchor` (compiled from comment anchors, never declared in AGENTS.md).

**Id normalization** (applied at build before any comparison, dedup, or lock write): ids are repo-root-relative POSIX paths — strip a single leading `./`, strip any trailing `/`, collapse repeated `/`, reject `..` and OS-absolute paths, and normalize `\` → `/`. The system node's id is the single character `/` (the sole non-relative id). Explicit `id:` overrides are normalized identically. Two nodes normalizing to the same id → duplicate-id error (exit 2), naming both declaring files.

### 2.2 Node schema

Declared fields (authored; all optional except `kind`):

```yaml
kind: component # system | container | component
id: apps/web/lib/core # default: declaring directory
purpose: >- # ≤200 chars — index scent, not prose [C19]
  Memory pipeline core: sweep, judge, dissolve queue.
commands: # constraint 2 — the proven payload
  test: MIX_ENV=test mix test apps/web/lib/core
  check: mix precommit
invariants: # claims with anchors; see 2.4
  - claim: memories are append-only; supersede, never mutate
    anchor: lm:memory-supersede # comment anchor (2.5) or path#symbol
    enforced_by: test/core/supersede_test.exs # optional but preferred (guards > prose)
hazards: # non-derivable warnings, each anchored
  - claim: sweep is idempotent ONLY per-hour; double-run duplicates extraction
    anchor: lm:sweep-entry
edges:
  depends: [apps/web/lib/store, packages/shared] # the signature — checked both ways (4.2)
  decided_by: [adr/0007]
  allow: # §4.2 structural escape hatches
    - edge: apps/web/lib/billing # tolerated cross-boundary target (a node id)
      reason: "Oban job name resolved at runtime" # REQUIRED; empty/absent → exit 2
budget: 1500 # max tokens this node may contribute to any materialized file
```

Compiled-only data (derived, never authored): on the node, `extracted.imports` (extracted dependency edges) and `contains` (from the tree). Resolved anchors are NOT a per-node array — landmarks serialize to the lock's top-level `landmarks{}` map (§3.2), and each claim carries its own `resolved` (file:line) and `verified {sha, digest}` (§4.5).

**The schema is the curation bar.** There is no free-prose body field. Anything an author wants an agent to know must fit a typed field or it does not enter the graph — this is the structural enforcement of constraint 1.

### 2.3 Edge vocabulary (deliberately minimal)

| edge         | authored?         | meaning                            | checked against                      |
| ------------ | ----------------- | ---------------------------------- | ------------------------------------ |
| `contains`   | derived from tree | structural parent/child            | filesystem                           |
| `depends`    | **authored**      | allowed dependency (the signature) | extracted imports, both directions   |
| `imports`    | derived           | actual dependency                  | —                                    |
| `decided_by` | authored          | rationale pointer                  | ADR file exists, status ≠ superseded |
| `anchors`    | derived           | node → code landmark               | slug-match cardinality + liveness    |

New edge types require a spec revision. A small closed vocabulary is what keeps `check` decidable and the query surface learnable.

### 2.4 Claims

`invariants` and `hazards` entries are **claims**: `{claim, anchor, enforced_by?, verified}`. Rules:

- Every claim carries an anchor (a comment anchor id or `path#symbol`). An unanchored claim is rejected at build — provenance or it doesn't compile (hearsay gate).
- `verified` is stamped by `stele build` as `{sha, digest}` — the commit SHA at which the anchor last resolved and the tree-sitter structural digest of its bound definition (§4.5); `digest` is `null` **only where the anchored file's language has no bundled parser** — those claims fall to the §4.5 churn-count fallback. A prose-only claim in a parseable language DOES carry a digest — the digest is precisely what stales it (§4.5, EXAMPLE 8.4); it is `enforced_by`-backed claims that are exempt from digest-staling, not prose-only ones.
- A claim is addressed **`<node-id>/<slug>`**, where `<slug>` is derived from the claim's anchor: for an `lm:<slug>` anchor, the landmark slug verbatim; for a `<path>#<symbol>` anchor, `<symbol>` lowercased with each maximal run of non-`[a-z0-9]` characters collapsed to a single `-` and leading/trailing `-` stripped (so `refund.ex#changeset` → `changeset`, `money.ts#MoneyType` → `moneytype`). Two claims in one node whose derived slugs are equal → exit 2 (naming both). This derived slug is the claim's lock `id` (§3.2) and the form the `stele:claim` comment (§2.5) and `stele blame` take. The node-id may be abbreviated to its final path segment when unambiguous across the graph, so `apps/web/lib/billing/refund-cap` and `billing/refund-cap` name the same claim.
- When a claim is mechanizable, `enforced_by` names the test/lint that enforces it; `check` verifies the referenced artifact exists. Prose-only claims are permitted but are the flagged minority — the direction of travel is gotcha → guard.

**Two anchor namespaces**, both valid in `anchor:`: (1) `lm:<slug>` — a landmark; resolves to the unique `// stele:landmark <slug>` comment. The `lm:` prefix is fixed; `<slug>` matches the comment slug verbatim; cardinality-1 is enforced in §4.1. (2) `<path>#<symbol>` — a direct binding; `<path>` is a repo-root-relative file, `<symbol>` resolves via tree-sitter to a named definition (function/module/class/etc.) in that file; in markdown (parser-less) it resolves against ATX heading slugs — exact slug match or slug-prefix at a hyphen boundary (`c1` matches `## C1: …`, never `## C10: …`) — with `digest: null` (§4.5 churn fallback). Cardinality MUST be 1 (0 → unresolved, >1 → ambiguous; both exit 1). No comment required — survives edits inside the symbol but breaks on rename/move (the EXAMPLE §4 trade-off).

**Scan scope** for referential + cardinality checks (§4.1): VCS-tracked files only (respects `.gitignore`), excluding `.stele/` and any path matched by a committed root **`.steleignore`** (gitignore syntax). A `.steleignore`d path is invisible to ALL build/check source scanning alike — node discovery (its AGENTS.md declares nothing, §3.1), the anchor scan, and import extraction — so vendored fixture trees and docs that quote anchor syntax cannot pollute the graph. `.steleignore` is a source artifact, not configuration: it is an input to `build` and therefore to the lock, unlike `.stele/config.toml` (§3.4), which stays check/emit-time only. AGENTS.md files ARE scanned so `stele:claim` back-references resolve; `stele:claim` and `stele:landmark` are distinct tokens and never collide. `resolved` (`file:line`) is recomputed every build and stored in the lock. In undercover mode (§3.5) node discovery instead walks the overlay under `<home>/.stele/tree/` (a plain filesystem walk, never `git ls-files` and never `.steleignore`-filtered); every other scan named here — anchors, extraction, exhaustiveness, freshness — stays VCS-tracked-only.

### 2.5 Comment anchor grammar

Language-native comments, `stele:` prefix, **identity and binding only — never description** (descriptive anchors restate code and rot [C17, C25]):

```
// stele:landmark <slug>            — declares a landmark; slug-match cardinality MUST be 1
// stele:claim <node-id>/<slug>          — binds this region to a declared claim
```

`<slug>` lexeme: `[a-z0-9]+(-[a-z0-9]+)*` (lowercase alphanumerics, single hyphens, no leading/trailing/doubled hyphen), terminated by the first whitespace or end-of-line after `stele:landmark `. Cardinality (§4.1) counts comments whose slug is byte-for-byte EQUAL to the queried slug — not a word-bounded substring: `stele:landmark refund-cap` and `stele:landmark refund-cap-2` are distinct landmarks and never inflate each other's count (hyphens are word-boundary characters, so a substring rule would false-match). A malformed slug (illegal char, empty) → exit 2.

"Language-native comments" is load-bearing for the scan: in parser-backed languages only tokens inside COMMENT nodes count (a string literal is not a declaration); in markdown the native comment is the HTML comment (`<!-- -->`) — an anchor token inside a fenced code block or prose is a quotation, not a declaration. Only parser-less non-markdown files fall back to a lexical line-scan.

Two forms, no more. `landmark` gives claims and queries a stable, move-proof address (renames don't break anchors; the registry does — and CI catches it). `claim` closes the loop from a declared invariant/hazard to the code region it governs. Prior art: todocheck proves comment-claims-validated-in-CI ships; AIDEV-NOTE proves agents grep for anchor prefixes; nothing combines them [C25].

### 2.6 ADRs

Standard Nygard/MADR files under `adr/`, numbered, immutable, superseded-not-deleted [C14]. stele does not define its own decision format; it indexes ADRs as nodes and validates `decided_by` targets.

## 3. Sources and artifacts

### 3.1 AGENTS.md — simultaneously source and rendering

Each node directory holds one `AGENTS.md` containing:

1. **The authored block** — the node declaration (§2.2) in exactly one fenced ` ```stele ` block, which MUST be the first fenced code block in the file (prose and the `# <name>` heading may precede it; no other fenced block may). It is the source of truth; humans and agents edit it in place. An AGENTS.md with **no** `stele` block is valid and simply declares no node — the plain-AGENTS.md degradation of §0; its directory contributes no node and its files fall to the nearest ancestor node's territory (§4.2). **Two or more `stele` blocks in one file → exit 2** (naming the file and both block line ranges). A ` ```stele ` block in any file not named `AGENTS.md` is ignored by `build`, as is any AGENTS.md under a `.steleignore`d path (§2.4).
2. **The generated region** — exactly ONE marker-fenced region per file, opened by `<!-- stele:begin router -->` and closed by `<!-- stele:end -->`, rendered by `emit`. The token immediately after `begin ` is the region name (`router` is the only name v1 emits); any text between the name and `-->` is free annotation the parser ignores (so the root's `router · generated … · do not hand-edit` and an interior node's bare `router` are equivalent). All bytes strictly between the markers are engine-owned; the markers and everything outside them are preserved verbatim. Regions do not nest. A `begin` with no matching `end`, an `end` with no `begin`, two `begin router` markers, or nesting → exit 2 (naming the file). A node's AGENTS.md that has a `stele` block but no generated region → `emit` exits 2 (`run stele init` to scaffold it); `emit` only rewrites between existing markers and never creates the region itself. Contents vary by node kind: the root renders the hazard banner (top-N active hazards across the graph), the router/index of child + depended nodes (id, kind, purpose, unfold pointer), the transpose-index pointers (§6.1), and the engine instructions; interior nodes render their resolved-anchor table (anchor → file:line) plus a child router where they have children. Identity and commands are never generated — they live in the authored block (§6 items 1–2). `emit --check` fails CI when a generated region diverges from the graph. Rendering is deterministic (stable node/edge ordering): the same graph always emits byte-identical regions — the property `emit --check`'s byte-diff relies on, and the mitigation that keeps generated-region merge conflicts rare and mechanical.
3. **Optional free prose** — allowed _outside_ both regions, counted against `budget`, and lint-flagged when it grows (the evidence says it shouldn't exist; the spec doesn't ban what a team insists on — it prices it).

One file, edit-in-place, git-diffable, readable by every harness with zero engine. No shadow source tree (rejects the Charter/ADF split where the readable file is a pure build artifact — that breaks edit-in-place and the no-engine story) [C26].

### 3.2 The lockfile

`.stele/graph.lock` — the compiled graph (committed in normal mode; a private on-disk file at the graph home in undercover mode, §3.5); the engine's sole read path (`query`/`serve`/`emit` load it and never re-parse the repo). CI's `check` byte-compares a fresh rebuild against it (§5.1); mismatch → exit 2 means the sources changed without a rebuild, or the lock was hand-edited. The byte-compare and its exit-2 mechanics are identical in both modes.

**Canonical serialization (the byte-diff contract).** UTF-8, LF newlines, one trailing LF, no trailing whitespace, 2-space pretty-print (diffable-in-PR, the deliberate choice over minified JCS). Object keys sorted by Unicode scalar value. Array order: `depends`/`decided_by`/`allow` in authored order; `imports`/`contains`/`claims`/`landmarks` sorted by id/slug. All numeric fields are integers (no floats emitted). Strings use minimal escaping (`\" \\ \b \f \n \r \t` + mandatory control `\u00xx`); every other character, non-ASCII included, is emitted raw — never `\u`-escaped.

**Top-level:** `{ "version": 1, "nodes": {"<id>": <node>}, "adrs": {"<id>": {"number":7,"status":"accepted","path":"adr/0007-integer-cents.md"}}, "landmarks": {"<slug>": {"file":"…","line":18,"node":"<id>"}} }`. Unknown `version` → reject (exit 2), never best-effort parse.

**Node object** (every field present; empty containers `[]`/`{}`, absent scalars `null`): `kind`, `id`, `purpose`, `commands{}`, `budget`, `declared{depends[],decided_by[],allow[]}`, `extracted{imports[]}`, `contains[]`, `claims[]`.

**Claim object:** `{id, kind: invariant|hazard, text, anchor, resolved:"file:line", enforced_by, verified}` — a `kind` discriminator is REQUIRED so invariants and hazards round-trip.

The JSON shown throughout this spec and EXAMPLE.md is in logical reading order for legibility; the on-disk lock sorts every object's keys by Unicode scalar value as specified above — e.g. the top-level object serializes as `adrs, landmarks, nodes, version`, and a node object as `budget, claims, commands, contains, declared, extracted, id, kind, purpose`.

v1 does not specify how the lock is surfaced in PR review; teams may `.gitattributes`-collapse it and rely on `check` in CI, accepting that graph changes are then not diff-visible to reviewers (unspecified in v1).

### 3.3 CLAUDE.md shim and per-harness output

- `CLAUDE.md` = `@AGENTS.md` (one line; Claude Code does not read AGENTS.md natively as of 2026-07-19 docs; re-verify — negative claim) [C2].
- Optional: `emit --claude-rules` renders path-scoped `.claude/rules/*.md` from node claims (native lazy loading) [C3].
- `emit --check` enforces per-harness budget profiles (§4.4).

### 3.4 Configuration

Check-time settings live in `.stele/config.toml` (committed; absent → all defaults, which is why EXAMPLE ships none). It is NOT an input to `build`/the lock (it tunes `check`/`emit`, not the graph). Keys, all optional: `exhaustiveness.depth` (int, default 1), `exhaustiveness.exclude` (glob list, default `["node_modules","_build","deps","target"]`), `budget.claude_root` (int tokens, default 2000), `budget.codex_cap` (int bytes, default 32768), `freshness.churn_threshold` (int) and `freshness.enforced_leash` (int) with per-node overrides under `[freshness.node."<id>"]`, and `check.disable` (list of assertion-class names — the "independently toggleable" knob; `--only <class>` remains the run-exactly-one flag). Unknown keys → exit 2.

### 3.5 Undercover mode

A single operator runs stele privately on a shared repo — a work, upstream, or open-source checkout whose collaborators must never see a stele artifact. Everything the engine authors or renders (node source files, `.stele/`, the harness shim) stays out of version control, analogous to the `CLAUDE.local.md` convention. Undercover is reached only by an explicit marker; its absence leaves every §2–§7 behavior byte-for-byte identical to normal mode.

**Graph home.** The home is the parent of `git rev-parse --git-common-dir`, canonicalized. In a normal checkout this is the repo root — today's `.stele/` location (§3.2), now untracked. In a bare-root worktree layout (a bare `.git` common dir with sibling working trees) it is the shared parent outside every work tree, so one graph serves every worktree and survives worktree churn. Canonicalization resolves symlinks on both the home and the invoking work tree before any path comparison, so a `.stele` symlink placed inside a work tree that points at the shared home (a future worktree provisioner may create one) changes nothing.

**Mode detection.** Presence of the marker file `.stele/undercover` at the home selects undercover mode; its absence is normal mode, byte-for-byte. The marker is a new artifact class: a deliberately-uncommitted **source** artifact. Like `.steleignore` (§2.4) it changes `build` inputs — it moves where node sources are read from, and so it is an input to the graph and the lock; unlike `.steleignore`, which is a committed source artifact, the marker is never committed. It is categorically distinct from `.stele/config.toml` (§3.4), which never feeds `build` at all. The uncommitted-yet-source combination is unique to the marker, and it is exactly what keeps the mode invisible to collaborators.

**Overlay node sources.** Node sources move off the tracked tree into an overlay under `<home>/.stele/tree/`, mirroring tree paths: `<home>/.stele/tree/<dirpath>/AGENTS.md` declares the node whose directory is `<dirpath>`. The root (system) node is `.stele/tree/AGENTS.md`; the territory (§4.2) of `.stele/tree/apps/api/AGENTS.md` is `apps/api`. Node discovery (§2.4) walks this overlay instead of tracked `AGENTS.md` files — a plain filesystem walk, never `git ls-files` and never `.steleignore`-filtered, because these are authored sources, not scanned code. The code scan is unchanged: anchors (§2.5), import extraction, exhaustiveness (§4.3), and freshness (§4.5) still run over the VCS-tracked work-tree files, and `.steleignore` still filters that code roster.

**The one materialized file.** Undercover renders exactly one file into the work tree: `CLAUDE.local.md` at the invoking work-tree root, containing a single relative `@`-import of the overlay root node file (normal checkout: `@.stele/tree/AGENTS.md`; a worktree one level below the home: `@../.stele/tree/AGENTS.md`). Claude Code auto-loads `CLAUDE.local.md` alongside `CLAUDE.md`, and its docs recommend gitignoring it (verified against live docs 2026-07-21) — the shim is the private analogue of the §3.3 `CLAUDE.md` = `@AGENTS.md` line. To keep the work tree clean, a marker-fenced managed block in `<common-dir>/info/exclude` — the lines `# stele:begin undercover` … `# stele:end undercover`, rewritten in place idempotently and never touching a line outside the fence — unconditionally carries `/CLAUDE.local.md` and `/.stele/`. Excess entries are harmless, and the block lives in the shared common dir, so one block covers every worktree.

**Mutual exclusion.** An undercover marker coexisting with a tracked `stele`-block `AGENTS.md` or a tracked `.stele/` in the work tree → exit 2: a repo already carrying a shared, committed graph cannot also run undercover. Mixing the two is a v2 concern.

## 4. The assertion suite — `stele check`

Six assertion classes; each is independently toggleable, all run in CI. Exit non-zero on any failure. This suite is the unclaimed core: as of 2026-07 nothing ships bidirectional declared-vs-extracted reconciliation coupled to agent docs — a negative-capability claim, re-verify (the category is moving) [C23, C24, C26].

### 4.1 Referential

Every anchor resolves (file exists, symbol found via tree-sitter); every `decided_by` names an existing, non-superseded ADR; every `enforced_by` artifact exists; every landmark has slug-match cardinality exactly 1 (comments whose slug is byte-for-byte equal, not a word-bounded substring; §2.5); every `stele:claim <node-id>/<slug>` comment resolves to a declared claim (a dangling `stele:claim`, or one naming a non-existent node/slug, fails referential).

### 4.2 Structural — the signature, both directions

For every node (a node with **no** declared `depends` permits no cross-boundary import — empty means "depends on nothing", not "unchecked"; the Vestigial direction below applies only where an edge is actually declared):

- **Violation:** an extracted import crossing node boundaries with no covering `depends` edge → _code broke the signature_. (Forward direction — the mature half; ArchUnit-family semantics.)
- **Vestigial:** a declared `depends` edge with no extracted import backing it → _the doc lied, or the dependency died_. (Reverse direction — shipped nowhere; dependency-cruiser's `required` rule is the only near-primitive [C24].)

**Territory attribution, non-inheriting.** A node's _territory_ is its declared directory and all descendants MINUS the territory of every nested node. Each extracted import is attributed to the node whose territory contains the importing file, and checked against THAT node's own declared `depends` only. A child never inherits a parent's `depends`, and a parent's `depends` never licenses an import that originates inside a child's territory. Rationale: inheritance would silently widen a child's allowed surface and defeat the signature. (Contrast §5.1 `invariants --touching`, where invariant _exposure_ DOES surface upward from ancestors — exposure is additive and safe to over-report; dependency permission must be explicit and narrow.)

Escape hatches, both required for real adoption: inline `allow: {edge, reason}` entries (dynamic dispatch, DI, FFI — the honest limits of static extraction), and `stele check --freeze` writing a baseline of pre-existing violations to `.stele/freeze.json` (committed alongside the lock; every later `check` reads it and suppresses exactly the baselined violations, so any NEW violation still fails) so legacy repos ratchet instead of failing wholesale (ArchUnit FreezeArchRule pattern). An `allow` entry suppresses BOTH structural directions for its `edge` (a violation import and a vestigial declared edge alike): it declares "the extractor cannot see this dependency — do not count it either way." It is distinct from `depends`, which asserts a dependency that MUST exist and is checked both ways. `reason` is mandatory and surfaced verbatim in `check --report`; v1 adds no cap or expiry — reason-plus-visibility is the sole governance against `# noqa`-style accumulation.

### 4.3 Exhaustiveness

Every top-level directory (configurable depth) maps into some node's territory — an unmapped directory is a recall failure, the most common failure mode in the measured set (SWE-Explore; missing-context dominates) [C6]. (eslint `boundaries/no-unknown` precedent.)

- _Territory_ is defined in §4.2 (declared subtree minus nested-node territories). Nesting is therefore NOT over-coverage; over-coverage means two nodes normalize to the SAME id, or two non-nested nodes declare equal directories.
- Files directly at the repo root (README, mix.exs, config/…) belong to the system node (id `/`) by construction; the system node always exists, so root files are never unmapped. **But the system node is NOT a subtree catch-all — a subdirectory is not "mapped" merely by being a descendant of `/`.** For exhaustiveness a non-ignored directory at depth ≤ D is mapped iff (a) it falls within some **non-root** node's territory, or (b) it is a structural ancestor of such a node (a pass-through like `apps/` that only holds container nodes) — the scan descends through pass-through dirs to check their own children regardless of D. A directory that is neither → exit 1.
- Directory scan honors an ignore set: `.git`, `.stele`, every VCS-ignored path (`.gitignore`), and a configurable `exhaustiveness.exclude` glob list (defaults `node_modules`, `_build`, `deps`, `target`). Untracked dot-directories are ignored.
- "top-level directory (configurable depth D, default 1)": every non-ignored directory at depth ≤ D must be **covered by a non-root node** — it is, contains (at any depth), or is nested under the declared location of at least one `container`, `component`, or `adr` node. The system node's `/` territory (§4.2) is a structural catch-all for import attribution ONLY; it satisfies exhaustiveness for repo-root direct files (bullet 2) but NEVER for a depth-≥1 directory — otherwise every directory under `/` would lie in the system territory by construction and the check could never fire. Deeper subdirectories inherit a covered ancestor's mapping and are not separately required. A depth-≤D directory covered by no such node (reachable through no router) → exit 1 — e.g. a new `apps/api/` with no node (EXAMPLE 8.6).

### 4.4 Budget

Accounting per materialized artifact per harness profile; the unit is explicit per profile — `claude` counts TOKENS, `codex` counts BYTES (UTF-8, the unit the harness truncates on):

- `claude`: root AGENTS.md + ancestor chain (always-loaded set) against a configured ceiling; default 2,000 tokens for the root [C12 — folklore-grade, therefore configurable].
- `codex`: full root→leaf concatenation chain against the 32 KiB default cap (`project_doc_max_bytes`, configurable, as of 2026-07 vendor docs) with margin — overflow truncation (reported silent) becomes a build failure [C4].
- Per-node `budget` fields enforced at emit.

**Tokenizer:** a bundled cl100k_base-class BPE approximation — no network, deterministic across platforms, accurate to ±10%; ceilings carry margin to absorb the error. The tokenizer identity is folded into the lock-format `version`, so a tokenizer change is a visible, reviewable bump.

**Counted content** for a node's `budget`: the entire materialized AGENTS.md it renders to — authored `stele` block + all generated regions + any free prose. Chain budgets (`claude` ancestor chain, `codex` root→leaf) count the concatenation of every file in that set.

### 4.5 Freshness

Primary signal: **AST-region digest.** At verify time, `build` stores a tree-sitter structural hash of the anchor's **bound definition** in the lock — `verified: {sha, digest}`. The bound definition is: for a `<path>#<symbol>` anchor, the resolved symbol's definition node; for an `lm:` landmark (or `stele:claim`) comment, the named definition (function/module/class) the comment **immediately precedes** in source order within its enclosing scope — a documentation-position binding, so `# stele:landmark refund-cap` sitting above `def changeset/2` digests `changeset/2`, not the surrounding module (EXAMPLE 8.4). If the comment precedes no definition in its scope, the digest falls back to its strictly-enclosing named node, then the file. (`resolved` records the comment/symbol line; the digested region is the bound definition, which may be a different line.) A claim stales when the digest changes, not when commits merely land nearby: formatting and comment churn vanish from the signal (AST ignores them); a flipped constant or reordered guard is an AST change and fires. An `enforced_by`-backed claim is not staled by a digest change — its guard, run by the same CI, is the freshness proof; only prose-only claims stale on a digest change alone. `stele blame <claim>` walks history recomputing the digest to name the staling commit.

Fallback signal (parser-less languages only): commits touching the anchored region since `verified.sha` (`git rev-list --count`), thresholded per node. Claims with `enforced_by` get a longer leash (the guard is the freshness proof); prose-only claims get a short one. Re-verifying = re-reading the region and rebuilding; `build` re-stamps only claims whose anchors still resolve.

**Known limitation (v1):** the digest detects changes _within_ the anchored region only. A semantic change _outside_ it that invalidates the claim — e.g. a new caller bypassing a guarded changeset — does not fire the freshness signal; `enforced_by` is the real proof there. This is why mechanizable claims are the stated direction of travel (§2.4), and why prose-only claims carry the shorter leash.

**HEAD is per-work-tree.** In an undercover multi-worktree layout (§3.5) the freshness fallback's commit walk and `stele blame` measure staleness against the *invoking* work tree's HEAD, not the shared home — an intentional semantic: the signal is relative to the tree the operator is actually working in, so a claim reads fresh or stale exactly as of that checkout.

### 4.6 Liveness

Every declared command is parsed and its executable resolved; `check --run-commands` optionally executes them scoped (the bonfires tier — off by default, on for nightly). Each command string is tokenized with POSIX shell word-splitting; leading `VAR=value` assignments are skipped; the first remaining token is the executable. It resolves if it is (a) a POSIX shell builtin (`cd`, `test`, `export`, …), (b) on `PATH`, (c) a repo-relative file with the executable bit, or (d) a known task of a detected runner — `mix <task>` → `mix.exs`/`mix help`, `npm|pnpm|yarn run <s>` → `package.json` scripts, `cargo <sub>` → builtins+aliases, `just <r>` → justfile. Shell operators (`&&`, `||`, `|`, `;`) split the string into multiple commands, each resolved independently.

## 5. Query surface

### 5.1 CLI (the portable tier — plain Bash tool calls)

```
stele root                                # the initialContext (§6), as text
stele node <id>                           # one node, all fields
stele unfold <id> [--depth 1]             # node + one-hop edge summaries (id, kind, purpose)
stele invariants [--touching <path>]      # cross-cutting transpose queries…
stele hazards [--node <id>]
stele nodes [--kind <kind>]
stele check [--only <class>]              # class ∈ referential|structural|exhaustiveness|budget|freshness|liveness (§4)
stele check [--freeze | --run-commands]   # --freeze baselines violations to .stele/freeze.json (§4.2); --run-commands executes them (§4.6)
stele check [--report]                    # human-readable findings, incl. every allow reason verbatim (§4.2)
stele emit [--check | --claude-rules]     # render regions + indexes; --check fails CI on divergence (§3.1); --claude-rules opt-in (§3.3)
stele blame <node-id>/<slug>              # walk history to the staling commit (§4.5)
stele build | init                        # all read/write commands take --json
```

**Queries are plain subcommands + flags — no query grammar.** Decision rationale (sequential-thinking gate, 2026-07-19): every v1 query is one noun + ≤2 filters; a grammar (GraphQL or bespoke DSL) adds parse surface and shell-escaping hazards — agents compose CLI flags reliably but are error-prone with nested quoting in Bash strings (a design judgment from this gate, not a measured result) — for zero expressiveness the enumerated queries need. Field selection saves ~nothing when whole nodes render at ~250 tokens. The data model stays graph-shaped and typed (the GraphQL _mental model_); the wire syntax is boring on purpose. If compositional queries ever materialize, a GraphQL surface rides on `stele serve` additively — the asymmetry is the point: a grammar is a one-way door, the flags surface wraps under any future syntax without breaking.

Cross-cutting queries are the capability nested files cannot provide at any size — the transpose (all invariants repo-wide, all commands per container) materialized from one graph (database-lens "Pivot") [C23].

**Command pipeline.** `build`: sources (tree + `stele` blocks + anchors + extracted imports) → in-memory graph → writes `.stele/graph.lock` (the ONLY writer of the lock). `emit`: reads the lock (never re-parses) → renders each AGENTS.md generated region, `.stele/index/*.md`, and per-harness projections; `emit --check` renders to memory and diffs on-disk (divergence → exit 1). `check`: runs `build` to an in-memory graph, byte-compares its canonical serialization to the committed lock (mismatch → exit 2), then runs the six assertion classes over that graph. `init`: the ONLY command that writes AGENTS.md source blocks; `emit` writes strictly inside marker regions and under `.stele/`. Recommended CI order: `stele check` → `stele emit --check`. CI does NOT run `stele build`: `check` already rebuilds to an in-memory graph and byte-compares it to the committed lock (§3.2), and `build` is the sole writer of `.stele/graph.lock` — running it first overwrites the committed lock on disk with a fresh build, making `check`'s comparison trivially pass and defeating the stale-lock gate. `build` is the local step, run before committing whenever sources change.

### 5.2 MCP (`stele serve`) — the Claude-first-class tier

Same engine, tools mirroring the read/query CLI verbs, one MCP tool per verb, named identically and un-prefixed within the server (the server's own name `stele` supplies the namespace, yielding fully-qualified `stele.root`, `stele.node`, `stele.unfold`, `stele.invariants`, `stele.hazards`, `stele.nodes`, `stele.check`, `stele.blame`). CLI flags map to named tool parameters (`unfold(id, depth?)`, `invariants(touching?)`, `hazards(node?)`, `nodes(kind?)`, `check(only?, run_commands?)`). The mutating verbs `build`/`init`/`emit` are NOT exposed over `serve` — they write source or the lock and belong to the shell/CI tier. Never required: every MCP tool has a CLI equivalent, and every CLI answer is derivable (slower) from the materialized files. Three-rung degradation ladder — **files → CLI → MCP** — is a spec invariant, not an implementation detail (hook-free-fallback house rule; llms.txt lesson: a pointer artifact without a consumption mechanism is unvalidated [C11]).

**Undercover mode (§3.5) is the one deliberate exception to the three-rung invariant.** By construction no tracked artifact exists for a no-engine collaborator harness to read, so the files rung is consciously sacrificed for everyone but the operator. The operator's ladder is **CLI → MCP** plus the single `CLAUDE.local.md` shim that re-injects the overlay root node into Claude's context — the private stand-in for the files rung. The full three-rung ladder holds for every committed graph; undercover trades the files rung for invisibility, by design.

### 5.3 Process contract

**Exit codes** (uniform across subcommands): `0` success (`check` clean); `1` assertion failure (≥1 violation from `check`/`emit --check` — reserved for "repo out of spec", never tool malfunction); `2` input error (malformed `stele` block: YAML parse error, unknown field, wrong type; duplicate/colliding node id; unknown lock `version`; committed lock ≠ freshly-built graph; bad flags); `3` internal error (tree-sitter/extractor crash, IO).

**Build atomicity:** any exit-2 condition prints offending `file:line` and aborts; `build` writes NO lock (never a partial graph). `query`/`serve`/`emit` read the committed lock and never rebuild — a missing lock → exit 2 "run stele build"; they trust the committed lock's freshness, since detecting staleness is `check`'s job, not theirs. They exit 2 only on a missing or unknown-`version` lock, never on staleness. `check` also requires a committed lock but rebuilds in-memory and byte-compares its canonical serialization to it (§5.1); committed lock ≠ freshly-built graph → exit 2 "run stele build". None of the four ever writes a lock (only `build` does; `init` writes source blocks, never the lock). Throughout this contract *the committed lock* names the committed lock in normal mode and the private on-disk lock at the graph home in undercover mode (§3.5); the byte-compare and every exit-2 condition above are mode-independent.

**`--json` envelope** (the stable machine contract; human-readable output is the default): `{"stele": <ver>, "command": "check", "ok": <bool>, "exit": <int>, "data": {…}, "findings": [{"class": "structural", "severity": "error", "node": "…", "message": "…", "fix": "…", "locations": [{"file": "…", "line": 9}]}]}`.

## 6. The initialContext (root contract)

`stele root` renders all six items below as text, in order [C15]; in the root AGENTS.md they split (§3.1) — items 1–2 (identity, commands) are the authored `stele` block, items 3–6 the generated region:

1. **Identity line** — system node's `purpose` (≤200 chars).
2. **Commands** — the system-level command table [C17].
3. **Hazard banner** — top-N active hazards across the graph, each one line + anchor.
4. **Router** — one line per first-hop node: `id · kind · purpose · unfold-with` (the explicit-router portability layer [C4, C12]).
5. **Index pointers** — one pointer line to each transpose index (§6.1); ~15 tokens total.
6. **Engine instructions** — 2 lines: how to `unfold`/query when the CLI/MCP is available, and the statement that everything is reachable through the files when it is not.

### 6.1 Transpose indexes (the no-engine cross-cutting path)

`emit` materializes `.stele/index/invariants.md` and `.stele/index/hazards.md` — the full repo-wide claim tables (claim · node · anchor), generated, `emit --check`-verified. The root carries the index pointer lines as their own section (§6 item 5), ~15 tokens total. Rationale (sequential-thinking gate, 2026-07-19): rendering all claims into the root is measured requirement-noise [C9, C17]; omitting the path entirely leaves no-engine harnesses blind on cross-cutting tasks — a recall failure [C6]. A materialized transpose file behind a pointer is load-bearing scent at root and full recall on demand, for every harness.

Hard exclusions, enforced by schema absence: architecture narrative, directory descriptions, style guidance derivable from linters, anything restating README [C7, C17].

## 7. Adoption path

- `stele init` — scans the tree, proposes node boundaries (top-level dirs → containers, import-cluster heuristics → components), and for each writes a **skeleton `stele` block with empty typed fields as the first fenced block** plus an **empty generated region** (`<!-- stele:begin router --><!-- stele:end -->`) for `emit` to later fill. It never generates prose, purposes, or invariants — generated restatement is measurably net-negative [C7]; the human/agent fills fields at the moment they know something non-derivable. **On a pre-existing AGENTS.md, `init` is non-destructive and idempotent:** a file that already carries a `stele` block is left untouched (authored fields are never overwritten, re-ordered, or pruned); a file with free prose but no `stele` block gets the skeleton block prepended as the first fenced block and its existing prose preserved below (thereafter counted as free prose, §3.1 item 3, and budget-flagged). `init` never deletes human content and never writes inside a generated region (that is `emit`'s sole domain, §5.1).
- `stele build` — compile the sources to `.stele/graph.lock` and commit it. `init` writes only AGENTS.md source blocks (§5.1), so no lock exists yet; `check`/`emit`/`query`/`serve` require a committed lock and exit 2 ("run stele build") without one (§5.3). This step is what makes the next one runnable.
- `stele check --freeze` — baseline existing structural violations into `.stele/freeze.json` (committed); ratchet down.
- `stele init --undercover` — the private variant (§3.5): writes the `.stele/undercover` marker, scaffolds the overlay node tree under `<home>/.stele/tree/`, and installs the `<common-dir>/info/exclude` managed block — never `git add`. `build`/`check`/`emit` and every query then behave identically, with all artifacts living at the graph home and the `CLAUDE.local.md` shim materialized in the work tree; `git status` stays clean.
- Incremental by construction: a repo with one root node and empty fields is valid, passes `check`, and emits a spec-compliant AGENTS.md. Value accrues per field filled.

## 8. Reserved extension points (v2+, schema-stable now)

- **Telemetry overlay** — `.stele/trails/` (gitignored): edge weights from session traces (pheromone router, circulation, confusion backlog). Node ids are the join key; the committed graph never carries behavioral data.
- **Resume deltas** — `stele brief <node> --since <sha>`: distilled `git log` over a node's territory since a watermark (the non-derivable-by-construction payload).
- **Antibody intake** — `.stele/intake/`: queue where session-end pipelines (e.g. a memory sweep classifying repo-durable learnings) deposit claim candidates as PR fodder.
- **Benchmark harness** — PR-distilled task suite gating doc changes on measured resolve-rate (proving-ground tier).

Each is additive; none changes the v1 schema. v1 ships the Signature layer only.

## 9. Implementation notes (Rust)

- Extraction: `tree-sitter-graph` crate (MIT/Apache) per language; SCIP import (`rust-analyzer` emits natively) as the high-fidelity path where an indexer exists [C22]. stack-graphs is archived — do not depend on it.
- Import-edge extraction per language is the long tail; v1 languages: Rust, TypeScript/JS, Elixir, Python (this machine's actual repos), behind one trait.
- Node ranking for budget-constrained rendering: tree-sitter tags + graph centrality + token-budget binary search (aider's repo-map mechanics; aider primary docs, verified 2026-07-19 · research/findings/sq2).
- Single static binary, zero runtime deps, `cargo install stele-cli` (crate name `stele` is squatted by a dormant 2024 atomic-Vec crate — binary stays `stele`; verified 2026-07-19) / prebuilt releases. Implementation is delegated to pinned opus agents per house rules; this spec + the assertion suite's fixtures are the review contract.

## 10. Resolved questions (decisions from the 2026-07-19 sequential-thinking gate; user veto welcome)

1. **Name: stele** — trellis and cairn are burned by AI-agent brand collisions (mindfold-ai/Trellis ★12.8k, two cairn agent tools — observed 2026-07-19); no colliding AI-agent tool named stele at check time (verified 2026-07-19).
2. **Anchor sigil: `stele:`** — greppable, self-naming, deliberately distinct from the `✻` human-notes convention.
3. **Query syntax: subcommands + flags, no grammar** — full rationale in §5.1. _Flagged for explicit user veto — this overrides the original "thinking in graphql" vocabulary while keeping the typed-graph data model._
4. **`purpose` ceiling: 200 chars, hard** — the one unverifiable field; the cap is the containment [C19].
5. **ADRs: detect existing convention** (`adr/`, `doc/adr/`, `docs/adr/`) at init; default `adr/` greenfield.
6. **`emit --claude-rules`: opt-in** — a second projection of the same claims is managed duplication, but still surface; the `@AGENTS.md` shim is the default Claude path.

## Decision log

- **Derived graph checked against authored intent, not authored-only or derived-only** — the reconciliation IS the product; each half alone is commodity [C23, C24, C26].
- **AGENTS.md as source+rendering in one file** (vs Charter's separate source tree) — preserves edit-in-place, no-engine readability, and the standard's nearest-wins semantics [C1, C26].
- **No embeddings, ever** [C5]. Behavioral/trace indexing is the only sanctioned index class, and it is v2, gitignored.
- **No free-prose node field** — typed fields are the curation bar; prose survives only outside the managed regions, budgeted and flagged [C7, C17, C26].
- **Closed edge vocabulary** — decidable checks and a learnable query surface beat expressive ontology [C14; IA §4: controlled vocabulary].
- **Rust** — for tree-sitter/SCIP embedding and single-binary distribution, not differentiation (C and Go twins exist) [C22, C23].
- **Flags over grammar** (0.2) — reversibility asymmetry: a query grammar is the one-way door, subcommands are wrappable by any future syntax; agents fumble nested shell quoting [gate 2026-07-19].
- **Transpose indexes as files, not root content** (0.2) — root claims-dump is measured requirement-noise [C17]; pointer + materialized view preserves recall [C6] at ~15 root tokens [gate 2026-07-19].
- **AST-region digest over churn-count for freshness** (0.2) — strictly dominant where a parser exists; churn stays as the parser-less fallback. Outside-region semantic changes still slip through, so `enforced_by` remains the real proof (§4.5) [gate 2026-07-19].
- **`.steleignore` for scan exclusion; markdown scans HTML comments only** (0.8) — discovered self-hosting: the spec's own docs quote anchor syntax and the test fixture vendors a full stele repo, so §2.4's unqualified scan broke `build` on this very repository. A committed root `.steleignore` (gitignore syntax) hides paths from node discovery, anchors, and extraction alike; it is a source artifact so the lock stays config-independent (§3.4 intact). Fenced code in markdown is quotation, not declaration — only `<!-- -->` counts (§2.5's "language-native comments", now explicit). `path#symbol` in markdown resolves against heading slugs (the definition analogue tree-sitter cannot provide) [dogfood + user decision 2026-07-20].
- **Undercover mode: private single-user graph rooted at parent-of-git-common-dir, overlay sources under `.stele/tree/`, one materialized `CLAUDE.local.md` shim** (0.9) — a single operator runs stele on a shared repo with zero collaborator-visible artifacts. The `.stele/undercover` marker is a deliberately-uncommitted source artifact: it moves where node sources are read, so it feeds `build` (unlike `.stele/config.toml`), yet it never enters version control. The parent-of-common-dir home is worktree-shared — the rule keeps one graph across disposable worktrees and outside every work tree in a bare-root layout. Normal mode is byte-for-byte unchanged, gated solely on the marker's presence. The §5.2 files rung is consciously sacrificed for collaborators — CLI/MCP plus the one shim are the operator's surface [user decision 2026-07-21].

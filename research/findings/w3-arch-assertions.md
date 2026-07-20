# W3 — Architecture-as-code conformance tools (declared architecture checked against real code)

Research date: 2026-07-19. Scope: tools that let you DECLARE an intended architecture and have CI CHECK it against the
actual code (import/dependency graph). Boundaries: conformance checking only — not diagramming, not doc frameworks
(see sq3), not repo-map generators (sq4). Focus questions per tool: (a) both directions? — undeclared dep fails AND
declared-but-unused edge flagged; (b) is the declaration human/agent-readable prose-adjacent or a test-code DSL buried
in tests; (c) does it couple the declaration to agent-read documentation; (d) CI ergonomics.

Verdict up front: **the doc-as-enforced-signature pattern is NOT shipped anywhere.** The forward direction (undeclared
dependency fails CI) is a mature, crowded space across every language. The reverse direction (a DECLARED edge that has
no corresponding import gets flagged as stale) is essentially absent from every mainstream tool. And no tool couples
its architecture declaration to the documentation an AI agent reads — the declaration always lives in a config/test
DSL that is machine-only, and the human/agent prose (README, AGENTS.md) is a separate, unchecked artifact. `agraph`'s
core novelty — one typed declaration that is simultaneously the agent's entry doc AND the CI conformance signature,
checked in BOTH directions — has no prior art in the surveyed field.

---

## The bidirectional gap (the crux finding)

Every conformance tool below checks the **forward** direction: "code imported X but the rules don't allow it → fail."
This is table stakes.

The **reverse** direction — "the declaration says module A may/does depend on B, but no such import exists in the code,
so the declared edge is stale/dead → flag it" — is not a first-class feature in any surveyed tool. The closest
partial primitives:

- **dependency-cruiser `required`** — enforces a dependency MUST exist (e.g. "every reducer must import redux"); fails
  if absent. This is presence-required, not unused-declaration-flagging. Source:
  https://github.com/sverweij/dependency-cruiser/blob/main/doc/rules-reference.md (accessed 2026-07-19; GitHub docs).
- **`no-orphans`** — flags code modules with no edges at all; about the code graph, not the declaration.
- None of ArchUnit / dependency-cruiser / eslint-plugin-boundaries / go-arch-lint flags an **allow-rule that never
  matched anything** ("dead permission"). An over-permissive `mayDependOn` / `allowed` entry that no code exercises
  passes silently. This is the exact staleness `agraph`'s "declared depends-edges vs extracted import graph, both
  directions" is designed to catch — and it is a genuine gap.

Rationale for why: these tools model architecture as a **whitelist of what is permitted**, not a **manifest of what
exists**. A whitelist can only ever be over-broad safely; unused entries are harmless to the checker even if they
mislead a human reader. `agraph` reframing the declaration as a manifest (must match reality exactly, both ways) is
the structural difference.

---

## 1. ArchUnit (Java) + family — the reference implementation, but test-code DSL

Source: https://www.archunit.org/ and userguide https://www.archunit.org/userguide/html/000_Index.html; repo
https://github.com/TNG/ArchUnit (accessed 2026-07-19; official site + GitHub). Type: primary.

- Mechanism: "checking the architecture of your Java code using any plain Java unit test framework." Analyzes
  **bytecode**, imports all classes into a Java code structure, evaluates rules as JUnit tests.
- Coverage: package/class dependencies, **layered architecture**, slices, **cyclic dependencies**, naming
  conventions. Rich predefined rules (`layeredArchitecture()`, `slices().should().beFreeOfCycles()`).
- (a) Direction: forward only. `noClasses().that()...should()...` fails on violations; does **not** flag a
  declared layer relation that no code uses. Cycle detection is a code-graph property, not a declaration check.
- (b) Declaration form: **test-code DSL buried in tests.** Rules are fluent Java compiled into a `@Test`/
  `@ArchTest`. This is the antithesis of prose-adjacent — it is Java that only a Java toolchain (and a reader fluent
  in the fluent API) can parse. Not agent-friendly as a declaration surface.
- (c) Documentation coupling: none. The rules live in `src/test`; any human-facing architecture doc is separate and
  unchecked.
- (d) CI: excellent — it IS a unit test, runs in the existing test job, `FreezingArchRule` lets you ratchet down
  violations on legacy code (freeze current violations, fail only on new ones).

Family (same model, same test-code-DSL shape, same forward-only direction):

- **ArchUnitNET** (C#) — https://github.com/TNG/ArchUnitNET — official TNG fork. "specify and assert architecture
  rules in C# for automated testing."
- **ts-arch / ArchUnitTS** (TypeScript) — https://github.com/LukasNiessen/ArchUnitTS — "Specify and ensure
  architecture rules in your TypeScript app." Runs in Jest/Vitest.
- **PyTestArch** (Python) — https://pypi.org/project/PyTestArch/ — "inspired by ArchUnit," rules as pytest tests.
- **go-arch-lint** — the exception in this family, see §3 (it uses YAML, not test code).

Takeaway: the ArchUnit family is the dominant paradigm and it is uniformly a **test-code DSL**, forward-direction
only, documentation-decoupled. Exactly the shape `agraph` is arguing against.

---

## 2. dependency-cruiser (JS/TS) — config-file rules, closest to prose-adjacent, still forward-only

Source: https://github.com/sverweij/dependency-cruiser + rules-reference.md (accessed 2026-07-19; GitHub docs).
Type: primary.

- Mechanism: walks the import graph of JS/TS/CoffeeScript (ES6/CJS/AMD), validates against your rules, and can
  visualize (graphviz).
- (b) Declaration form: **a config file** — `.dependency-cruiser.js` (or JSON) exporting an object with `forbidden`,
  `allowed`, `required` arrays; each rule has `from`/`to` with `path`, `pathNot`, `orphan`, `circular`,
  `couldNotResolve`. This is the most prose-adjacent of the mainstream tools — declarative data, not compiled test
  code — though still a machine-config idiom, not documentation.
- (a) Direction: forward-strong. `forbidden` (deny undeclared), `allowed` → emits **`not-in-allowed`** for anything
  outside the allow-list, `no-circular`, `no-orphans`. Reverse: `required` forces a dep to exist, but no
  flag for an `allowed`/`forbidden` rule that never fires. Folder-level rules via `scope: "folder"` (limited to
  `path`/`circular`/`moreUnstable`).
- (c) Documentation coupling: partial and one-directional — it can **generate** diagrams/archi reports FROM the graph,
  but the rules are not themselves the doc an agent reads.
- (d) CI: `--init` seeds sensible rules (circular, orphans, deps-not-in-package.json); `severity: "error"` yields a
  non-zero exit for build gating. Strong CI story.

Takeaway: proves a **declarative config** (not test code) can drive conformance — a useful precedent for `agraph`'s
"typed blocks" being data. But the config is still machine-only and forward-only.

---

## 3. go-arch-lint (Go) — YAML that doubles as documentation (closest prior art on (b)+(c))

Source: https://github.com/fe3dback/go-arch-lint (accessed 2026-07-19; GitHub). Type: primary.

- (b) Declaration form: **a YAML file** `.go-arch-lint.yml` with `components` (named layers via `in:` path globs) and
  `deps` (`mayDependOn:` relations), plus `commonComponents`. Declarative, readable.
- (c) Documentation coupling: **the strongest of any surveyed tool** — the maintainer frames the YAML as
  semantically "describing/declaring the project architecture," i.e. it functions as both enforcement and
  human-readable architecture documentation in one file. This is the nearest existing thing to a
  declaration-that-is-also-doc — but it is still a bespoke YAML, not the agent's entry point, and not typed beyond
  path globs + mayDependOn.
- (a) Direction: forward only — compares actual code deps to declared `mayDependOn`, "shows warnings when code
  violates rules." No unused-component / dead-permission flagging documented.
- (d) CI: "can be used in your CI workflow"; Docker + prebuilt binaries; exit 0/1 for pipeline gating.

Takeaway: go-arch-lint is the single best precedent for `agraph`'s (b)+(c) ambition (declarative, doc-adjacent), yet
it still (i) is forward-only, (ii) is not the agent's initialContext, and (iii) has no typed fields beyond
components/deps.

---

## 4. eslint-plugin-boundaries (JS/TS) — element-type layering inside the linter

Source: https://github.com/javierbrea/eslint-plugin-boundaries + https://www.jsboundaries.dev/docs/rules/ (accessed
2026-07-19; GitHub + docs site). Type: primary.

- Mechanism: an ESLint plugin. Define `boundaries/elements` (each element = `{ type, pattern }`, e.g.
  `{ type: "controller", pattern: "controllers/*" }`), then `boundaries/element-types` declares allowed relations
  ("controllers may depend on models and views; models may not depend on views").
- (b) Declaration form: **ESLint config** (`settings` + `rules`). Config-adjacent, not test code, but ESLint-idiom.
- (a) Direction: forward. Also ships `boundaries/no-unknown` — every file must belong to a known element (prevents
  stray files). That is a code→declaration completeness check (a partial reverse-ish signal: code with no declared
  element fails), closer to `agraph`'s "exhaustiveness" than most tools — but still no flagging of an unused
  allow-relation.
- (c) Documentation coupling: none — pure lint config.
- (d) CI: runs in the existing ESLint job; violations are lint errors. Frictionless if ESLint already gates CI.

Takeaway: `no-unknown` (files must map to a declared element) is a notable partial for `agraph`'s exhaustiveness
requirement — prior art exists for "every file must be accounted for in the declaration."

---

## 5. Rust — no ArchUnit equivalent; crate-level cycles free, module-level DIY

Sources: cargo-deny docs https://docs.rs/cargo-deny; cargo-modules (crates.io); "Rustifying Lakos"
https://talesfromthearmchair.net/rust-circular-dependencies.html (accessed 2026-07-19; official docs + practitioner
blog). Type: primary + practitioner.

- **Cargo/crate level**: Cargo "effectively rules out circular dependencies between crates" by construction (the
  dependency graph is a DAG). So inter-crate cycles are a non-problem for free. But **no built-in barrier to circular
  dependencies between modules WITHIN a crate.**
- **cargo-deny**: lints the **external dependency graph** — licenses, advisories/RUSTSEC, banned/duplicate crates,
  source allow-lists. It is a supply-chain/policy tool, **not** an internal-module-architecture checker. Declaration
  = `deny.toml` (readable TOML). Does NOT check declared internal module boundaries against code.
- **cargo-modules**: prints/visualizes the internal module dependency graph (`--no-fns --no-traits --no-types` for
  module-only). It is a **visualizer/analyzer**, not a rule engine — you can build a cycle barrier on top of its
  output, but there is no declared-architecture-vs-code conformance out of the box.
- No mainstream Rust tool offers ArchUnit-style declared-layer conformance. This is a **gap in the Rust ecosystem
  itself** — directly relevant since `agraph` is a Rust binary: there is no incumbent Rust conformance tool to
  displace, and building the extracted-import-graph checker in Rust is greenfield territory.

Takeaway: for Rust specifically the forward direction isn't even well-served, let alone the reverse. `agraph` would be
early even for the mature (forward) half of the space in Rust.

---

## 6. Structurizr DSL / models-as-code (Simon Brown) — diagram/model, generation not conformance

Sources: https://docs.structurizr.com/dsl, https://docs.structurizr.com/dsl/language (accessed 2026-07-19; official
docs). Type: primary.

- Structurizr is **"models as code"** for the C4 model — one DSL model renders many diagrams. Text-based, which the
  docs note makes it "easy for AI agents to parse your model" (AI summaries, queries, drift detection use-cases).
- **`!components` keyword**: "a DSL wrapper around the Structurizr for Java component finder, providing the ability to
  automatically discover components in a Java codebase." Critically this is **code → model GENERATION** (populate the
  model from code), not **model → code CONFORMANCE** (fail when code diverges from the model). Detailed component-
  finder docs are paywalled (Patreon early access).
- No documented feature validates that declared relationships in the model match real import dependencies, or fails
  CI on divergence. Structurizr is diagram/model-authoring; conformance is out of scope in the shipped product.
- Direction: n/a — it is not a checker. Documentation coupling: it IS the documentation/diagram, but nothing enforces
  the doc against code.

Takeaway: Structurizr answers the sub-question's explicit query — **a Structurizr model is essentially diagram/model-
only**; the one code-touching feature (`!components`) generates FROM code, it does not check code AGAINST the model.
The "self-asserting doc" idea is exactly what Structurizr does NOT do.

---

## 7. Architecture fitness functions (Ford/Parsons, _Building Evolutionary Architectures_)

Sources: https://nealford.com/books/buildingevolutionaryarchitectures.html;
https://www.oreilly.com/library/view/building-evolutionary-architectures/9781492097532/ch02.html;
https://www.infoq.com/articles/fitness-functions-architecture/ (accessed 2026-07-19; primary book + InfoQ). Type:
primary concept / secondary commentary.

- Definition: a fitness function is "any mechanism that provides an objective integrity assessment of some
  architectural characteristic(s)." The governance shift is "governance by inspection → governance by rule" — a
  PR-failing automated test instead of a hoped-for code-review catch.
- This is the **conceptual umbrella** over everything in §§1–5. The canonical implementations named in the
  literature are ArchUnit / ArchUnitTS. There is no dedicated "fitness function runtime" — you compose them from
  existing test/lint tools.
- Relevance to `agraph`: the CI self-assertion (anchor liveness, token budgets, freshness watermarks, both-direction
  edge check) is squarely a **suite of fitness functions**. The concept validates the approach; the novelty is not
  "having fitness functions" but (i) making the DECLARATION they check also the agent's entry document, and (ii) the
  reverse-direction edge check.

---

## 8. AGENTS.md / CLAUDE.md as boundary declaration — declared, essentially never enforced

Sources: https://agents.md/; guides at augmentcode.com, morphllm.com, asdlc.io (accessed 2026-07-19; standard site +
practitioner guides). Type: secondary/practitioner.

- Practitioners DO put architectural boundaries in AGENTS.md as **prose**: e.g. "imports from `src/internal/` are
  never permitted outside that directory," "database access goes through repositories in `src/repos/`." The
  three-tier "Always / Ask first / Never" boundary pattern is the common idiom.
- Enforcement is near-absent and shallow where it exists: the strongest reported practice is "diff AGENTS.md changes
  in PRs and require human approval" and "AGENTS.md can be linted, validated in CI" — but this is linting the FILE
  (freshness/format), **not checking the declared boundaries against the actual import graph.** No tool was found that
  parses boundary declarations out of AGENTS.md/CLAUDE.md and enforces them against code.
- This is the **exact gap `agraph` targets**: the boundary lives as unenforced natural-language prose in the very
  document the agent reads, while the enforced version (if any) lives separately in ArchUnit/dependency-cruiser
  config. Nobody has unified them.

Takeaway: the two halves `agraph` fuses exist independently in the wild — (i) boundaries-as-agent-prose in AGENTS.md,
and (ii) boundaries-as-CI-conformance in ArchUnit/dep-cruiser/go-arch-lint — but **no shipped tool makes the agent's
document itself the enforced signature.** That fusion is the genuine gap.

---

## Scorecard

| Tool                      | (a) both directions                 | (b) declaration form                | (c) coupled to agent doc      | (d) CI                  |
| ------------------------- | ----------------------------------- | ----------------------------------- | ----------------------------- | ----------------------- |
| ArchUnit + NET/TS/Py      | forward only                        | test-code DSL (buried in tests)     | no                            | excellent (is a test)   |
| dependency-cruiser        | forward + `required` presence       | config file (.js/JSON, declarative) | no (generates diagrams)       | strong (exit code)      |
| go-arch-lint              | forward only                        | YAML, doc-adjacent                  | **partial — YAML is the doc** | good (exit 0/1)         |
| eslint-plugin-boundaries  | forward + `no-unknown` completeness | ESLint config                       | no                            | frictionless (lint job) |
| Rust (cargo-deny/modules) | forward only (external / cycles)    | TOML / none                         | no                            | good                    |
| Structurizr DSL           | n/a (not a checker)                 | DSL model                           | it IS a diagram, unchecked    | n/a                     |
| AGENTS.md boundaries      | none (prose)                        | natural-language prose              | **YES — but unenforced**      | file-lint only          |

Two cells are the whole story: go-arch-lint gets closest on "declaration doubles as doc," and AGENTS.md is the only
place the declaration IS the agent's document — but it is unenforced. **No row is both agent-doc-coupled AND enforced
AND bidirectional.** That empty intersection is `agraph`'s claim.

---

## Why this file is not thin

It is not thin — the space is well-populated on the forward-direction/conformance half, and 6 searches + 5 full
fetches gave solid primary coverage of every tool named in the sub-question plus the two conceptual anchors (fitness
functions, AGENTS.md-as-boundary). The one genuinely thin spot is Structurizr's component-finder internals (detailed
docs are Patreon-paywalled), but the public docs are unambiguous that it generates model-from-code rather than checking
code-against-model, which is all the sub-question needs.

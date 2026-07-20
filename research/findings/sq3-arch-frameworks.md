# SQ3 — Architecture-description frameworks adaptable to hierarchical agent-facing docs

Research date: 2026-07-19. Scope: existing doc/architecture frameworks whose structure can be borrowed for a
hierarchical, agent-facing repo-documentation spec. Failure modes, agent standards (AGENTS.md/CLAUDE.md), repo-map
tooling, and generators are siblings and are excluded except where a source directly informs the crux question.

## Queries run

1. C4 model context containers components code zoom levels documentation guidance
2. Diátaxis tutorial how-to reference explanation why mixing modes fails
3. arc42 architecture template structure sections
4. Architecture Decision Records ADR conventions Michael Nygard adr-tools log4brains
5. Simon Brown notation-independent zoom "architecture haiku" README-driven development fractal documentation (dud — didn't surface the specific terms; had to split)
6. curated documentation improve LLM coding agent task performance vs exploration retrieval (crux)
7. "architecture haiku" one page architecture description constraints quality attributes
8. Backstage TechDocs docs-like-code hierarchical documentation nested per-component
9. Tom Preston-Werner readme driven development write readme first

Pages fetched in full: c4model.com; diataxis.fr/start-here; cognitect.com ADR (Nygard primary, 2011);
arxiv 2602.12670 (SkillsBench); dev.to Simon-Brown-minimal (thin — pointer only); arxiv 2509.19931 (doc-retrieval
planning, crux).

---

## 1. C4 model — the zoom-level backbone (adopt the hierarchy)

Source: https://c4model.com/ (official, Simon Brown) + https://c4model.com/diagrams. Type: primary framework site.

- Four **hierarchical abstractions**, each a zoom level with a distinct audience: **System Context → Container →
  Component → Code**. The C4 name = the four C's.
  - **L1 System Context** — the system as one box plus its users and external systems. Owns _scope and
    boundaries_; audience = everyone incl. non-technical. (Maps to a root AGENTS.md "what is this / what talks to it".)
  - **L2 Container** — separately deployable/runnable units (apps, services, data stores) and how they communicate.
    "The most universally useful level" (maps to deployment architecture). Owns _technical decomposition_. (Maps to
    per-service/per-app mid-level docs.)
  - **L3 Component** — zoom into one container: major logical building blocks behind well-defined interfaces, and
    their interactions. Owns _design within a deployable unit_. (Maps to per-module/per-package docs.)
  - **L4 Code** — classes/functions. Brown's official guidance: **generate from source, don't hand-maintain** — hand-
    drawn code diagrams "go stale almost immediately." (Directly supports: leaf level = the code itself + generated
    artifacts, not curated prose.)
- **Explicit zoom-in relationship**: every element at a higher level "opens up" into the next level. This is the
  single most transferable idea for a fractal agent-doc spec — each doc is complete at its altitude and points down.
- **Notation-independent and tooling-independent** (verbatim framing on the site) — the _structure_ is the contract,
  not any diagram syntax. Good: our spec can adopt the altitude discipline without mandating diagrams.
- Official "only draw what adds value": **most teams need only L1 + L2**. Practical implication for us: don't force
  a fixed depth; depth follows the codebase's actual nesting.
- **"Map of your code" / Google-Maps-zoom analogy** is Brown's canonical metaphor (one altitude at a time; don't cram
  all detail on one page). This is the core "keep each doc at one altitude" rule to encode.

## 2. arc42 — the section checklist (adopt as a menu, not a mandate)

Source: https://arc42.org/overview + docs.arc42.org. Type: primary template.

12 standardized sections; **every section optional**:
1 Introduction & Goals (top 3–5 quality goals, stakeholders) · 2 Constraints · 3 Context & Scope · 4 Solution
Strategy · 5 Building Block View (_static decomposition as nested white-box/black-box hierarchy_ — this is arc42's
own fractal-decomposition section, the direct analog of C4 zoom) · 6 Runtime View · 7 Deployment View · 8
Cross-Cutting Concepts · 9 Architecture Decisions (→ ADRs) · 10 Quality Requirements · 11 Risks & Technical Debt ·
12 Glossary.

- Adaptability: arc42 = _what topics a level's doc might cover_; C4 = _how levels nest_. They compose (Brown himself
  pairs C4 models with the arc42 template — see §6). arc42 §5 "Building Block View" is literally "hierarchy of white
  boxes containing black boxes, up to the appropriate level of detail" — the fractal principle stated as a section.
- For agent docs: sections 3 (context), 5 (decomposition), 8 (cross-cutting concepts), 9 (decisions), 12 (glossary)
  are the highest-value; runtime/deployment/quality are situational. Treat as an optional menu, matching arc42's own
  "all sections optional" stance — avoids forcing empty boilerplate that wastes an agent's context budget.

## 3. Architecture Decision Records — the decision-capture unit (adopt the convention)

Source: https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions (Nygard, 2011, primary) +
https://adr.github.io/. Type: primary + standards hub.

- Nygard template = **Title · Status · Context · Decision · Consequences**. Decision in active voice ("We will…");
  Context is value-neutral; Consequences list positive, negative, AND neutral outcomes.
- Rationale (verbatim-ish): "Large documents are never kept up to date… Nobody ever reads large documents." Small,
  modular, per-decision records "have at least a chance at being updated." Directly echoes the crux argument for
  curated-small over comprehensive-large.
- Conventions to adopt wholesale:
  - Live **in the repo** under `doc/adr/` or `doc/arch/adr-NNN.md`.
  - **Numbered monotonically; numbers never reused.**
  - **Immutable**: superseded ADRs stay, marked "superseded by NNN" — an audit trail, not deletion. (Important for
    agents: history of _why_ prevents blind re-litigation.)
  - One or two pages each.
- Tooling: **adr-tools** (Nat Pryce, CLI to create/supersede/index; ships its own ADRs as worked examples);
  **Log4brains** (thomvaill) = docs-as-code ADR base, log from IDE, auto-publishes a static site. adr.github.io
  hosts alternative templates (MADR = Markdown ADR is the other common one). Adopt-vs-build: **adopt the format and
  numbering convention; likely adopt adr-tools' CLI patterns rather than build a new ADR generator.**

## 4. Diátaxis — the mode-separation discipline (adopt the "one mode per doc" rule)

Source: https://diataxis.fr/start-here/ (primary, Daniele Procida). Type: primary framework.

- Two axes: **action ↔ cognition** and **acquisition (study) ↔ application (work)**. Four quadrants:
  - **Tutorial** = action + acquisition (learning by doing, instructor-led).
  - **How-to** = action + application (competent user getting work done).
  - **Reference** = cognition + application (facts, no interpretation; mirrors the system's structure "just like a map").
  - **Explanation** = cognition + acquisition (understanding, the "why").
- Why mixing fails (verbatim): "crossing or blurring the boundaries… is at the heart of a vast number of problems in
  documentation." A doc serving two modes at once serves neither. Canonical anti-patterns: tutorial derailed by
  reference options; how-to interrupted by design essays.
- Most transferable rule for agent docs: **each doc file should occupy one mode.** For our framework this maps to:
  reference (auto-generated API/module facts) must be kept separate from explanation (curated "why/how it fits").
  Mixing is what makes human docs bloat — and bloat is exactly what blows an agent's context budget.
- Diátaxis's "reference architecture should mirror the thing it describes" = the same map principle as C4/arc42 §5:
  **doc tree should be isomorphic to the code tree.** Strong convergent signal across three independent frameworks.

## 5. Prior art on "fractal / zooming / complete-at-each-resolution" docs

Multiple sources; each level complete at its own resolution is the recurring idea.

- **Simon Brown "map of code" / C4 zoom** (see §1) — the primary articulation of notation-independent, altitude-per-
  document zoom. Brown's "minimal approach" bundles: C4 models-as-code (Structurizr) + arc42 template + ADR tools
  (source: dev.to/maxarshinov — thin, pointer only; the trio composition is the takeaway).
- **Architecture Haiku** (George Fairbanks, ~2010; https://www.rhinoresearch.com/assets/pdf/arch-haiku-2010-09-07.pdf;
  ResearchGate case study 276079978). Type: practitioner method + case study. **One page, uber-terse**: quality-
  attribute priorities, constraints, tradeoffs, design rationale, architectural styles, key diagrams. The extreme
  size constraint _forces_ focus on load-bearing decisions — precisely the discipline a root AGENTS.md needs to fit a
  context window. Caveat: assumes an educated reader (knows arch drivers/QA scenarios). Directly transferable as the
  model for the **root compressed picture** = an "architecture haiku for agents." Strong adopt candidate for the
  top-of-hierarchy format.
- **README-Driven Development** (Tom Preston-Werner, 2010; https://tom.preston-werner.com/2010/08/23/readme-driven-
  development). Type: primary essay. Write the README first; it forces you to think through the design before code.
  "A perfect implementation of the wrong specification is worthless." Punishes over-long/over-precise specs, rewards
  small modular libs. Transferable framing: the curated doc is the _spec the code is measured against_, authored
  before/alongside code — supports "curated hierarchical doc" over "reconstruct from code."
- **Backstage TechDocs** (Spotify, 2020; https://backstage.io/docs/features/techdocs/). Type: primary platform docs.
  **Docs-like-code**: Markdown lives beside code, one component = one repo = one doc site, tied via catalog metadata
  (`backstage.io/techdocs-ref`). Monorepo case: a main component aggregates distributed doc folders from other repo
  parts (MkDocs plugin). This is the closest existing production pattern to "hierarchical docs nested per component,
  co-located with code, discoverable via a manifest" — i.e. the deployment shape our framework wants. Adopt the
  co-location + per-component + manifest-linkage pattern; the catalog annotation is a concrete precedent for a
  root manifest that points to child docs.

Convergence worth flagging: C4 (§1), arc42 §5, and Diátaxis-reference (§4) _independently_ prescribe the same rule —
**the doc structure must be isomorphic to the system structure, and each document holds exactly one altitude/mode.**
That triangulation is the strongest design signal in this sub-question.

## 6. Crux evidence — does curated hierarchical doc beat on-demand exploration for agents?

Two primary papers found; both point YES, with the decisive factor being _curation + examples_, not mere retrieval.

- **SkillsBench** (arxiv 2602.12670, 2026). Type: primary benchmark paper.
  - "Skills" = structured packages of _procedural_ knowledge (SKILL.md + optional code/examples) injected at
    inference time — the closest published analog to curated hierarchical agent docs.
  - **Curated skills: +16.2 pp avg** across 7 model-harness configs. **Self-generated skills: −1.3 pp** ("models
    cannot reliably author the procedural knowledge they benefit from consuming"). → Argues for _human/curated_
    docs over agent-reconstructed-on-the-fly.
  - Domain variance huge: Healthcare +51.9pp, Manufacturing +41.9pp, but **Software Engineering only +4.5pp**
    (34.4%→38.9%), Math +6.0pp. IMPORTANT caveat for our crux: the SE gain is real but modest — the strongest gains
    are in domains where procedural conventions are non-obvious; a well-explored, idiomatic codebase may benefit less.
  - Dosage: **2–3 skills optimal (+18.6pp); 4+ diminishing (+5.9pp).** "Focused procedural guidance… more effective
    than exhaustive documentation"; detailed/focused (+18.8pp) beats comprehensive (−2.9pp). → Directly validates the
    "compressed, altitude-limited, don't-dump-everything" design; over-documentation can _hurt_.
  - Smaller-model+skills can beat larger-model-without (Haiku+skills 27.7% > Opus w/o 22.0%).
  - Limits: containerized/terminal eval; may not transfer to GUI/multi-agent; contamination not fully excluded.
- **Documentation Retrieval Improves Planning Language Generation** (arxiv 2509.19931, 2025). Type: primary.
  - Retrieval lifts recall **18.30 vs 9.03** (~2×) for unseen functions.
  - **Code examples are the decisive component**: with examples 0.66–0.82 accuracy vs 0.22–0.39 without. → For our
    leaf docs, _runnable examples/snippets_ matter more than prose description.
  - Curated/structured docs beat unfiltered corpus retrieval; dense-embedding retrieval underperforms on domain
    jargon (marginal gains from fancy pipelines — "simplicity often suffices").
  - Documentation influences the _initial generation_ phase more than error-refinement.

Net for the crux: published evidence supports curated, focused, example-bearing, hierarchically-scoped docs over both
(a) agent self-generated docs and (b) exhaustive dumps. But the **software-engineering-specific uplift is the weakest
domain measured (+4.5pp)**, so our spec should be validated on our own repos, and should bias toward _procedural/
convention_ content and _examples_ (where evidence is strongest) rather than exhaustive structural prose (which the
agent can grep). This is the tension the framework must resolve, not assume away.

---

## Adopt-vs-build shortlist (for the spec)

- **Adopt**: C4 zoom hierarchy + "one altitude per doc" + "L4=generate, don't curate"; ADR format + numbering +
  immutability + `doc/adr/` location (and likely adr-tools/MADR rather than a new ADR generator); Diátaxis "one mode
  per doc" separation (esp. reference-vs-explanation); TechDocs co-location + per-component + manifest-linkage;
  Architecture-Haiku one-page extreme-terse format for the root compressed picture.
- **Consider adopting**: arc42 12-section menu (as optional topic checklist per level, not mandate).
- **Design rules the sources triangulate**: doc tree isomorphic to code tree; small-modular beats large-monolithic;
  focused+examples beats comprehensive; depth follows actual nesting (don't force fixed levels).
- **Open risk to test empirically**: SE-domain uplift from curated docs is modest (+4.5pp) — the crux is _not_
  settled for well-idiomatic code; measure on target repos before committing generator/drift-check tooling.

## Why any part is thin

Not thin overall. Two soft spots: (a) Simon Brown's own "minimal approach" essay wasn't fetched at primary source —
the dev.to secondary only confirmed the C4+arc42+ADR trio composition; the specific self-contained-at-each-resolution
wording lives in Brown's books/talks behind Leanpub, not free-fetchable. (b) "Fractal documentation" as a named term
returned no canonical framework — the concept exists only distributed across C4/arc42/Haiku/TechDocs, so §5 synthesizes
rather than cites a single authority. Both gaps are on wording/attribution, not substance.

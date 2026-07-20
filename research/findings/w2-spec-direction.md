# Wave 2 — Spec Direction & Official Tooling

Research date: 2026-07-19. Two primary targets fetched in full plus one adjacent
docs page. This file is **not thin** — both targets were rich. Quotes kept < 15 words.

---

## Target 1 — AGENTS.md v1.1 proposal (Issue #135)

- **URL:** https://github.com/agentsmd/agents.md/issues/135
- **Source type:** GitHub issue (community proposal) on the official spec repo
- **Repo ownership:** `agentsmd/agents.md`. Spec is "stewarded by the Agentic AI
  Foundation under the Linux Foundation" — no longer an OpenAI-owned repo. Format
  "emerged from" OpenAI Codex, Amp, Jules (Google), Cursor, Factory.
- **Author:** `johncmunson` (John Munson). Association: NONE (not a repo maintainer/member).
- **Status: OPEN, unmerged.** Created 2026-01-08; last updated 2026-06-11. It is a
  _draft proposal_, not adopted spec text as of 2026-07.
- **Traction:** 5 THUMBS_UP + 1 EYES on the issue. 4 comments, **all from
  authorAssociation=NONE** (no maintainer has commented on the record). No labels,
  no assignees. Read as: a well-developed community draft with mild interest, no
  official ratification signal.

### What it proposes (four planks)

1. **Nesting semantics ("Jurisdiction" + "Accumulation" + "Implicit Inheritance").**
   - "An AGENTS.md file applies to all files and subdirectories within its
     containing folder." Ancestor-based only; siblings unaffected.
   - Guidance **accumulates** down the tree — a child "extends and builds upon"
     ancestors "rather than replacing it entirely."
   - Implicit inheritance means "a leaf AGENTS.md file can be small and focused";
     project-wide conventions belong at root. (This is exactly the compressed-root
     → hierarchical-unpack model the framework is chasing, stated as spec intent.)

2. **Precedence / conflict resolution.** Explicit chain, highest→lowest:
   `LLM System Prompt → Agent System Prompt → User Prompt → Local AGENTS.md →
Ancestor AGENTS.md (nearest first)`. Rule: "more specific instructions take
   precedence over more general ones." User prompt overrides file guidance;
   override _syntax_ is left to authors.

3. **Progressive disclosure (the headline recommendation, in "For Implementers").**
   Three-step recommended approach:
   - **Index** — at session start scan for AGENTS.md files, extract paths + frontmatter.
   - **Inject index** — give the agent a "lightweight index of available" files.
   - **Load on demand** — load full contents "as needed for the task at hand."
   - Enabled by **optional YAML frontmatter**: `description` (rec. < 200 chars) and
     `tags`. Frontmatter is optional "because AGENTS.md files rely primarily on
     positional context — the path itself serves as the identifier." Index format
     left to implementers (JSON / XML / Markdown examples given).
   - A **Compaction & Summarization** section (spec tail) adds: keep directive
     rules/invariants, compact prose/examples/redundant inherited guidance; "should
     not retain both a summarized and a full representation" of one file; allow
     discard + reload by relevance.

4. **Scope clarification vs SKILL.md.** "AGENTS.md focuses on behavior (rules,
   constraints, workflows). SKILL.md focuses on capabilities (what an agent knows
   how to do)." Complementary, not competing; when both present, AGENTS.md "should
   avoid detailing skills and may reference SKILL.md." README.md demoted to "a
   fallback," not a peer.

### Best-practices numbers stated in the draft

- "aim for under 500 lines" per file (guideline). Split across nested files when extensive.
- Backwards compatible: "All existing AGENTS.md files remain valid." v1.1 = clarifications
  - additions (frontmatter, implementer guidance, design principles); nothing removed.

### Comments — high-value leads for this framework

- **`dgenio` (2026-03-02, 1 HEART):** reviewed 7 papers + applied to a production
  monorepo. Directly load-bearing for Wave 1 open items:
  - **Chatlatanagulchai et al. 2025 — arXiv 2511.12884:** optimal context-file length
    **335–535 words** (diminishing returns beyond); testing instructions appear in
    **75%** of high-quality files (most common section).
  - **ContextBench 2025 — arXiv 2602.05892:** **file localization is the #1 failure
    mode** for AI agents (wrong file → cascade).
  - **Lulla et al. 2025 — arXiv 2601.20404:** well-structured AGENTS.md on SWE-bench
    Verified (Claude 3.5 Sonnet): **28.64% runtime reduction, 16.58% token reduction.**
  - **Gloaguen 2025 — arXiv 2602.11988:** "LLM-generated documentation hurts agent
    performance by ~3%" vs human-written. **This is very likely the primary source
    behind Wave 1's secondary-sourced "ETH Zurich: generated context hurts" claim —
    verify author/affiliation.**
  - Galster 2025: 4,860 context files analyzed; AGENTS.md is the interoperable standard.
- **`johncmunson` (author, 2026-03-04):** "number one anti-pattern ... letting the
  agent ... generate the AGENTS.md file rather than authoring it by hand." (Converges
  with Gloaguen. Directly cautions against the framework's _generation_ leg.)
- **`stackbilt-admin` (2026-06-11) — MOST IMPORTANT for the framework's novelty claim.**
  Proposes progressive disclosure via an optional **module index** and points at a
  shipping tool that **already does hierarchical generation + CI drift-checking together**:
  - **Charter** (Apache-2.0, github.com/Stackbilt-dev/charter): "trigger-based modular
    agent context since early 2026"; manifest declares always-loaded vs on-demand
    modules with trigger keywords + per-bundle token budgets; a bundler composes
    exactly the modules a task needs; repo "governs itself with the mechanism."
  - `adf migrate` classifies an existing flat CLAUDE.md/AGENTS.md/.cursorrules "by
    rule strength" and routes content into modules (auto-split — the generation leg).
  - `charter adf compile --target agents` renders modular source back to one flat
    AGENTS.md; **`--check` gates CI on drift between the two** (the drift-check leg).
  - **⚠ This directly challenges Wave 1's premise that "no known tool does
    hierarchical generation + CI drift-checking together." Charter appears to. Audit it.**
  - Design constraints worth stealing: flat files stay valid; graceful degradation
    (non-conforming tools just read a Markdown list of "read file X when Y");
    deterministic resolution (string/substring match, "no LLM call"); no new format.
  - Module index = HTML-comment-fenced `<!-- agents-modules -->` list in AGENTS.md,
    each line: path — desc — `Triggers: kw, kw` — optional `(~600 tokens)` budget hint.
  - Notes **Codex caps project docs at 32 KiB, drops the rest silently**; field
    guidance "converges on keeping AGENTS.md under ~150–200 lines" (matches Wave 1's
    "≤200-line router" practitioner convergence).
  - References issue **#71** (proposes a dedicated `.agent/` directory).
- **`AlexKenbo` (2026-04-21):** harness-internals angle (v1.2 scope): AGENTS.md
  fragments on the "prompt tape" need consistent handling — exclude from user-turn
  rollback boundaries, exclude `AGENTS_MD_FRAGMENT`/`SKILL_FRAGMENT` from memory
  extraction inputs ("prompt scaffolding, not conversation content"). Tangential to
  the docs framework but relevant if it ever feeds a memory pipeline.

---

## Target 2 — Claude Code large-codebases docs

- **URL:** https://code.claude.com/docs/en/large-codebases
- **Source type:** Official Claude Code documentation
- **Title:** "Set up Claude Code in a monorepo or large codebase"

### Recommended layout & loading model

- **Root + per-directory CLAUDE.md.** "Claude Code loads every CLAUDE.md file from
  your working directory and every parent directory at launch, then loads each
  subdirectory's file on demand when it reads files there." This is progressive
  disclosure already implemented for CLAUDE.md — start-from-root loads **"Root only;
  subdirectory files load on demand."**
- Common split = two levels: root = repo-wide (coding standards, commit conventions,
  layout); per-subdirectory = area/stack-specific. Root CLAUDE.md example is a short
  router orienting Claude to package structure (matches the ≤200-line router pattern).
- **Path-scoped rules** under `.claude/rules/` as an alternative to per-dir CLAUDE.md:
  loads "when Claude works with a file matching the rule's `paths:` glob." Choose
  per-dir CLAUDE.md when owners version conventions with code; choose rules when you
  want all conventions centralized or one rule spans scattered paths.
- **`claudeMdExcludes`** setting (glob or absolute path, matched against absolute
  paths — prefix relative patterns with `**/`): skips specific CLAUDE.md/rules files
  so "they never load." Static, not a per-task switch. Merges across settings scopes
  (user/project/local/managed). Managed-policy CLAUDE.md files cannot be excluded.

### Size / drift guidance (as stated)

- **No hard line/word cap on this page.** Guidance is qualitative: a single root file
  "tends to either grow to cover every subsystem ... or stay too generic." (For a
  numeric cap, the AGENTS.md draft's "under 500 lines" and dgenio's 335–535 words are
  the citeable figures — Claude's docs defer to splitting rather than a number.)
- **Staleness / drift is called out explicitly.** Keep files current via: review
  CLAUDE.md edits in PRs; revisit after major model releases (delete workaround-rules
  a newer model no longer needs); "Add a Stop hook that proposes updates" — a Stop
  hook gets the transcript path and "can review the session and propose CLAUDE.md
  updates while the gap ... is fresh." (A generation/refresh primitive.)
- "Centralize conventions when layering stops scaling" — move on-demand content into
  skills / plugins / MCP (e.g. an existing RAG index exposed as an MCP tool). A
  SessionStart hook can print a plugin recommendation into context.

### Other loading facts relevant to a hierarchical framework

- `additionalDirectories` grants file access but **never loads** that dir's
  CLAUDE.md/rules/skills. `--add-dir` loads skills, and loads CLAUDE.md/rules only
  with env `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`.
- Skills are the on-demand tier: "loads on demand when Claude determines it's
  relevant"; `paths:` frontmatter glob scopes a root skill to matching files.
  Descriptions get shortened when many skills exist — lead with request keywords.

---

## Target 3 (glance) — InstructionsLoaded hook (drift-tooling primitive)

- **URL:** https://code.claude.com/docs/en/hooks
- **Source type:** Official Claude Code documentation
- **This is the clean drift-detection primitive for the framework.** Fires when a
  CLAUDE.md or `.claude/rules/*.md` file loads into context — at session start and
  during lazy loading when Claude enters a new directory.
- **Matcher = load reason:** `session_start`, `nested_traversal`, `path_glob_match`,
  `include` (frontmatter include of another file), `compact` (reload after compaction).
- **Input `loaded_instructions[]`:** each `{ file_path (absolute), load_reason }`.
  So a hook can record _exactly which instruction files actually reached context, and
  why_ — enables auditing "did the doc I expected to load actually load," coverage
  checks, and CI-style drift alarms keyed on the real load set.
- **Non-blocking / observability-only:** cannot block or modify which files load;
  exit code + stderr ignored. Intended uses listed: auditing, side effects,
  validation, logging.
- Companion: **Stop hook** (above) is the write-back half — propose doc updates from
  the finished transcript. InstructionsLoaded (what loaded) + Stop (what was missing)
  = a closed drift loop, both first-class in the harness.

---

## Bottom line for the framework thesis

- The **hierarchical root→detail→code** model is now explicit spec _intent_ (v1.1
  jurisdiction/accumulation/inheritance) and already the _implemented_ CLAUDE.md
  loading behavior — the framework is aligned with where the standard is heading, not
  inventing against it.
- **Progressive disclosure = index + inject + load-on-demand** is the recommended
  pattern in BOTH the AGENTS.md draft and Claude Code's actual loader.
- Two Wave-1 premises need revision:
  1. "Generated context hurts / human-written helps" now has a likely **primary
     source (Gloaguen, arXiv 2602.11988, ~3% hurt)** to fetch, plus practitioner
     agreement (johncmunson).
  2. "No tool does hierarchical generation + CI drift-checking together" is
     **contradicted by Charter** (`adf migrate` splits, `adf compile --check` gates
     CI on drift) — must be audited before the novelty claim stands.
- Claude Code ships the drift-loop primitives (InstructionsLoaded + Stop hook) the
  framework would otherwise have to build.

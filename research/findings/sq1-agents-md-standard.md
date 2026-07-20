# SQ1 — The AGENTS.md standard & Claude Code's CLAUDE.md mechanics (mid-2026)

Scope: standards + native mechanics only. Repo-map tooling, doc generators, and
failure-mode/performance evidence belong to sibling agents and are excluded here.
This file is _not_ thin — primary sources (agents.md spec, code.claude.com/docs,
OpenAI Codex docs, Linux Foundation press) covered every sub-point.

## Queries run

1. `AGENTS.md specification standard agents.md who maintains`
2. `AGENTS.md format nesting subdirectories precedence spec`
3. `Claude Code CLAUDE.md discovery order memory user project directory scoped official docs`
4. `Claude Code CLAUDE.md @import syntax max depth 5 hops recursive imports docs`
5. `AGENTS.md donated Linux Foundation Agentic AI Foundation December 2025`

Pages fetched in full: agents.md/ (spec homepage); code.claude.com/docs/en/memory
(official Claude Code memory doc); learn.chatgpt.com/docs/agent-configuration/agents-md
(OpenAI Codex AGENTS.md guide — backward hop from the spec's tool list). Linux
Foundation / AAIF press confirmed via search synthesis.

---

## PART A — AGENTS.md

### Ownership & governance

- **Now stewarded by the Agentic AI Foundation (AAIF) under the Linux Foundation.**
  Formation announced **2025-12-09**; AGENTS.md was a founding donated project
  (alongside Anthropic's MCP and Block's goose). Source: Linux Foundation press
  release, 2025-12-09 (press release / primary); OpenAI "OpenAI co-founds the
  Agentic AI Foundation" 2025-12-09 (vendor primary); TechCrunch / InfoQ 2025-12-09
  (news, corroborating).
- **Origin:** released/formalized as an open spec by **OpenAI in August 2025**,
  with collaboration from Amp, Google, Cursor, and Factory. Source: agents.md
  homepage + LF press release.
- AAIF Platinum members named: AWS, Anthropic, Block, Bloomberg, Cloudflare,
  Google, Microsoft, OpenAI. (LF press release, 2025-12-09.)
- Canonical repo referenced across sources: `openai/agents.md` (also `agentsmd/agents.md`
  in one GitHub issue URL — the org appears to have moved; treat `agents.md` site as
  authoritative). Source type: spec site + GitHub.

### What the spec says (format)

- **"A README for agents"** — a dedicated, predictable place for context/instructions
  for AI coding agents; complements README.md. Source: agents.md (spec, primary).
- **No required structure or fields.** Direct spec quote: instructions are "just
  standard Markdown. Use any headings you like." Common (not mandatory) sections:
  project overview, build/test commands, code style, testing instructions, security
  considerations. Source: agents.md.
- Intentionally minimal — plain Markdown, no schema. This is the key design contrast
  with a structured/typed manifest.

### Nesting & precedence (spec)

- **Nearest file wins.** Spec quote: "Agents automatically read the nearest file in
  the directory tree, so the closest one takes precedence and every subproject can
  ship tailored instructions." Source: agents.md.
- Conflict rule (spec quote): "The closest AGENTS.md to the edited file wins;
  explicit user chat prompts override everything."
- A file at `./frontend/AGENTS.md` governs everything under `./frontend/` and has no
  bearing on sibling dirs. Root + nested are considered _together_; more specific
  overrides more general. Source: agents.md + community guides (secondary).
- **Note:** the _spec itself_ is loose on whether ancestor + descendant are merged
  vs. only-nearest-loaded. The concrete, implementable precedence semantics live in
  each harness's docs (see Codex below). A proposal to make this explicit is open:
  GitHub issue #135 "AGENTS.md v1.1: Making Implicit Semantics Explicit … Progressive
  Disclosure" (agentsmd/agents.md, community proposal — a lead, not yet spec).

### Codex's concrete implementation (backward hop — most precise precedence spec found)

Source: OpenAI Codex docs, learn.chatgpt.com/docs/agent-configuration/agents-md
(vendor primary). Three-tier chain:

1. **Global:** `~/.codex/AGENTS.override.md` first, then `~/.codex/AGENTS.md` — first
   non-empty file only.
2. **Project:** from Git root walking toward cwd, each dir checks
   `AGENTS.override.md` → `AGENTS.md` → names in `project_doc_fallback_filenames`.
3. **Merge:** quote — "Codex concatenates files from the root down, joining them with
   blank lines. Files closer to your current directory override earlier guidance
   because they appear later in the combined prompt."

- `AGENTS.override.md` = temporary override without deleting the base file.
- Size cap: `project_doc_max_bytes` default **32 KiB**; skips empty files, stops
  adding once combined size hits the limit.
- `CODEX_HOME` env var repoints the Codex home.
- This "concatenate root→cwd, later overrides" model is **structurally identical to
  Claude Code's** (see Part B) — a convergent convention worth adopting.

### Tool/harness support & adoption

- **Adoption: 60,000+ open-source repos** (GitHub search cited on agents.md homepage
  and LF press release, both 2025-12). ~**30+ agents** read it.
- Supporting tools (union across agents.md + LF press release): OpenAI Codex, Google
  Jules, Gemini CLI, GitHub Copilot, Cursor, Devin, Factory, Amp, Aider, Zed, Warp,
  VS Code, Windsurf, JetBrains Junie.
- **Claude Code does NOT natively read AGENTS.md.** Official Claude Code docs state
  Claude reads `CLAUDE.md`, not `AGENTS.md`. Recommended bridge: a `CLAUDE.md` that
  does `@AGENTS.md` import (or a symlink `ln -s AGENTS.md CLAUDE.md`; on Windows use
  the import since symlinks need admin/Developer Mode). `/init` also reads an existing
  AGENTS.md (and `.cursorrules`, `.devin/rules/`, `.windsurfrules`) and folds relevant
  parts into the generated CLAUDE.md. Source: code.claude.com/docs/en/memory (primary).
  - Caveat: some sources still list "Claude Code" among AGENTS.md supporters; the
    authoritative Claude Code doc explicitly says it reads CLAUDE.md only. Treat the
    bridge (import/symlink) as the real integration path.
- **VS Code / Copilot CLI nested support was still landing in early 2026** — open
  issues: microsoft/vscode #266120 "Support AGENTS.md in parent and sub-folders";
  github/copilot-cli #1655 "Flag for nested agents.md files". Lead: nesting support
  is uneven across harnesses even where top-level AGENTS.md is read.

---

## PART B — Claude Code native mechanics

All from **code.claude.com/docs/en/memory** ("How Claude remembers your project"),
official docs, fetched 2026-07-19 (primary). Version-gated notes cite the min-version
comments embedded in the doc.

### Two systems

- **CLAUDE.md** (you write; instructions/rules) and **auto memory** (Claude writes;
  learnings). Both load at the start of _every_ conversation. Both are context, not
  enforced config — "To block an action regardless … use a PreToolUse hook."

### CLAUDE.md discovery / load order (broadest → most specific; later = higher priority

because later text in context gets more attention)

1. **Managed policy** — macOS `/Library/Application Support/ClaudeCode/CLAUDE.md`;
   Linux/WSL `/etc/claude-code/CLAUDE.md`; Windows `C:\Program Files\ClaudeCode\CLAUDE.md`.
   Cannot be excluded by users. Can instead be inlined via `claudeMd` key in
   managed-settings.json.
2. **User** — `~/.claude/CLAUDE.md`.
3. **Project** — `./CLAUDE.md` or `./.claude/CLAUDE.md`.
4. **Local** — `./CLAUDE.local.md` (gitignore it).

- All discovered files are **concatenated, not overridden**. Across the tree, ordered
  **filesystem root → cwd**, so `foo/CLAUDE.md` precedes `foo/bar/CLAUDE.md`. Within a
  dir, `CLAUDE.local.md` is appended after `CLAUDE.md`.

### When directory-scoped CLAUDE.md files load (session start vs on file access)

- **Ancestor files (cwd and up): loaded in full at launch.** Direct quote: "CLAUDE.md
  and CLAUDE.local.md files in the directory hierarchy above the working directory are
  loaded in full at launch."
- **Subdirectory files (below cwd): load on demand.** Direct quote: "Files in
  subdirectories load on demand when Claude reads files in those directories." /
  "they are included when Claude reads files in those subdirectories."
- Compaction nuance: project-root CLAUDE.md is re-read from disk and re-injected after
  `/compact`; **nested subdir CLAUDE.md are NOT auto re-injected** — they reload next
  time Claude reads a file there. (Directly relevant to a hierarchical-doc design:
  deep docs are lazy and evaporate on compaction.)

### @-import syntax and its limits

- Syntax: `@path/to/import` anywhere in a CLAUDE.md.
- Both **relative and absolute** paths; relative resolves **relative to the file
  containing the import**, not cwd. Home-dir imports allowed (`@~/.claude/…`) — the
  recommended way to share personal instructions across worktrees.
- **Recursive imports allowed; maximum depth = FOUR hops.** Official doc says "a
  maximum depth of four hops." (Some third-party guides say 5 — the official doc is
  authoritative at four.)
- **Imports load at launch and DO enter the context window** — they aid organization
  but do **not** save context. (Explicit in doc; repeated in the "too large" section.)
- Import parsing **skips Markdown code spans and fenced code blocks** — wrap a path in
  backticks to mention it without importing.
- First-time external imports trigger a one-time approval dialog; declining disables
  them permanently (no re-prompt).

### `.claude/rules/` with `paths` frontmatter

- Purpose: split large instructions into modular topic files; `.claude/rules/*.md`
  discovered **recursively** (subdirs like `frontend/`, `backend/` supported).
- **Rules WITHOUT `paths` frontmatter:** loaded at launch, **same priority as
  `.claude/CLAUDE.md`**.
- **Path-scoped rules (`paths:` YAML frontmatter, glob patterns):** load **only when
  Claude reads a file matching the pattern**, not on every tool use. This is the
  native "progressive disclosure" primitive. Example patterns: `src/api/**/*.ts`,
  `src/**/*.{ts,tsx}`. Rules without `paths` apply unconditionally.
- User-level rules `~/.claude/rules/` load before project rules (project rules win).
- Symlinks supported (share rule sets across projects; circular symlinks handled).
- Version notes: as of v2.1.198 path matching works through symlinked checkouts;
  before v2.1.207 one invalid glob broke Read for all files the rule touched; before
  v2.1.211 on-demand/path-scoped rules loaded even when `project` was excluded from
  `--setting-sources`.

### Size guidance & tooling (mechanics relevant to the framework)

- **Target < 200 lines per CLAUDE.md**; loaded in full regardless of length, but
  longer = more context + lower adherence. Path-scoped rules are the recommended
  escape hatch (imports are NOT — they don't cut context).
- `MEMORY.md` (auto memory) loads only first **200 lines / 25KB** each session; topic
  files load on demand. (Auto memory is the sibling system; noted for completeness.)
- Block-level HTML comments in CLAUDE.md are stripped before injection (free
  maintainer notes); preserved inside code blocks and when Read directly.
- `claudeMdExcludes` (glob, any settings layer, arrays merge) skips other teams'
  CLAUDE.md in monorepos; managed policy files cannot be excluded.
- `/doctor` (v2.1.206+) proposes trims: cuts derivable content (dir layouts,
  dependency lists, architecture overviews), keeps pitfalls/rationale/conventions
  that differ from tool defaults. **Directly germane to the crux** — Anthropic's own
  tooling treats "structure Claude can derive by exploring" as trimmable, implying
  curated docs should carry only the non-derivable.
- `InstructionsLoaded` hook logs exactly which instruction files loaded, when, and why
  — useful for the drift-check tooling deliverable.
- `--add-dir` dirs don't load CLAUDE.md unless `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`.

---

## Design-relevant takeaways (for the framework, not sibling scope)

- **Convergent convention exists:** both AGENTS.md/Codex and Claude Code use
  "concatenate root→cwd, nearest overrides, deeper loads lazily." A universal root
  file + nested files is natively supported by both ecosystems.
- **Native progressive disclosure = path-scoped `.claude/rules/` (Claude) and nested
  AGENTS.md (Codex).** `@`-imports are NOT progressive — they all load at launch and
  cost context. Any "unpacks hierarchically" design must lean on path-scoping/nesting,
  not imports.
- **Cross-tool portability:** author AGENTS.md as source of truth; bridge to Claude
  via `@AGENTS.md` in CLAUDE.md or symlink. One authoritative doc, two readers.
- Anthropic's `/doctor` trim heuristic is a strong prior on the crux question
  (curated vs. on-demand): keep the non-derivable, drop what grep/read recovers.

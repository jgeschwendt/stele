# SQ6 — How real teams structure CLAUDE.md/AGENTS.md in practice (mid-2026)

Scope: practitioner/vendor **patterns** for laying out agent-instruction files — root vs
subdirectory vs skills, token/size rules of thumb, monorepo nesting, progressive disclosure.
Spec mechanics (sq1), performance evidence (sq2), doc frameworks (sq3), generators (sq4),
failure-mode research (sq5) belong to siblings and are only touched where a pattern implies them.

## Queries run

1. `Claude Code best practices CLAUDE.md content size Anthropic engineering`
2. `AGENTS.md monorepo nested per-package convention examples GitHub`
3. `CLAUDE.md token budget always-loaded context what belongs subdirectory skills`
4. `Anthropic "Claude Code best practices" engineering blog CLAUDE.md tune iterate concise`
5. `CLAUDE.md AGENTS.md what belongs hacker news discussion context bloat` (news.ycombinator.com only)
6. `openai codex repository 88 AGENTS.md files nested example github`

Fetched in full: Anthropic best-practices doc; Claude "Steering Claude Code" blog; HumanLayer
blog; Datadog dev.to monorepo post; codegateway Codex playbook; OpenAI official Codex AGENTS.md
guide (redirect → learn.chatgpt.com); HN Ask thread 48160604; Codex issue #12115. Backward
chain: search-2 → openai/codex repo → issue #12115 (dynamic-nesting gap).

---

## Finding 1 — Anthropic official: CLAUDE.md is always-loaded; an explicit include/exclude table

- Source: **Best practices for Claude Code**, code.claude.com/docs/en/best-practices — vendor
  (Anthropic), official docs, current as of 2026-07 fetch.
- "CLAUDE.md is loaded every session, so only include things that apply broadly." Per-line test:
  _"Would removing this cause Claude to make mistakes?" If not, cut it._ Explicit warning:
  "Bloated CLAUDE.md files cause Claude to ignore your actual instructions."
- Include/Exclude table (verbatim buckets):
  - INCLUDE: bash commands Claude can't guess; code-style rules that differ from defaults; test
    instructions/preferred runners; repo etiquette (branch/PR conventions); project-specific
    architectural decisions; env quirks/required vars; non-obvious gotchas.
  - EXCLUDE: anything Claude can infer from code; standard language conventions; detailed API
    docs (link instead); frequently-changing info; long tutorials; file-by-file codebase
    descriptions; self-evident practices ("write clean code").
- Mechanics that matter for a framework: `/init` generates a starter and you "refine over time";
  imports via `@path/to/import` (e.g. `See @README.md`, `@docs/git-instructions.md`); locations =
  home `~/.claude/CLAUDE.md`, project root, `CLAUDE.local.md` (gitignored), **parent dirs**
  (monorepo, pulled automatically) and **child dirs** ("pulled in **on demand** when it reads a
  file in those directories"). Emphasis knobs: "IMPORTANT"/"YOU MUST". "Treat CLAUDE.md like
  code: review it, prune regularly, test changes by observing behavior."
- Routing rule (into sq-framework territory): "For domain knowledge or workflows that are only
  relevant sometimes, use skills instead. Claude loads them on demand without bloating every
  conversation."

## Finding 2 — Anthropic blog: concrete size ceiling + the four-way routing model

- Source: **Steering Claude Code: when to use CLAUDE.md, skills, hooks, and subagents**,
  claude.com/blog/steering-claude-code-skills-hooks-rules-subagents-and-more — vendor (Anthropic),
  blog.
- Hard rule of thumb (the most-cited number): _"Keep CLAUDE.md under 200 lines, give it an owner,
  and review changes to it like code."_
- Loading-cost model (directly relevant to the crux — always-loaded vs on-demand budget):
  - **Root CLAUDE.md** — always loaded at session start; memoized; **re-read after compaction**.
  - **Subdirectory CLAUDE.md** — loads **on-demand** when Claude reads a file under it; "lost
    until that subdirectory is touched again" (i.e. dropped on compaction, not re-injected).
  - **Skills** — only name/description loaded at start; full body loads on invocation, "re-injected
    up to a shared budget; oldest dropped first."
  - **Hooks** — config lives outside the context window; near-zero context cost; deterministic.
  - **Subagents** — separate context window; only final message returns.
- Content routing: root CLAUDE.md = build commands, directory layout, monorepo structure, coding
  conventions, team norms. **Procedural** workflows (deploy checklists, release, review playbooks)
  → skills, not CLAUDE.md. Path-scoped rules use `paths:` frontmatter so they "activate only for
  matching files."

## Finding 3 — HumanLayer: real numbers, WHAT/WHY/HOW, and an `agent_docs/` disclosure tree

- Source: **Writing a good CLAUDE.md**, humanlayer.dev/blog/writing-a-good-claude-md —
  practitioner (agent-tooling company), blog.
- Sizes observed in the wild: keep under **~300 lines**; HumanLayer's own **root file is <60
  lines**. Rules section: keep **under ~15 rules** — more than that means you haven't decided which
  are load-bearing.
- Root CLAUDE.md answers three things: **WHAT** (tech stack, project structure, codebase map —
  "especially critical for monorepos"), **WHY** (purpose, what each part does), **HOW** (tooling,
  verification, test/typecheck/compile steps).
- Progressive-disclosure pattern = a sibling **`agent_docs/`** dir of task-specific markdown
  (`building_the_project.md`, `running_tests.md`, `code_conventions.md`, `service_architecture.md`)
  referenced from root with a one-line description each; agent decides relevance. This is the exact
  "root map → detail docs → code" shape the framework targets.
- Instruction-count ceiling (crux-adjacent, borders sq2/sq5): frontier models hold ~**150–200
  instructions** with reasonable consistency; Claude Code's own system prompt already spends ~50;
  so CLAUDE.md should carry few, universally-applicable ones. Smaller models degrade "MUCH more
  quickly" (exponential vs linear). Prefer **"pointers to copies"** — `file:line` references over
  pasted code snippets, so context stays authoritative and current.
- Exclude list mirrors Anthropic: code style (use linters), exhaustive command refs, unrelated DB
  schema, auto-generated content.

## Finding 4 — Datadog: nested-alone is insufficient; use a root **router** + `.agents/`

- Source: **Steering AI Agents in Monorepos with AGENTS.md**,
  dev.to/datadog-frontend-dev/steering-ai-agents-in-monorepos-with-agentsmd-13g0 — practitioner
  (Datadog frontend team), blog.
- Position: "Nested AGENTS.md are the default recommendation for monorepo … but I find this
  approach quite limited on its own." Nearest-file-wins auto-discovery fails when the agent never
  "touches" the right directory, so pair it with an explicit root **router/dispatcher**.
- Layout: root `AGENTS.md` = routing table (`To create an email, read @emails/AGENTS.md` /
  `To create a Go service, read @go/services/AGENTS.md` / `To add unit tests, read
@.agents/unit-tests.md`); nested `AGENTS.md` = domain-specific per-folder; **`.agents/`** dir =
  generic cross-cutting guidance not tied to a package.
- Terseness rationale (token budget): "Characters add to an agent context window, so terseness
  allows them to do more."
- Claude interop trick: `echo "Read @AGENTS.md" > CLAUDE.md` — one-line CLAUDE.md that imports the
  shared AGENTS.md, leaving room for Claude-specific extensions without duplication. (Corroborated
  by HN item 48629831: "My CLAUDE.md is just: @AGENTS.md.")

## Finding 5 — OpenAI Codex: the only hard byte budgets + a _different_ loading model

- Sources: **codegateway AGENTS.md playbook 2026**,
  codegateway.dev/en/blog/agents-md-playbook-2026 (practitioner deep-dive) and **OpenAI official
  Codex AGENTS.md guide**, learn.chatgpt.com/docs/agent-configuration/agents-md (redirected from
  developers.openai.com/codex/guides/agents-md) — vendor (OpenAI), official docs.
- Concrete limits (the sharpest numbers found anywhere):
  - Combined instruction chain capped at **32 KiB** (`project_doc_max_bytes`, raisable to 64 KiB).
    Warning: "28 KiB of style guidance … can starve repo-specific instructions further down."
  - Per-file guidance: keep each AGENTS.md to **1–3 KiB (5–15 lines/section)**; global file **≤5
    KiB** to preserve budget for nested instructions.
- Loading model — **materially different from Claude Code**: Codex loads only files **on the path
  from Git root → cwd**, concatenated root-first, "files closer to your current directory override
  earlier guidance because they appear later." A dev in `apps/web/components` sees
  global → root → `apps/web` but **not** `apps/api` or `packages/ui`, and **not** deeper siblings.
  Per directory it takes the first non-empty of `AGENTS.override.md` → `AGENTS.md` → configurable
  fallbacks; `.override.md` silences the same-level `AGENTS.md` (personal, uncommitted layer).
- Loaded **once per session** (static) — contrast Claude Code's on-demand child-file pull-in.
  Implication for a "universal" framework: the same nested tree behaves differently across agents;
  Codex will _not_ auto-discover a deep package file unless cwd is inside it.

## Finding 6 — OpenAI's own repo is the canonical large-monorepo example

- Source: reported across morphllm / buildbetter / codersera guides citing **openai/codex**;
  root file github.com/openai/codex/blob/main/AGENTS.md — vendor repo (public GitHub), practitioner
  aggregation.
- Reported scale: the main OpenAI repo carries **~88 AGENTS.md files** across sub-components — one
  at root plus per-subpackage — the go-to concrete public example of per-package nesting. (Number
  is second-hand from guides; verify by `find . -name AGENTS.md | wc -l` against a clone before
  citing as fact.) AGENTS.md itself is a cross-vendor convention (OpenAI Codex, Amp, Google Jules,
  Cursor, Factory).

## Finding 7 — Nested/dynamic loading & path-scoping is an acknowledged open gap

- Source: **openai/codex issue #12115** "Dynamically loading nested AGENTS.md" — vendor issue
  tracker, open/unassigned as fetched.
- Enterprise customers named (Wix, Stripe, Clay) want (a) clearer precedence/debuggability of which
  files are active and (b) **path/glob-aware scoped rules**. Codex today has no documented dynamic
  (on-read) loading of deep files — it's a feature request, not shipped behavior. Signals the
  market wants Claude-Code-style on-demand disclosure + explicit scoping, but it isn't standardized.

## Finding 8 — HN practitioner consensus on what actually gets followed

- Source: **Ask HN: Do you still spend time maintaining Claude.md/AGENTS.md files?**,
  news.ycombinator.com/item?id=48160604 — practitioner discussion.
- Compliance patterns (borders sq5 but stated as authoring guidance):
  - **Factual content works** (directory structure, commands, doc references); so do constraint
    rules tied to an _observed_ failure if kept to one sentence.
  - **Preventative rules invented from scratch are largely ignored** — "Rules made from scratch are
    usually not followed. The 50-line limitation is justified for this category."
  - **Prefer positive to negative framing**: "Always clarify intent before acting" beats "Never act
    without getting intent" — negative phrasing can prime the undesired behavior.
  - Trend toward distributing guidance (localized files + reference dirs + skill repos) over one
    monolith. Related HN threads: 48289950 (daily-driver: CLAUDE.md/skills/subagents/plugins),
    45786738 ("How I use every Claude Code feature"), 44381169 (standardize on AGENTS.md).

---

## Synthesis for the framework decision (adopt vs build)

- **Strong convergence to adopt, not invent**: every vendor + practitioner independently lands on
  root = compressed always-loaded map/router → nested/detail docs (on-demand) → code. That _is_ the
  proposed framework. Adopt AGENTS.md as the canonical filename with a one-line `CLAUDE.md →
@AGENTS.md` shim for Claude Code.
- **Numbers to bake into the spec/drift-check** (converging rules of thumb):
  root ≤ **200 lines / ≤5 KiB**; whole session chain ≤ **32 KiB** (Codex hard cap); per nested file
  **1–3 KiB / 5–15 lines per section**; rules **≤15**; total instructions well under **~150–200**;
  HumanLayer's <60-line root as an aspirational floor.
- **The load-bearing divergence to design around**: loading semantics are _not_ portable. Claude
  Code pulls child files **on-demand when reading them**; Codex loads only the **static root→cwd
  chain** and won't discover deep/sibling files. A "universal" generator must not assume on-read
  disclosure works everywhere — either keep detail reachable via explicit `@`-import routers from
  root (Datadog pattern, portable) or emit per-agent variants.
- **Crux (does curated hierarchy beat grep/read?)**: this sub-question is pattern evidence, not
  measured proof — that's sq2. But the _unanimous_ practitioner rationale is context economy:
  always-loaded tokens degrade performance (Anthropic: bloat → ignored rules; the whole
  best-practices doc hangs on "context fills fast, performance degrades"). Hierarchy is adopted
  precisely to keep the always-loaded slice small while detail stays retrievable — an argument for
  curated compression over dumping raw code into context, not a benchmark. `file:line` "pointers
  over copies" (HumanLayer) is the concrete tactic bridging map → code.

## Why any thinness

Not thin — 8 findings across 4 vendor/official sources and 4 practitioner sources, with hard
numbers. Two gaps left for siblings/verification: (1) the openai/codex "88 files" count is
second-hand and should be re-counted against a clone; (2) measured performance of hierarchy vs
grep is deliberately deferred to sq2.

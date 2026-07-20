# SQ4 — Generators & Drift Tooling for Agent-Facing Codebase Docs

Scope: existing open-source generators that produce/maintain hierarchical agent-facing
codebase docs, plus doc-drift detection and CI checks that docs match code. Boundaries
(owned by siblings, excluded here): the AGENTS.md/CLAUDE.md _spec_ itself, repo-maps-for-
context, general doc frameworks, and failure-mode research.

Compiled 2026-07-19. Each finding cites URL, access date, source type. Quotes kept <15 words.

## Queries run

1. DeepWiki Cognition automatic codebase wiki generation
2. deepwiki-open self-hosted repository documentation generator
3. Swimm documentation drift detection code coupling CI
4. AGENTS.md generator tool automatically create CLAUDE.md
5. embedme markdown-code-embed keep code snippets docs in sync CI
6. CodeWiki academic system automatic hierarchical code documentation generation LLM
7. LLM-generated AGENTS.md reduced success rate research benchmark human-written
8. mutable.ai auto wiki codebase deprecated status 2025
9. documentation code anchor checksum CI fail when referenced code changes tool (dud — surfaced only embedme/Swimm, no new anchor-checksum tool)
10. "mutable.ai" shut down discontinued acquired 2025

---

## Category A — Whole-codebase auto-wiki generators (hierarchical, human-first)

### DeepWiki (Cognition AI) — hosted, closed-source

- What it generates: architecture diagrams, module-level explanations, dependency maps,
  and a conversational Q&A assistant over the code. Treats source as ground truth,
  regenerating from code. Use by swapping `github.com`→`deepwiki.com` on any public repo.
- Hierarchical: yes (module tree + architecture overviews), but rendered as a browsable
  web wiki, not files committed into the repo.
- Staleness: re-generates from current code on (re)indexing; no per-change drift-detection
  or CI gating described. Marketing claims accuracy "as code evolves" but no committed
  artifact to go stale.
- Maturity/adoption: launched 2025-04-25 by Cognition (makers of Devin). Indexed 50k+
  repos, ~4B LOC, ~$300k compute for indexing. Free for public repos; private via Devin.
- Agent-facing?: designed for human browsing, though Devin consumes it as agent context.
- Source: https://docs.devin.ai/work-with-devin/deepwiki (vendor docs, accessed 2026-07-19);
  https://ghost.codersera.com/blog/what-is-deepwiki-ai-code-documentation-github-repository/
  (blog, 2026-07-19).

### deepwiki-open (AsyncFuncAI) — open-source clone of the above

- What it generates: comprehensive markdown docs, auto Mermaid diagrams (architecture +
  data flow), RAG-powered Q&A chat over the repo. GitHub/GitLab/Bitbucket.
- Hierarchical: yes — builds an internal index of component relationships; organized,
  navigable wiki structure.
- Staleness: no drift detection or re-indexing mechanism documented — regenerate-on-demand
  only. This is a gap for a maintained-docs use case.
- Maturity: 17.4k stars, 1.9k forks, MIT, Python+TS. Notably **0 published releases**
  and 202 open issues — active but not packaged/versioned; deploy-as-web-app posture.
- Providers: OpenAI, Gemini, OpenRouter, Ollama (100% local), custom LiteLLM. Self-hostable
  via docker-compose with your own keys.
- Source: https://github.com/AsyncFuncAI/deepwiki-open (repo, accessed 2026-07-19).

### mutable.ai "Auto Wiki" — DEAD (do not adopt as a live dependency)

- What it was: turned a codebase into a Wikipedia-style article with diagrams; Auto Wiki v2
  added diagrams + AI-revisable wikis ($2/repo/mo).
- Status: **effectively shut down**. Company (YC W22, founder Omar Shams) went dark end of
  2024; CEO joined Google (~Dec 2024). No open-sourcing of the tech. Listed among AI
  startups that folded. Treat any surviving `wiki.mutable.ai` endpoint as unmaintained.
- Relevance: cautionary precedent — hosted auto-wiki generators are fragile as
  infrastructure; prefer local/OSS + committed artifacts.
- Source: https://news.ycombinator.com/item?id=42542512 ("Ask HN: What Happened to
  Mutable.ai", forum, accessed 2026-07-19); https://blog.mutable.ai/p/auto-wiki-v2
  (vendor blog, historical).

### CodeWiki (FSoft AI4Code) — academic, open-source, closest to our design

- What it generates: holistic repository-level markdown docs across 7 languages
  (Python, Java, JS, TS, C, C++, C#), with feature summaries, usage guides, and visual
  diagrams (architecture, data flow).
- Pipeline (3 phases, directly relevant to our hierarchical-unpack design):
  1. **Repository analysis** — Tree-Sitter ASTs → dependency graph of functions/classes/
     modules; hierarchical decomposition into modules.
  2. **Recursive multi-agent generation** — per-leaf-module agents with source + module
     tree + dep graph; "dynamic delegation" to sub-agents when complexity exceeds capacity,
     enabling arbitrary repo sizes with bounded processing.
  3. **Hierarchical assembly** — synthesize parent-module docs from child docs via LLM,
     up to a repo overview. Maintains cross-module references.
- Hierarchical: yes, explicitly — "preserves architectural context across multiple levels
  of granularity." This is the strongest published blueprint for the unpack-hierarchically
  goal.
- Staleness: **not addressed** — paper has no update/drift/staleness mechanism. Generation
  is one-shot. A build-vs-adopt gap our framework must fill.
- Evaluation: introduces CodeWikiBench (hierarchical rubrics from official project docs,
  multi-judge binary scoring, weighted propagation). Scores: Claude Sonnet 4 = 68.79%
  (+4.73% vs DeepWiki baseline 64.06%); Kimi K2 = 64.80%. Big per-language gains for
  TypeScript (+18.54%) and Python (+9.41%).
- Agent-facing?: outputs are for human developers, but the AST/dep-graph + recursive-agent
  architecture is reusable for agent-facing docs.
- Maturity: arXiv 2510.24428 (v1), Oct 2025; "first open-source framework for holistic
  repo-level documentation." Research-grade, not a productized tool.
- Source: https://arxiv.org/html/2510.24428v1 (arXiv paper, accessed 2026-07-19);
  https://fsoft-ai4code.github.io/CodeWiki/ (project page).

---

## Category B — AGENTS.md / CLAUDE.md generators (flat, agent-facing)

### `claude /init` (Claude Code, built-in)

- Generates a starter CLAUDE.md by scanning the codebase (build/test commands, structure,
  conventions). Flat single file, not hierarchical. No drift detection — regenerate/edit
  manually. (Baseline every user already has; adoption = ubiquitous among Claude Code users.)
- Source: referenced across the AGENTS.md-guide corpus, e.g.
  https://www.morphllm.com/agents-md-guide (guide, accessed 2026-07-19).

### agents-md-generator (nyosegawa) — git-clone hook

- Generates a starter AGENTS.md + a CLAUDE.md symlink automatically when you clone an
  _empty_ repo; wraps `git clone` / `ghq get`. Template is minimal by design:
  "20–30 line budget" with placeholder sections and HTML-comment guards.
- Hierarchical: no — flat single file.
- Staleness: runs only at clone; **no refresh mechanism**, manual upkeep after.
- Maturity: 37 stars, 2 forks, 0 releases, ~9 commits, MIT — early/personal.
- Source: https://github.com/nyosegawa/agents-md-generator (repo, accessed 2026-07-19).

### Other generators (lower priority, similar shape)

- **ClaudeForge** (alirezarezvani): CLAUDE.md generator + maintenance tool aligned to
  Anthropic best practices. https://github.com/alirezarezvani/claudeforge
- **Apify "AGENTS.md/CLAUDE.md Generator"** actors: draft a CLAUDE.md for any public repo
  in <30s by auto-detecting stack/commands/conventions.
  https://apify.com/veridian-synthetics/agents-md-generator
- **"agents-md-generator" Claude Code skill**: scans files + interactive Q&A to infer
  agent roles/entry points/deps. https://mcpmarket.com/tools/skills/agents-md-generator
- All flat, all one-shot generation, none with drift detection.
- (Directory listings, accessed 2026-07-19.)

### ⚠ Critical caveat for the crux (auto-generation can HURT)

ETH Zurich SRI Lab, "Evaluating AGENTS.md: Are Repository-Level Context Files Helpful for
Coding Agents?" (2026-02-25). Method: agents = Claude Code Sonnet-4.5, Codex GPT-5.2/5.1
mini, Qwen3-30b-coder; each task run 3× (no file / LLM-generated / human-written).
Benchmarks: SWE-bench Lite + **AGENTbench** (138 instances, 12 Python repos with existing
dev-written context files, avg 641 words / 9.7 sections).

Results (task success vs no-context baseline):

| Condition     | SWE-bench | AGENTbench | Reasoning token cost |
| ------------- | --------- | ---------- | -------------------- |
| No context    | baseline  | baseline   | baseline             |
| LLM-generated | −0.5%     | −2%        | +14–22%              |
| Human-written | +2%       | +4%        | +14–22%              |

Core finding: agents follow instructions faithfully, but **auto-generated files hurt by
restating info already in the repo** (README/code); human files help only by encoding
_non-obvious_ info. Recommendation: omit LLM-generated context files; limit human files to
non-inferable details (custom build/test commands, project-specific constraints).
Directly bears on our crux — a generator that regurgitates code into docs may be net-negative.

- Source: https://academy.dair.ai/blog/agents-md-evaluation (analysis of the paper,
  accessed 2026-07-19); https://www.infoq.com/news/2026/03/agents-context-file-value-review/
  (news, 2026-07-19).

---

## Category C — Doc-drift detection & CI freshness checks

### Swimm — code-coupled continuous documentation (commercial, mature)

- Mechanism: docs embed **code snippets, "Smart Tokens", and "Smart Paths"** that are
  linked to actual source. On code change, Swimm checks these against the latest code,
  identifies affected docs, and offers **Auto-sync** to update them. CI integration flags
  stale docs during review and can **block the PR** ("notify … which documents are
  affected and how").
- Hierarchical: no true hierarchy — organizes via "Playlists" and tags; human-onboarding
  focus, not agent-facing.
- Maturity: established commercial product; markets "Continuous Documentation" as a
  paradigm with customer case studies.
- Relevance: the reference model for **anchor→source coupling with CI enforcement**;
  strongest pattern to adopt conceptually (link doc fragments to code so edits fail CI).
- Source: https://swimm.io/blog/integrating-swimm-and-continuous-documentation-into-your-workflow
  and https://swimm.io/blog/swimm-universal-playlists-as-code-coupling-for-continuous-documentation
  (vendor blog, accessed 2026-07-19).

### embedme (zakhenry) — snippet embedding + CI verify (OSS, adoptable)

- Mechanism: embeds real source files into markdown code fences via a filename comment in
  the fence; supports line-range selection (`file.ts#L20-L30`). **`--verify` flag in CI**
  fails if the embedded snippet no longer matches source (i.e., drift = CI failure).
  Respects .gitignore / .embedmeignore.
- Scope: snippet-level freshness only — not whole-doc generation, not hierarchical.
- Maturity: widely used npm utility; active forks (romnn). MIT.
- Source: https://github.com/zakhenry/embedme and
  https://dev.to/zakhenry/ensuring-accuracy-of-readme-code-snippets-525p (accessed 2026-07-19).

### Siblings/equivalents to embedme (same anchor-embed-verify pattern)

- **embedmd** (campoy, Go): "embed code into markdown and keep everything in sync."
  https://github.com/campoy/embedmd
- **markdown_code_embed** (ippie52, Python): embed source into markdown; "can also be used
  for CI." https://github.com/ippie52/markdown_code_embed
- **snips** (cortesi, Rust): keep code snippets in markdown in sync.
  https://github.com/cortesi/snips
- Pattern across all: doc references a file/range by anchor; a `--check/--verify` mode makes
  CI fail when the referenced code changes but the doc wasn't regenerated. This is the
  closest existing answer to "anchors/checksums so CI fails when referenced code changes."
- (Repos, accessed 2026-07-19.)

### Doc-tests as an adjacent freshness mechanism

- Language-native doctests (Rust `cargo test --doc`, Python `doctest`) execute code
  examples in docs, failing the build when the example diverges from real API behavior.
  Not hierarchical, not agent-facing, but proven CI-enforced drift protection for
  example code. (Well-established; no single URL fetched — flagged as a known pattern.)

---

## Synthesis for the build-vs-adopt decision

- **Hierarchical whole-repo generation**: CodeWiki is the best open blueprint (AST →
  dep-graph → recursive multi-agent → hierarchical assembly). deepwiki-open is the most
  adopted OSS implementation. Neither maintains/refreshes committed docs — the
  hierarchical _generation_ is solved-ish; hierarchical _maintenance_ is open.
- **Drift/CI enforcement**: Swimm (commercial) proves anchor-to-source coupling + PR
  blocking; embedme/embedmd/snips (OSS) provide the same for snippets with a `--verify`
  CI gate. Adopt the anchor-embed-verify pattern; likely build the doc-level (not just
  snippet-level) coupling ourselves.
- **AGENTS.md/CLAUDE.md generators**: all flat, one-shot, no drift handling — thin market.
- **Crux warning**: the ETH Zurich study is the single most decision-relevant finding —
  naive auto-generated context files _reduced_ agent success (−0.5% to −2%) and raised cost
  ~20%, because they duplicated inferable content. Any generator we build must produce
  _non-obvious, non-redundant_ content or it is net-negative. This is evidence AGAINST
  "generate docs from code" and FOR curated, gap-filling, verified-against-code docs.

## Leads for siblings / deeper follow-up

- ETH Zurich AGENTbench paper (full text) — quantifies curated-vs-generated; central to the crux.
- CodeWikiBench rubric methodology — reusable for evaluating our own doc quality.
- Swimm "Smart Tokens / Smart Paths" technical docs — exact anchoring primitives.
- Whether any tool commits hierarchical docs INTO the repo AND drift-checks them in CI
  (none found — apparent whitespace/opportunity for our framework).

## Why any thinness

Not thin on generators or drift tooling. The one genuinely under-served area: a tool that
**both** (a) produces hierarchical, agent-facing docs committed to the repo AND (b) enforces
staleness in CI via anchors/checksums. No single existing tool does both — generators skip
drift (CodeWiki, deepwiki-open, mutable.ai, AGENTS.md generators), and drift tools operate
at snippet granularity without hierarchical generation (embedme et al.) or are commercial +
human-focused (Swimm). That gap is a finding, not a research shortfall.

# Hierarchical agent-facing codebase docs — state of the art & adopt-vs-build

Research run: 2026-07-19 · heavy mode (2 waves, 10 subagents, ~60 sources) · originating workspace archived with the author
Question: for a framework where a root AGENTS.md gives an agent a complete compressed picture that unpacks hierarchically into docs then code — what exists, what's measured, what to adopt vs build?

## Executive summary

The layout you want — compressed always-loaded root, nested detail loaded on demand, code as the leaf — is where the whole ecosystem already converged: it is the shipped loading model of Claude Code, the recommended model in OpenAI Codex docs, the direction of the AGENTS.md v1.1 spec proposal, and unanimous practitioner practice [C3, C4, C12, C19]. **But the strongest measured result of this run cuts against half the premise: a "complete compressed picture" as prose overview does not help coding agents.** The ETH Zurich study (the only controlled A/B of context files) found repo-level context files don't reduce time-to-relevant-file even when developer-written, LLM-generated ones slightly hurt, and the _only_ content that measurably transmits value is specific tooling/build/test instructions; the harm mechanism is redundancy with what the agent can read anyway [C7, C17]. Meanwhile task-relevant _pointers_ (edit locations, correct concise summaries) are worth ~+10–12pp, and wrong/stale ones actively hurt [C6, C10]. So the framework's value lives in three places: **(1) non-derivable knowledge** (commands, conventions, gotchas the code can't tell you), **(2) navigation structure that cuts file-localization failures** — delivered lazily at the point of relevance, never as a front-loaded overview — and **(3) a freshness guarantee**, because an unmaintained map is measurably net-negative, not neutral. The tooling whitespace is real: generation and drift-checking both exist maturely as separate tools, and nothing fuses code-isomorphic hierarchical agent docs with docs-verified-against-code CI [C13]. That fusion — plus the curation bar the evidence demands — is the build.

## 1. The standards layer — adopt AGENTS.md, bridge to CLAUDE.md

- AGENTS.md is the cross-vendor standard: OpenAI-originated (2025-08), donated to the Linux Foundation's Agentic AI Foundation (2025-12-09, alongside MCP), 60k+ repos, ~30 tools [C1]. The spec is deliberately schema-free Markdown with "nearest file wins"; concrete merge semantics live per-harness.
- Claude Code does **not** read AGENTS.md natively; the sanctioned bridge is a CLAUDE.md containing `@AGENTS.md` (or a symlink) [C2] — the Datadog/HN pattern (`echo "Read @AGENTS.md" > CLAUDE.md`) is exactly your prefer-standard-formats answer.
- Loading mechanics diverge in a design-critical way [C3, C4]:
  - **Claude Code:** ancestor CLAUDE.md files load in full at launch; _subdirectory_ files load on demand when files there are read (and are NOT re-injected after compaction); `@`-imports all load at launch — they organize, they don't save context (max 4 hops); path-scoped `.claude/rules/` globs are the native progressive-disclosure primitive.
  - **Codex:** static root→cwd chain only, concatenated, 32 KiB cap, silent truncation beyond; no on-demand deep/sibling discovery (dynamic nesting is an open feature request from Wix/Stripe/Clay-scale customers).
  - ⇒ **Portability requires an explicit router in the root** (a pointer table: "working on X → read path/Y"), not reliance on any harness's lazy loading. The router costs a few lines and works everywhere; lazy loading is a bonus where it exists.
- Spec direction agrees: the v1.1 proposal (open, community, unratified — 5 👍, no maintainer signal) writes down jurisdiction/accumulation/inheritance nesting and an index→inject→load-on-demand progressive-disclosure recommendation with optional `description`/`tags` frontmatter [C19]. Building to this shape is building with the current, not against it.

## 2. The crux — what measured evidence actually supports

Four independent evidence lines triangulate, and they force a sharper thesis than "docs help":

1. **Agentic search beats retrieval for _finding_ code.** On repo localization, BM25/embeddings are near-random (HitFile 0.079) while agentic explorers (Claude Code 0.667) sit in a different tier; Anthropic deleted Claude Code's vector index in May 2025 for this reason [C5]. → Never build an embedding index into this framework.
2. **Static overviews are inert.** The ETH A/B (arXiv 2602.11988, 4 agents × 2 benchmarks): LLM-generated files −0.5% to −3%, human-written +4% average (and not beating _no file_ for Claude Code); context files — even developer-written — did not reduce steps-to-relevant-file; costs rose >20% in all conditions. When all other docs were stripped, LLM-generated files flipped to +2.7% — proving the null is caused by _redundancy_. Only concrete tooling instructions reliably changed behavior (uv used 1.6×/instance when mentioned vs ~0 when not) [C7, C17]. Hedge the authors themselves flag: Python-only; parametric knowledge may be doing the work, so niche stacks may benefit more.
3. **Task-relevant pointers are worth real points.** Oracle edit-location +12pp; correct concise summaries +12pp over no-context — but summaries are "misleading when irrelevant," and missing context is the dominant failure mode (recall beats minimalism) [C6]. Curated skills: +16.2pp average but only +4.5pp in software engineering, with 2–3 focused docs optimal and comprehensive docs _negative_ [C8].
4. **Staleness is measurably harmful, and length alone degrades performance.** Inconsistent comments ~1.52× odds of preceding a bug-introducing commit [C10]; long context hurts even with perfect retrieval (the one measured mitigation, retrieve-then-solve, is exactly what hierarchical unpack implements) [C9]; U-shaped position effects put load-bearing content at the top of any loaded file [C15].

**Verdict:** hierarchy pays through the _navigation-prior and non-derivable-knowledge channels, delivered lazily_ — not through a front-loaded compressed picture. Every token in the always-loaded root must clear the bar: _could the agent derive this by reading the repo?_ If yes, cut it (Anthropic's own `/doctor` trim heuristic encodes the same rule). Efficiency is a second payoff even where success is flat: one (unverified, comment-relayed) study reports ~29% runtime / ~17% token reduction from well-structured AGENTS.md [C21].

## 3. Zoom-level frameworks worth stealing [C14]

Three independent frameworks converge on the same two rules — **doc tree isomorphic to system structure; one altitude/one mode per doc**:

- **C4** (Context → Containers → Components → Code): each level "opens up" into the next; most systems need only L1+L2; **L4 is generated, never hand-maintained** — hand-drawn code-level docs "go stale almost immediately."
- **arc42 §5** building-block view: nested white-box/black-box decomposition — the fractal principle as a template section. Use its 12 sections as an optional menu (context, decomposition, cross-cutting concepts, decisions, glossary are the high-value five), never a mandate.
- **Diátaxis:** one mode per doc; keep generated _reference_ strictly apart from curated _explanation_ — mode-mixing is what makes docs bloat.
- Plus: **ADR** conventions (numbered, immutable, in-repo, superseded-not-deleted — prevents agents re-litigating settled decisions), **Architecture Haiku** (the one-page discipline for the root), **Backstage TechDocs** (production precedent for co-located per-component docs + root manifest).

## 4. Existing tools — the adopt-vs-build map

| Component                             | Verdict                                                                                                                                                                                    | Basis       |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------- |
| Root/nested file convention           | **Adopt** AGENTS.md + nearest-wins + CLAUDE.md `@AGENTS.md` shim                                                                                                                           | C1, C2, C12 |
| Explicit root router table            | **Adopt** pattern (Datadog); it's the portability layer                                                                                                                                    | C4, C12     |
| Progressive disclosure                | **Adopt** Claude Code path-scoped rules + subdir files; mirror v1.1 index shape                                                                                                            | C3, C19     |
| Compressed code map extractor         | **Adopt mechanics** if needed: aider tree-sitter tags + graph ranking + token-budget; repomix `--token-budget` CI guard                                                                    | C5, C6      |
| Embedding/vector index                | **Reject**                                                                                                                                                                                 | C5          |
| Auto-generated prose overviews        | **Reject** — measured net-negative                                                                                                                                                         | C7, C17     |
| Hierarchical doc generation blueprint | **Steal design** from CodeWiki (AST → dep-graph → recursive agents → hierarchical assembly); from hcc (root TOC + per-dir AGENTS.md)                                                       | C13         |
| Drift-check                           | **Build**, stealing patterns: embedme-family `--verify` snippet anchors, `regenerate + git diff --exit-code`, agents-lint-style path/command liveness                                      | C13         |
| Drift primitives (Claude Code)        | **Adopt**: InstructionsLoaded hook (what loaded, why) + Stop hook (propose updates) + claudeMdExcludes                                                                                     | C20         |
| Single-source → multi-vendor compile  | **Watch/steal from** Charter (ADF): `.ai/` modules → compile to CLAUDE.md/AGENTS.md/.cursorrules + `--check` CI gate; 0 stars, vendor-promoted, checks artifact-vs-source not docs-vs-code | C13         |

**The whitespace (adversarially verified):** nothing both generates code-isomorphic hierarchical agent docs in-repo AND verifies docs-against-code in CI. Both halves exist off-the-shelf; the fusion doesn't. But it's _thin_ — assembly is easy, so the durable differentiation is the part the evidence says matters: the **curation bar** (non-derivable content only) and **verified-against-code freshness** (path/command/claim liveness, not just regeneration equality) [C13, C17].

## 5. Design constraints the evidence imposes

1. Root file: router + non-derivable facts only; no prose overview; ≤200 lines (folklore, but converged); load-bearing content at top [C12, C15, C17].
2. Every claim in docs must be _checkable against code_ — paths, commands, symbols as anchors; CI fails on dead anchors [C10, C13].
3. Recall over minimalism in navigation structure — omitting the load-bearing file is the dominant failure; redundant context is tolerated [C6].
4. Detail docs: one altitude/one mode each; reference = generated, explanation = curated; leaf code docs favor runnable examples over prose [C8, C14].
5. Decisions live as ADRs — immutable, numbered, pointed at (not inlined) from the levels they shaped [C14].
6. Depth follows the codebase's real structure — don't force fixed levels (most repos need 2) [C14].
7. Nothing depends on hook/harness magic alone: explicit router works everywhere; lazy loading and hooks are progressive enhancements [C4; your hook-free-fallback rule].

## Where sources disagree

- **U-shape attribution:** Chroma explicitly found _no_ U-shape in its NIAH runs; the U-curve is Liu et al. (TACL 2024), whose magnitudes here are secondary-sourced. Both agree length itself degrades performance [C9, C15].
- **"Does documentation help agents?"** SkillsBench (+16.2pp) vs ETH (≈0). Reconciled by scope: procedural, task-relevant, curated packages help; static repo overviews don't. Domain matters — SE is the weakest-uplift domain measured [C7, C8, C17].
- **ETH internal framing:** intro says −3%/+4% averages; §4.2 says −0.5%/−2% per-dataset. Same data, two framings [C7].

## Limitations

- No study directly tests _hierarchical lazy-loaded_ docs vs flat or vs none — ETH tested static root files only; the lazy-unpack benefit is inferred from retrieve-then-solve (+31%/+4%) and localization-failure evidence, not measured end-to-end. Validating on your own repos is the honest next step [C8 hedge].
- Stale-doc harm is measured on human developers; agent transfer is plausible, not measured [C10].
- C21's numbers (335–535 words optimal; 29%/17% efficiency gains) are relayed by one GitHub comment; papers not fetched.
- ETH per-agent success rates exist only as a bar chart; aggregates are authoritative, cell values are not.
- Negative claims (C2 no-native-AGENTS.md; C13 whitespace) are dated 2026-07-19 and rot by default.
- Refuted en route: "openai/codex has ~88 AGENTS.md files" — the public repo has 2 (verified via git-trees API) [C16].

## Sources (load-bearing)

1. https://agents.md/ — spec
2. https://code.claude.com/docs/en/memory — CLAUDE.md mechanics
3. https://code.claude.com/docs/en/large-codebases — monorepo guidance
4. https://code.claude.com/docs/en/hooks — InstructionsLoaded/Stop
5. https://github.com/agentsmd/agents.md/issues/135 — v1.1 proposal
6. https://arxiv.org/html/2602.11988v1 — ETH context-file A/B (crux)
7. https://arxiv.org/html/2606.07297 — SWE-Explore (agentic ≫ retrieval)
8. https://arxiv.org/abs/2604.07789 — ORACLE-SWE (edit-location +12pp)
9. https://arxiv.org/html/2602.08316 — SWE Context Bench (summaries help/hurt)
10. https://arxiv.org/abs/2602.12670 — SkillsBench (curated +16.2pp, SE +4.5pp)
11. https://arxiv.org/html/2510.05381 — length hurts despite perfect retrieval
12. https://www.trychroma.com/research/context-rot — Chroma context rot
13. https://aclanthology.org/2024.tacl-1.9/ — lost in the middle
14. https://arxiv.org/html/2409.10781 — stale comments → bugs
15. https://arxiv.org/html/2510.24428 — CodeWiki
16. https://c4model.com/ · https://arc42.org/ · https://diataxis.fr/ · https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions
17. https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
18. https://code.claude.com/docs/en/best-practices — include/exclude table
19. https://humanlayer.dev/blog/writing-a-good-claude-md · https://dev.to/datadog-frontend-dev/steering-ai-agents-in-monorepos-with-agentsmd-13g0
20. https://learn.chatgpt.com/docs/agent-configuration/agents-md — Codex loading
21. https://github.com/reyavir/hierarchical-context-compressor · https://github.com/Stackbilt-dev/charter · https://github.com/zakhenry/embedme · https://github.com/openbmb/repoagent
22. https://github.blog/ai-and-ml/github-copilot/how-to-write-a-great-agents-md-lessons-from-over-2500-repositories/ — (editorial)
23. https://searchenginejournal.com — SE Ranking llms.txt relay (2025-11-20)

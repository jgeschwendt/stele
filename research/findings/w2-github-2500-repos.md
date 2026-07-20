# W2 — GitHub's "2,500 repositories" AGENTS.md analysis

**Research wave:** 2 · **Compiled:** 2026-07-19

## TL;DR / why this file is thinner than hoped

The headline source (`github.blog`, Matt Nigh, Nov 2025) is an **editorial best-practices
post, not a study**. Despite the "Lessons from over 2,500 repositories" title, the _only_
quantitative datum in the whole piece is the sample size (2,500+). It reports **no size
distributions, no section-frequency tables, no correlations, and no measured agent
outcomes**. Everything else is asserted advice framed as "what I saw." So the "findings"
section below is short by necessity — there is little measured to extract. The genuinely
load-bearing find is adjacent: **GitHub Copilot's own docs formalize hierarchical /
nested AGENTS.md with a "nearest file wins" precedence rule** — direct product-level
support for this framework's root-router-plus-nested-detail thesis.

---

## Source 1 — the blog post (PRIMARY, but editorial)

- **Title:** "How to write a great agents.md: Lessons from over 2,500 repositories"
- **URL:** https://github.blog/ai-and-ml/github-copilot/how-to-write-a-great-agents-md-lessons-from-over-2500-repositories/
- **Author:** Matt Nigh (@mattnigh), Program Manager Director, GitHub
- **Published:** 2025-11-19 (updated 2025-11-25)
- **Source type:** Vendor blog / practitioner opinion. First-party (GitHub) but **not
  peer-reviewed and not a methods writeup**. Treat as expert editorial, evidence-tier ~
  practitioner-convergence, NOT empirical study.

### Dataset & method (what's actually stated)

- Claim: "I analyzed over 2,500 `agents.md` files across public repos" (stated twice).
- **Selection method: not specified.** No sampling frame, no date range, no repo criteria.
- **Analysis technique: not described.** Framed only as spotting patterns separating files
  that "fail" from files that "work."
- No inter-rater process, no scoring rubric, no code/notebook, no appendix. Unreproducible
  as published.

### Size distributions — **NONE**

The post gives **zero** metrics on line counts, word counts, section counts, or any
median/average. This directly disappoints the Wave-2 question about size distributions.
(Contrast: Wave-1's practitioner convergence on "≤200-line root router" is _not_ corroborated
or contradicted here — the post is silent on length entirely, including no "keep it short"
guidance.)

### Structures that "correlate with quality" — **ASSERTED, NOT MEASURED**

The post presents recommendations as observed patterns but supplies **no supporting
statistics** for any of them. There is no measured correlation between any section and any
outcome. Key claims (all editorial):

- "Most agent files fail because they're too vague."
- "The successful agents aren't just vague helpers; they are specialists."
- Advice: put commands early; prefer one real code snippet over prose; set explicit
  boundaries; pin tech-stack versions; give real output examples.

### The "six core areas" (the closest thing to a taxonomy)

Framed as: hitting these "puts you in the top tier" (again, no quantified validation):

1. **Commands** — executable, with flags/options, in an early section.
2. **Testing** — how to run/scope tests.
3. **Project structure** — file hierarchy and purposes.
4. **Code style** — shown via a real snippet, not described.
5. **Git workflow** — commit/branch conventions.
6. **Boundaries** — what the agent must never do.

Note: the post _also_ presents a starter **template** with slightly different top-level
headings than the "six areas": **Persona · Project knowledge · Tools you can use ·
Standards · Boundaries**, with a three-tier boundary block (✅ Always / ⚠️ Ask first /
🚫 Never). The two lists aren't reconciled in the text.

### Measured outcomes vs prevalence — **NO OUTCOMES AT ALL**

- No agent success rate, task-completion rate, or with/without comparison anywhere.
- The one frequency-style statement — "never commit secrets" was the most common helpful
  constraint — is **prevalence, not efficacy**. It says nothing about whether including it
  improves agent behavior. Flagging explicitly because the Wave-2 brief asks to separate
  the two: **this post has measured prevalence of exactly one item, and measured outcomes
  of zero items.**

### Hierarchy / nesting / drift — **SILENT**

The post does **not** discuss nested AGENTS.md, monorepos, multiple files, hierarchy,
staleness/drift, or the AGENTS.md open spec. So it neither supports nor undercuts the
framework's hierarchical-unpacking + CI-drift-checking thesis. (That thesis gets its
support from Source 2, not here.)

### Concrete recommendations (editorial, verbatim-ish)

- Place executable commands early, "with flags and options, not just tool names."
- "One real code snippet showing your style beats three paragraphs describing it."
- Three-tier boundaries (Always / Ask first / Never).
- Specify tech stack **with versions**.
- Provide real **output** examples.
- Iterate: "Start simple. Test it. Add detail when your agent makes mistakes." — "The best
  agent files grow through iteration, not upfront planning."

---

## Source 2 — GitHub Copilot docs formalize hierarchy (the useful part)

This is where the framework's core structural bet gets first-party product backing.

**2a. Repository custom instructions doc**

- URL: https://docs.github.com/en/copilot/how-tos/configure-custom-instructions-in-your-ide/add-repository-instructions-in-your-ide
- Source type: first-party product documentation (authoritative for Copilot behavior).
- Verbatim rules:
  - "You can create one or more `AGENTS.md` files, stored anywhere within the repository."
  - "When Copilot is working, the **nearest `AGENTS.md` file in the directory tree will
    take precedence**." ← nearest-wins, i.e. hierarchical override.
  - Support for files outside the workspace root is **disabled by default** (VS Code
    setting to enable).
  - Does **not** define a general precedence order _between_ AGENTS.md vs
    `.github/copilot-instructions.md` vs CLAUDE.md; duplicates of identical instruction
    files are de-duplicated and remaining ones are **combined**.

**2b. `@`-path file includes (from Copilot CLI docs)**

- URL: https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions
- "In `.github/copilot-instructions.md`, `AGENTS.md`, or `CLAUDE.md`, you can use `@`
  followed by a relative path to include another file." ← native mechanism for a root
  file to point into detail docs. This is exactly the "root compressed picture → unpack
  into detail docs" move, supported at the tool level.
- Copilot CLI discovers instruction files at: repo root, current working directory,
  intermediate directories, and directories nested along the path of a file it's editing.

**2c. Changelog — when coding agent gained AGENTS.md**

- URL: https://github.blog/changelog/2025-08-28-copilot-coding-agent-now-supports-agents-md-custom-instructions/
- Date: **2025-08-28**.
- "You can create a single `AGENTS.md` file in the root of your repository. You can also
  create nested `AGENTS.md` files which apply to specific parts of your project."
- Note: the changelog states nesting exists but does **not** itself spell out the
  nearest-wins precedence; the IDE docs (2a) do.

### Live feature-gap signals (open Copilot issues — leads, not conclusions)

- github/copilot-cli #1655 — "Flag for nested agents.md files along file hierarchy."
- github/copilot-cli #3051 — "Recursively discover AGENTS.md in subfolders, like VS Code's
  `chat.useNestedAgentsMdFiles`."
  These suggest nested discovery is **partially implemented / inconsistent across Copilot
  surfaces** (VS Code has `chat.useNestedAgentsMdFiles`; CLI users are still asking for it).
  Relevant to the framework: hierarchical AGENTS.md is endorsed in docs but **not uniformly
  enforced by tooling yet** — a real adoption gap, not a solved problem.

---

## How this bears on the framework (Wave-2 synthesis)

- **Supports hierarchy thesis:** Copilot docs bless "one or more" nested AGENTS.md with
  nearest-wins precedence, and `@path` includes — i.e., a root file _unpacking_ into detail
  docs is a first-party-supported pattern, not just practitioner folklore.
- **Does NOT support (or refute) the size/length thesis:** the blog post is silent on
  length; the "≤200-line root router" convergence from Wave 1 is not corroborated here.
- **Does NOT touch drift/CI-checking:** neither source addresses staleness. The Wave-1
  finding that "stale/wrong docs actively hurt" gets no reinforcement from GitHub — and
  the "no tool does hierarchical gen + CI drift-check together" gap stands; GitHub tooling
  does the hierarchy half, nothing on the drift half.
- **Evidence-quality caution:** do **not** cite the "2,500 repositories" post as empirical
  evidence for what correlates with agent success. It is expert editorial with N stated but
  no measurements. Its value is as (a) a well-known section taxonomy and (b) a signal of
  GitHub's house style, not as data. If the framework needs measured correlations, this
  is not that source — look instead to academic work (e.g. the ETH Zurich line still
  needing a primary pull) or to controlled internal eval.

## Open leads for later waves

- ETH Zurich "LLM-generated context files hurt / human-written help" — still secondary,
  needs primary pull (carried from Wave 1).
- VS Code `chat.useNestedAgentsMdFiles` setting — the concrete nested-discovery toggle;
  worth reading its docs for exact resolution order.
- AGENTS.md open spec (agents.md / agentsmd) — the blog doesn't reference it; verify
  whether a formal cross-vendor spec exists and what it says about hierarchy.
- Copilot CLI issues #1655, #3051 — track for when/if uniform nested discovery ships.

# W2 — ETH Zurich AGENTS.md study (PRIMARY SOURCE)

**Paper:** "Evaluating AGENTS.md: Are Repository-Level Context Files Helpful for Coding Agents?"
**Authors:** Thibaud Gloaguen, Niels Mündler, Mark Müller, Veselin Raychev, Martin Vechev (ETH Zurich SRI Lab + LogicStar.ai)
**arXiv:** 2602.11988v1 · submitted ~2026-02 · venue tag "ICML"
**Primary URLs fetched:**

- HTML body (full): https://arxiv.org/html/2602.11988v1 (fetched 2026-07-19)
- Abstract: https://arxiv.org/abs/2602.11988
- SRI Lab landing: https://www.sri.inf.ethz.ch/publications/gloaguen2026agentsmd

**Source type:** peer-style preprint (primary). Fetched the full HTML body, not just abstract. All numbers below are from the paper text/tables unless flagged as a Figure-3 bar-chart estimate.

---

## Headline result (one sentence)

Context files do NOT generally improve task success; they raise inference cost >20%. Human-written files give a marginal gain, LLM-generated files a marginal loss — both across every LLM and agent tested.

Verbatim (abstract): "context files tend to reduce task success rates compared to providing no repository context, while also increasing inference cost by over 20%." Conclusion clause: "unnecessary requirements from context files make tasks harder, and human-written context files should describe only minimal requirements."

---

## 1. Benchmark construction — AGENTbench

**Final size:** 138 instances, from 12 Python repos, distilled from **5,694 PRs**. Constructed using GPT-5.2 + Codex as the builder agent. Covers **both bug-fixing and feature-addition** tasks. All 12 repos have developer-committed context files (AGENTS.md or CLAUDE.md at root).

**Why niche repos:** Context files were only formalized Aug 2025, so only recently/niche repos have developer-committed ones; popular benchmark repos (SWE-bench) have none. AGENTbench is the complement to SWE-bench Lite (the latter used for the LLM-generated condition on popular repos).

**Five-stage pipeline:**

1. **Finding repos** — GitHub search for repos with a root context file; filter to (a) main language = Python, (b) has a test suite, (c) **≥400 PRs** (ensures ≥10 instances survive post-processing).
2. **Filtering PRs** — keep PRs that (a) reference ≥1 issue AND (b) modify ≥1 Python file; then an LLM agent keeps only PRs judged to introduce "deterministic, testable behaviors." Unlike SWE-bench Lite, they do NOT require the PR to edit unit tests (niche repos rarely include them).
3. **Environment setup** — an agent writes a script to set up env, run the suite, and emit machine-readable results; keep only PRs with ≥1 passing test → **87% of filtered instances** survive.
4. **Task descriptions** — a third LLM agent standardizes each task into 6 sections (description, steps to reproduce, expected behavior, observed behavior, specification, additional information). Instructed NOT to leak the solution. Manual inspection of **10% random sample → zero solution leaks**.
5. **Generating unit tests** — LLM generates regression tests from task desc + PR-modified test files + golden patch X* + base repo R. Verified to FAIL on R and PASS on R∘X*. Over-specified tests manually pruned. Final test set = generated tests ⊎ subset of existing repo tests that pass on patched code. **Average coverage 75% of modified code** (min 2.5%, max 100%).

**Decontamination:** git history + remotes stripped from Docker envs; web access monitored; manual inspection found no cheating.

**Table 1 key stats (mean / min / max across 138):**

- PR body words: 415.3 / 5 / 4961
- Issue words: 211.6 / 96 / 500
- Codebase files: 3337 / 151 / 26602
- PR patch lines edited: 118.9 / 12 / 1973; files edited: 2.5 / 1 / 23
- Test coverage: 75% / 2.5% / 100%
- **Context file words: 641.0 / 24 / 2003; sections: 9.7 / 1 / 29** (section = content between markdown headers)

**Comparators:** SWE-bench Lite = 300 tasks, 11 popular Python repos, NONE with developer context files (skews toward Django). Used only for None-vs-LLM comparison.

---

## 2. Agents & conditions

**Four agent+model pairs:**

- Claude Code + Sonnet-4.5 (temp 0, default settings)
- Codex + GPT-5.2 (temp 0)
- Codex + GPT-5.1 mini (temp 0)
- Qwen Code + Qwen3-30b-coder (temp 0.7, top-p 0.8; local vLLM; chat compression at 60% of 256K ctx; shell output capped 2000 tokens)

Single sample per agent (no repeats). Context file written to CLAUDE.md (Claude Code) or AGENTS.md (Codex, Qwen Code).

**Three settings:** None (context file removed) · LLM (generated via each agent's recommended init command+model on pre-patch repo) · Human (developer-committed file; AGENTbench only).

---

## 3. The deltas (exact where the paper states them)

### IMPORTANT caveat on per-agent, per-condition success rates

The **per-cell resolution rates are published ONLY as a bar chart (Figure 3)** — they are NOT tabulated anywhere in the text, tables, or appendix (confirmed by scanning the full HTML: only a handful of percentages appear in prose). So exact per-agent success percentages for None/LLM/Human are not recoverable from the text; only the aggregate deltas and qualitative orderings below are authoritative.

### Authoritative aggregate deltas (from prose)

- **LLM-generated context files:** cause performance drops in **5 of 8 settings** (4 agents × 2 datasets). Average resolution-rate reduction = **−0.5% on SWE-bench Lite**, **−2% on AGENTbench**. Intro summarizes both as "**a decrease of 3% on average**." (Note the intro's −3% vs Section 4.2's −0.5%/−2% are the paper's own two framings; treat −0.5%/−2% as the per-dataset figures and −3% as the cross-dataset headline — a mild internal inconsistency worth flagging.)
- **Human-written context files (AGENTbench only):** "**an increase of 4% on average**" vs None. Ordering: human **outperforms LLM for all four agents**; human **beats None for all agents EXCEPT Claude Code** (Claude Code is the sole agent where even human files don't beat no-file). Human files are "not agent-specific."

### Qualitative per-agent notes the paper does give

- **Claude Code / Sonnet-4.5:** the one agent where human files do NOT beat None. 100% of its LLM-generated files were flagged as containing a codebase overview.
- **GPT-5.1 mini:** anomalous step inflation — it re-issues commands to find and re-read context files already in its context (only when a context file is present). Only 36% of its generated files contained overviews (vs 95–100% for others).
- **GPT-5.2:** 99% of generated files had overviews.
- **Qwen3-30b-coder:** 95% of generated files had overviews; cost estimated from average OpenRouter price (only estimated, not metered).

### Bar-chart ESTIMATES (Figure 3 — LOW confidence, read by a summarizer, NOT verified against the image; do not cite as exact)

AGENTbench None / LLM / Human ≈ Sonnet-4.5 57/54/56 · GPT-5.2 34/32/35 · GPT-5.1mini 15/14/16 · Qwen 22/21/23. SWE-bench Lite None/LLM ≈ Sonnet 61/60 · GPT-5.2 33/32 · GPT-5.1mini 20/19 · Qwen 24/23. **These are approximations only** — the true values require reading the Figure 3 image.

---

## 4. Token / cost figures (Table 2 — EXACT, verbatim)

Table 2 = avg **steps** and **execution cost (USD)** per instance. (A "step" = one env interaction: a shell call or file modification.)

| Dataset        | Setting | Sonnet-4.5 steps / $ | GPT-5.2 steps / $ | GPT-5.1 mini steps / $ | Qwen3-30B steps / $ |
| -------------- | ------- | -------------------- | ----------------- | ---------------------- | ------------------- |
| SWE-bench Lite | None    | 54.4 / 1.30          | 12.5 / 0.32       | 40.9 / 0.18            | 29.7 / 0.12         |
| SWE-bench Lite | LLM     | 57.2 / 1.51          | 12.7 / 0.43       | 45.2 / 0.22            | 32.2 / 0.13         |
| AGENTbench     | None    | 40.7 / 1.15          | 12.1 / 0.38       | 40.6 / 0.18            | 31.5 / 0.13         |
| AGENTbench     | LLM     | 46.5 / 1.33          | 13.1 / 0.57       | 46.9 / 0.20            | 34.2 / 0.15         |
| AGENTbench     | Human   | 45.3 / 1.30          | 13.6 / 0.54       | 46.6 / 0.19            | 32.8 / 0.15         |

**Aggregate cost/step deltas (prose):**

- LLM files add **+2.45 steps (SWE-bench Lite)** and **+3.92 steps (AGENTbench)** on average → **cost +20% (SWE-bench Lite), +23% (AGENTbench)**.
- Human files add **+3.34 steps** on average → cost up **at most 19%**.
- **Reasoning tokens** (GPT models, adaptive reasoning): LLM files +22% (GPT-5.2) / +14% (GPT-5.1 mini) on SWE-bench Lite; +14% / +10% on AGENTbench. Human files +20% (GPT-5.2) / +2% (GPT-5.1 mini).

---

## 5. What distinguished HELPFUL human content from HARMFUL generated content (content analysis)

This is the core of Wave 2's question. The paper's mechanism findings:

**A. LLM-generated files are redundant with existing docs; human files add net-new info.**
Test: manually delete ALL documentation (`*.md`, example code, `docs/`) AFTER generating the context file, then evaluate (Fig 5; Claude Code excluded for cost). In this docs-stripped setting, **LLM-generated files improve performance by +2.7% on average AND outperform the human files.** Interpretation: when other docs exist, LLM files just duplicate them (no marginal value); their apparent benefit in anecdotes comes from repos that have little/no other documentation. Human files help because they carry information not already in the README/docs.

**B. Overviews don't work — for either kind.** Recommended practice is to include a codebase overview. 8/12 human files have a dedicated overview (4 enumerate directories); LLM files almost always do (100% Sonnet, 99% GPT-5.2, 95% Qwen, 36% GPT-5.1 mini). But measuring "steps before the agent first touches a file that the golden patch modifies" (excluding the 3% where it never touches one), context files — even human ones — do NOT reduce time-to-relevant-file (Fig 4). Conclusion verbatim: "context files, even developer-provided ones, are not effective at providing a repository overview."

**C. The one thing that DOES transmit: specific tooling instructions.** Instructions are followed reliably. `uv` is used **1.6×/instance when mentioned vs <0.01× when not**; repo-specific tools **2.5×/instance when mentioned vs <0.05× when not.** Context files also increase testing, grep/read/write file traversal (Fig 6). So the failure is NOT instruction-following.

**D. Why following instructions still doesn't help — the harmful mechanism.** "unnecessary requirements from context files make tasks harder." Extra (often unnecessary) requirements induce more exploration + more reasoning tokens (adaptive-reasoning models spend more, i.e., they perceive the task as harder), inflating cost without raising success. The actionable prescription: **human files should carry ONLY minimal requirements — e.g., the specific tooling to use — not overviews, not exhaustive documentation.**

**Practical takeaway for the framework:** a root router that (a) states repo-specific build/test/tooling commands (transmits reliably, high value) and (b) resists dumping a codebase overview or duplicating README/docs prose (redundant at best, cost-inflating requirement-noise at worst). "Compressed complete picture" is exactly what this study shows agents do NOT benefit from as an overview — the win is minimal, non-redundant, tooling-specific pointers.

---

## 6. Ablations (robustness)

- **Stronger generator model doesn't help:** generating files with GPT-5.2+Codex vs each agent's own model → +2% avg on SWE-bench Lite but **−3% avg on AGENTbench** (Fig 8). "Stronger models do not necessarily generate superior context files."
- **Prompt choice barely matters:** Codex vs Claude Code generation prompts show no consistent winner (Fig 9). "sensitivity to different (good) prompts is generally small."

---

## 7. Authors' hedges / limitations / threats to validity

- **Python-only / parametric-knowledge confound (biggest hedge):** "much detailed knowledge about tooling, dependencies... might be present in the models' parametric knowledge, nullifying the effect of context files." Effect could be larger for niche languages/toolchains under-represented in training data. (Directly relevant: their null/negative result may UNDERSTATE context-file value for less-common stacks.)
- **Only measures task-resolution rate** — not code efficiency or security, which context files might help; explicitly future work.
- **Improving generation is open** — humans currently dominate; planning / continuous-learning methods might close the gap.
- **Niche repos have laxer PR rules** (they say this shapes benchmark construction, hence the LLM test-generation step + manual pruning).
- **LLM-heavy pipeline:** task descriptions LLM-standardized (only 10% manually inspected), tests LLM-generated (75% avg coverage), instance filtering by LLM agent — all potential quality/bias vectors.
- **Single sample per agent** (temp 0 for 3 agents; Qwen at 0.7) — no variance estimate.
- **Qwen cost is estimated**, not metered (OpenRouter avg price).
- **Internal number inconsistency to flag:** intro says LLM −3% avg / human +4% avg; Section 4.2 says LLM −0.5% (SWE) / −2% (AGENTbench). Reconcile as headline-vs-per-dataset framings.

---

## 8. Reconciliation with Wave 1

- Wave 1's "LLM-generated context HURTS, human-written HELPS" — **CONFIRMED from primary source**, but both effects are MARGINAL (human +4% avg / not even beating None for Claude Code; LLM −0.5% to −3%), not large. The secondary blogs' "human +4% / LLM −2%" framing matches Section 4.2 (AGENTbench).
- Wave 1's "stale/wrong docs actively hurt" — this paper's mechanism is subtler: it's not staleness but **redundancy + unnecessary requirements** that hurt; and overviews simply don't work regardless of correctness.
- Supports Wave 1's "root router ≤200 lines, minimal, on-demand detail": the paper's explicit prescription is minimal-requirements-only, tooling-specific; overviews and completeness are shown NOT to pay off as agent context.
- **Tension with the framework's premise:** the framework bets on a root AGENTS.md giving "a compressed COMPLETE picture." This paper is direct counter-evidence that completeness/overview content is inert-to-harmful for coding agents on Python task resolution. The defensible surviving value is (a) tooling/build/test commands and (b) non-redundant info absent from README/docs — plus the untested-here hypotheses (niche languages, hierarchical on-demand unpacking, drift-checking) the paper does not evaluate.

---

## Notes on completeness of this file

This file is NOT thin. Full HTML body retrieved and parsed. The ONE gap: exact per-agent per-condition success percentages are published only as a bar chart (Figure 3) and are unrecoverable from text (verified by scanning all percentages in the HTML — only aggregate deltas appear in prose). Aggregate deltas, all cost/step/token numbers (Table 2), the full content analysis, ablations, and limitations are captured verbatim/exact.

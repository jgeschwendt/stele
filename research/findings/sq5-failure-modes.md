# SQ5 — Known failure modes of documentation-for-agents & measured mitigations

Scope: failure modes + mitigation _evidence_ only. Specs/tooling/frameworks belong to sibling sub-questions. Compiled 2026-07-19.

Not thin — five of the six requested threads have primary or strong sources; the CLAUDE.md-size thread is thin _by nature_ (only blog heuristics exist, no rigorous study — flagged inline).

## Queries run

1. Chroma context rot report long context degradation U-shaped position retrieval distractor
2. llms.txt no effect AI crawler behavior study 300000 domains measurement
3. "lost in the middle" Liu et al long context position language models findings
4. outdated code comments documentation mislead developers study empirical harm stale comments
5. instruction following degradation longer prompt context length LLM adherence 2025 study many instructions
6. Liu lost in the middle GPT-3.5 20 documents accuracy drops 20 percentage points middle position
7. CLAUDE.md system prompt size instruction adherence agent too long context bloat 2026

---

## 1. Context rot / long-context degradation — Chroma (PRIMARY FETCHED)

- **Source:** trychroma.com/research/context-rot — Chroma Research report. Type: vendor research report (industry lab, methodology disclosed). **Date: 2025-07-14.**
- **Scope:** 18 frontier LLMs (Claude Opus 4 / Sonnet 4 / 3.7 / 3.5 / Haiku 3.5; OpenAI o3, GPT-4.1 + mini/nano, GPT-4o, GPT-4 Turbo, GPT-3.5 Turbo; Gemini 2.5 Pro/Flash, 2.0 Flash; Qwen3-235B/32B/8B). Controlled NIAH variants, input lengths from ~25 up to 1K–10K tokens (Qwen extended to 131,072 via YaRN).
- **Core finding:** performance _consistently degrades with increasing input length_ across every model and every task — no model holds flat across its advertised window. Held task complexity constant to isolate the length variable.
- **Position effect (IMPORTANT nuance — contradicts the framing in the sub-question prompt):** Chroma explicitly did **NOT** find a U-shape in its primary NIAH task — "no notable variation in performance" across 11 needle positions. The U-shape is Liu et al.'s finding (§3), not Chroma's. Chroma _did_ see position sensitivity in the repeated-words task: accuracy highest when the unique word sits near the beginning, worsening with length.
- **Distractor cost (the near-duplicate/semantic-distractor finding):** distractors were "topically related to the needle but do not quite answer the question." A single distractor measurably lowered accuracy vs. needle-only baseline; four distractors compounded the drop. Impact was **non-uniform** — some distractors (e.g., "distractor 3") hurt far more than others, and distractors 2 & 3 appeared most often in hallucinated answers. Claude family had lowest hallucination rates; GPT family highest.
- **Haystack-structure finding (counterintuitive, relevant to doc layout):** models perform _worse_ when the haystack preserves logical flow; shuffling sentences to remove local coherence _improved_ performance across all 18 models. Hedge worth carrying: this is a NIAH artifact and may not transfer to real curated docs, but it complicates any assumption that "coherent prose helps retrieval."
- **Authors' own hedge:** real-world tasks are more complex than these controlled probes, so "the influence of input length may be even more pronounced in practice."
- **Mitigation implied (not a controlled A/B):** minimize tokens in context; don't rely on advertised window size. Directly motivates _compression + hierarchical unpack_ over dumping everything into a root file.

## 2. Context length hurts _even with perfect retrieval_ — EMNLP 2025 Findings (PRIMARY FETCHED)

- **Source:** arxiv.org/html/2510.05381v1 "Context Length Alone Hurts LLM Performance Despite Perfect Retrieval." Type: peer-reviewed (EMNLP 2025 Findings) preprint. **Date: Oct 2025.**
- **Scope:** 5 models (Llama-3.1-8B, Mistral-v0.3-7B, GPT-4o, Claude-3.5, Gemini-2.0), 4 task types (GSM8K math, MMLU QA, HumanEval coding, VarSum). Synthetic long-context: evidence + question separated by distraction padding; retrieval verified by exact-match _before_ scoring the task.
- **Core finding:** degradation is **not** explained by retrieval failure or distractors. Llama-3.1 dropped 24.2% on MMLU _while exactly retrieving 970/1000 problems_. VarSum: 59% drop (Llama), 44% (Mistral) with minimal retrieval loss. Claude-3.5 dropped 67.6% on MMLU at 30K tokens. Even with all distraction masked, open models still degraded 7.9%+.
- **Measured mitigation:** "retrieve-then-solve" — model recites the relevant evidence before answering, converting a long-context task into a short one. Gains: Mistral +31.2% on synthetic benchmark; GPT-4o +4% on RULER. This is the strongest _measured_ mitigation in the corpus and argues for docs that let the agent extract-then-act rather than reason in-place over a large loaded context.
- **Scope limit:** 5 models, some closed-model combos missing due to refusal behaviors.

## 3. Lost in the middle — Liu et al. (PRIMARY; PDF binary, numbers via ACL abstract + secondary)

- **Source:** aclanthology.org/2024.tacl-1.9 (TACL 2024; arXiv 2307.03172, 2023). PDF at cs.stanford.edu/~nfliu/papers/lost-in-the-middle.arxiv2023.pdf could not be parsed (binary/FlateDecode) — numbers below are from the ACL abstract (fetched) plus corroborating secondary summaries; treat the exact percentages as secondary-sourced, the shape as primary.
- **Scope:** multi-document QA (NaturalQuestions-Open) + key-value retrieval; relevant doc placed at positions 1/5/10/15/20 among distractor docs (10, 20, 30 doc settings). Models: GPT-3.5-Turbo, GPT-4, Claude, several open models.
- **Core finding (PRIMARY, from abstract):** performance is highest when relevant info is at the **beginning or end**, and "significantly degrades" in the middle — a **U-shaped curve**. Holds "even for explicitly long-context models."
- **Magnitude (SECONDARY, verify before quoting as fact):** ~20–30 point drop from ends to middle; in the 20-doc setup roughly ~75% (pos 1) / ~72% (pos 20) vs. ~55% (pos 10). These specific figures come from secondary summaries, not the parsed paper.
- **Relevance:** the canonical evidence that _placement within a loaded doc matters_ — high-value content belongs at the top/bottom of any always-loaded file, not buried mid-document.

## 4. llms.txt findability — the ~300k-domain measurement (PRIMARY publisher identified)

- **Study author:** **SE Ranking** (SEO tooling vendor). Reported via searchenginejournal.com (fetched). Type: industry measurement study. **Article date 2025-11-20**; primary is the SE Ranking blog (lead to fetch directly — see leads).
- **Exact scope:** ~300,000 domains analyzed. Only **10.13%** had an llms.txt file. Measured = _domain-level AI citation frequency_ across major LLM responses, via correlation tests + an XGBoost model.
- **Finding:** no meaningful relationship between having llms.txt and AI citation rate. Notably, **removing llms.txt as a feature improved the model's predictive accuracy** — i.e., the file carried no useful signal.
- **Hedge (theirs, verbatim-ish, <15 words):** it doesn't directly impact citation frequency "at least not yet."
- **Corroborating crawler-behavior measurements (secondary, different studies):** OtterlyAI — only **0.1%** of AI-crawler requests touched /llms.txt over 90 days; another measurement found only **408** hits to llms.txt across 500M+ AI-bot visits in 90 days. GPTBot/ClaudeBot/PerplexityBot/OAI-SearchBot/Google-Extended overwhelmingly crawl HTML directly. Google (June 2026 docs) states llms.txt has zero effect on Search rankings / AI Overviews.
- **Caveat for our framework:** llms.txt is a _web-crawler discoverability_ convention; its null result is about external AI answer engines, **not** about a coding agent reading a repo it already has on disk. Do not over-transfer — but it is direct evidence that "publish a curated pointer file and assume agents will read it" is unvalidated at best.

## 5. Stale-documentation / stale-comment harm (PRIMARY FETCHED)

- **Source:** arxiv.org/html/2409.10781v1 "Investigating the Impact of Code Comment Inconsistency on Bug Introducing." Type: peer-reviewed empirical SE study. **Date: 2024.**
- **Scope:** 8 Apache Java projects (of 32; cost-limited). GPT-3.5 used to detect comment/code inconsistency; odds-ratio survival analysis of subsequent bug-introducing commits.
- **Finding (measured effect size):** inconsistent (stale) comments are **~1.52×** more likely to precede a bug-introducing commit within a **7-day** window; effect decays to **1.14×** at 14 days. Recent inconsistency matters more than older. 7-day exposed group: 2,710 with-event / 2,342 without; non-exposed 36,672 / 44,907.
- **Corroborating (secondary):** iComment study (Linux, Mozilla, Wine, Apache) surfaced real inconsistencies devs agreed could mislead; 2 Mozilla cases where stale comments caused later-reported bugs. Springer study of 3,000+ GitHub projects: most contain ≥1 outdated code-element reference at some point.
- **Relevance to crux:** this is the strongest evidence that documentation can be **net-negative** — a stale doc actively misleads and correlates with bugs. It is the empirical case _for_ drift-check tooling and _against_ docs that aren't kept in lockstep with code. Note: measured on human developers + comments, not AI agents specifically — transfer is plausible (agents also trust prose) but not directly measured (a gap).

## 6. Instruction-following decay as context grows (SECONDARY-heavy; one weak-primary thread)

- **Multi-turn decay (secondary summaries of primary papers):**
  - Laban et al. 2025: LLMs ~39% worse and ~112% less reliable in multi-turn vs. equivalent single-turn ("Lost in Conversation").
  - He et al. Multi-IF: adherence to earlier-turn instructions degrades monotonically with turn count; o1-preview 88%→71% from turn 1 to turn 3.
  - Jia et al. EvolIF: strongest 2025 frontier models sustain only ~18 reliable conversational turns.
  - These are turn-count effects, adjacent to (not identical to) "always-loaded context size vs. compliance." Fetch primaries if this thread becomes load-bearing (leads).
- **CLAUDE.md / system-prompt size vs. compliance (THIN — heuristics only, NO rigorous study found):**
  - tianpan.co (2026-02-14) and similar blogs claim: keep under ~200 lines / ~300-350 words median for well-performing files; >1,000 words negatively correlates with performance; frontier models "reliably follow ~150-200 instructions, Claude Code already uses ~50." **On fetch, these specific numbers are unsourced author heuristics** — the author's only cited empirical source is GitHub's "2,500 repositories AGENTS.md" analysis (structure, not a size-vs-compliance experiment; belongs to sibling scope). Treat the 150-200-instruction and word-count thresholds as folklore, not measured.
  - Recommended mitigation across these blogs: split oversized root file into path-scoped `.claude/rules/*` loaded only when working matching files — i.e., hierarchical/lazy loading. This is consistent with §1–2 evidence but is not itself independently measured.

---

## Bearing on the crux (does curated hierarchical doc beat on-demand grep/read?)

No source in this sub-question runs that head-to-head A/B directly (that's the central gap for a sibling to close). But the failure-mode evidence constrains the design:

- Context rot (§1) + length-hurts-even-with-perfect-retrieval (§2) → **against** loading large docs into context; **for** compression + retrieve-then-act. Favors a small root file that unpacks on demand over a big always-loaded one — and partially favors on-demand exploration (small extracted context) over front-loading everything.
- Retrieve-then-solve (§2) is the one **measured** mitigation (+31% / +4%) and maps cleanly to hierarchical unpack.
- Lost-in-the-middle (§3) → high-value content at top/bottom of any loaded file.
- Stale-doc harm (§5) → curated docs are net-negative when they drift; drift-check tooling is not optional if we ship docs.
- llms.txt null result (§4) → a curated pointer file is not automatically consumed; don't assume adoption without enforcement (for on-disk repo agents this is weaker, but the discipline lesson holds).

## Leads (cited-but-unfetched / worth chasing)

- SE Ranking original blog post on the 300k-domain llms.txt study (primary; SEJ is the secondary relay) — get exact N, model list, and statistical method.
- OtterlyAI GEO study (otterly.ai/blog/the-llms-txt-experiment) — 0.1% crawler-touch figure primary.
- Liu et al. lost-in-the-middle full PDF (arXiv 2307.03172) — parse for exact per-position accuracy tables (our magnitudes are secondary).
- Laban et al. "LLMs Get Lost in Multi-Turn Conversation" (2025); He et al. Multi-IF; Jia et al. EvolIF — primaries for instruction-decay-vs-length.
- GitHub blog "how to write a great AGENTS.md — lessons from over 2,500 repositories" — the one real empirical AGENTS.md-structure study (sibling scope, but the only measured doc-structure dataset surfaced).
- EMNLP 2025 "Context Length Alone Hurts..." — solid primary already fetched; cite RULER + retrieve-then-solve numbers.
- Terms of art: NIAH (needle-in-a-haystack), "context rot," "retrieve-then-solve," odds-ratio bug-introducing-commit, U-shaped positional bias / primacy-recency.

# SQ2 — Code-map / repo-summarization tooling & MEASURED evidence on context strategy

Research date: 2026-07-19. Scope: tooling (aider repo-map, repomix, ctags/tree-sitter, agentic search) + measured evidence comparing curated structural maps vs on-demand agentic exploration vs embeddings retrieval. Doc frameworks / AGENTS.md spec / drift modes are OUT (sibling agents).

## Queries run

1. `aider repository map tree-sitter graph ranking pagerank token budget`
2. `SWE-bench context retrieval ablation curated map vs agentic exploration embeddings`
3. `Agentless SWE-bench hierarchical repo structure localization vs agentic tool use results`
4. `Anthropic Claude Code agentic search vs indexing RAG codebase engineering blog`
5. `repomix ctags repo map coding agent token budget comparison`

Pages fetched in full: aider repomap blog, SWE-Explore (arXiv 2606.07297) HTML, SWE Context Bench (arXiv 2602.08316) HTML, Claude.com large-codebases blog, ORACLE-SWE (arXiv 2604.07789) abstract + HTML results, Anthropic effective-context-engineering post. Backward hop: SWE-Explore downstream-validation → ORACLE-SWE oracle-signal ablation.

---

## THE CRUX (short answer)

The evidence splits cleanly and does NOT support "a pre-built map beats exploration" as a blanket claim. It supports a sharper, more useful conclusion:

1. **For FINDING code, on-demand agentic exploration (grep/read/glob) beats both embeddings retrieval and lexical retrieval — measurably and by a large margin.** Static pre-indexed retrieval (vector or BM25) is near-random on repo localization; every agentic explorer sits in a clear tier above it. (SWE-Explore; Anthropic.)

2. **BUT giving the agent curated, correct, compact pointers/summaries measurably lifts task success** — an oracle "edit location" signal is worth ~+12pp resolve rate, and correctly-selected concise summaries beat raw retrieval by ~+12pp. The win comes from _curation quality and compactness_, not from _pre-indexing as a mechanism_. (ORACLE-SWE; SWE Context Bench.)

The reconciliation: a hierarchical curated doc is NOT an embedding index. It is closer to the "oracle edit-location / curated-summary" signal that DOES help. The failure mode a map must avoid is being stale or wrong — a misleading summary hurts as much as a right one helps ("misleading when irrelevant"). So the framework's value proposition is real but conditional on freshness/correctness, which is exactly where drift-check tooling earns its keep.

---

## Tooling findings

### Aider tree-sitter repo-map

- Source: `https://aider.chat/2023/10/22/repomap.html` (blog, 2023-10-22, primary) + `https://aider.chat/docs/repomap.html` (docs) + DeepWiki `Aider-AI/aider/4.1`.
- Mechanism: tree-sitter parses each file to an AST; language `.scm` tag queries extract `def` (definitions) and `ref` (references). Builds a graph: files = nodes, edges = dependency references. Runs **graph ranking (PageRank-style)** to rank which symbols/files matter most, then a **binary search** fits the top-ranked tags into a token budget.
- Token budget: `--map-tokens` default **1024**; when no files are in chat it expands by `map_mul_no_files` (default **8×**) up to context window minus ~4096 padding.
- **Benchmark deltas: NONE published in the primary blog or docs.** The blog is purely mechanistic. Third-party summaries assert "PageRank-scored context beats naive file inclusion" but I found no first-party quantified aider benchmark for map-on vs map-off. Treat as unquantified by the vendor. (Lead: aider code-editing leaderboard may have edit-format numbers, not map-ablation numbers.)

### Repomix (and neighbors)

- Source: `https://github.com/yamadashy/repomix`, `https://repomix.com/`, comparison writeups (rywalker.com, sentra.app), 2026.
- Packs a whole repo into one AI-friendly file. `--token-budget N` fails CI with non-zero exit if output exceeds N (guardrail pattern for agent workflows).
- **Tree-sitter compression** extracts signatures/structure and strips implementation bodies → claimed **~70% token reduction** (vendor claim, not an independent benchmark).
- Positioning: best for _one-shot ad-hoc context packing, zero infra_. Contrast with aider's map, which is _dynamic and inside the agent loop_.
- 2026 category shift noted by third parties: local-first pre-computed **code graphs served over MCP** (CodeGraph, GitNexus) emerging as a pattern — pre-computed _structure_ on-device, distinct from vector RAG. (Lead, third-party claim, unverified.)

### ctags-based maps

- Aider's own lineage: an earlier ctags approach was replaced by the tree-sitter map (ctags gave definitions but no reference graph, so no centrality ranking). ctags maps = flat symbol index, no relevance ranking. Effectively superseded for agent use by tree-sitter + graph ranking.

### Anthropic / Claude Code — agentic search, no index

- Sources: Anthropic "Effective context engineering for AI agents" (`anthropic.com/engineering/effective-context-engineering-for-ai-agents`, **2025-09-29**, primary); Claude.com "How Claude Code works in large codebases" (**2026-05-14**); corroborated by multiple secondary writeups citing a Claude engineer on HN.
- **May 2025: Anthropic removed vector search / embedding index from Claude Code.** Replaced embedding pipeline + local vector DB + chunking with Glob/Grep/Read on-demand ("just-in-time context loading"). Reported reason: agentic search was "more accurate, simpler to operate," and dodged staleness/security/privacy of a maintained index. A Claude engineer: agentic search outperformed embeddings "by a lot… surprising" (secondary quote, HN).
- Nuance directly relevant to the framework: navigation quality is "**shaped by how well the codebase is set up, layering context with CLAUDE.md files and skills**"; "folder hierarchies, naming conventions, and timestamps all provide important signals." LSP (semantic refs) beats grep (thousands of raw matches) for reference-finding. **No quantitative numbers given** in either post — all qualitative.
- Industry adoption of "no vector index": Claude Code, Cursor, Windsurf, Cline, Sourcegraph Amp (per secondary sources).

---

## MEASURED evidence (the ablations)

### SWE-Explore — arXiv 2606.07297 (2026-06, HTML fetched)

Benchmark of repository _exploration/localization_ in isolation: 848 instances, 10 languages, 203 repos; ground truth = code spans successful agents actually consulted. Metrics @K=5 regions:

| Explorer             | HitFile | Precision | Line Recall | nDCG@500 |
| -------------------- | ------- | --------- | ----------- | -------- |
| Random               | 0.004   | 0.002     | 0.004       | 0.004    |
| BM25                 | 0.079   | 0.055     | 0.021       | 0.132    |
| TF-IDF               | 0.140   | 0.117     | 0.049       | 0.223    |
| Claude Code          | 0.667   | 0.598     | 0.154       | 0.938    |
| Mini-SWE-Agent       | 0.640   | 0.530     | 0.151       | 0.885    |
| CoSIL (graph search) | 0.544   | 0.581     | **0.788**   | 0.824    |

- **Sparse/embedding retrieval ≈ random; agentic explorers form a clear tier above.** Embedding RAG only marginally beats lexical, still near-random.
- Downstream validation (restricted-context repair): exploration quality strongly predicts patch success — Context Efficiency r=+0.950, File Hit Rate r=+0.925, nDCG@500 r=+0.921 with resolve rate. Oracle context → 59.7% resolve; general agents 41–50%.
- **"Missing context is the dominant failure mode"** — patchers tolerate moderate redundant/extra context but are hurt badly by _incomplete_ core evidence. → Recall-oriented maps (don't omit the load-bearing file) beat precision-oriented ones. This is a direct design constraint for a compressed map: err toward including the structurally-central node, not toward minimalism.

### ORACLE-SWE — arXiv 2604.07789v2 (2026-05-29, HTML fetched)

Isolates 5 "oracle" information signals and measures each one's resolve-rate lift (SWE-bench-Verified, GPT-4o baseline ~38%):

| Signal            | Baseline | +Signal | Delta     |
| ----------------- | -------- | ------- | --------- |
| Reproduction Test | ~38%     | ~64%    | **+26pp** |
| Execution Context | ~38%     | ~50%    | +12pp     |
| **Edit Location** | ~38%     | ~50%    | **+12pp** |
| API Usage         | ~38%     | ~46%    | +8pp      |
| Regression Test   | ~38%     | ~41%    | +3pp      |

- Ordering: Reproduction Test ≫ Execution Context ≈ Edit Location ≫ API Usage ≫ Regression Test.
- **"Edit Location" is the signal that maps most directly onto a structural repo map** (telling the agent _where_ the relevant code is): worth ~+12pp. Reproduction tests win overall but are a task artifact, not a doc/map. Takeaway for the framework: a map's measurable payoff is roughly the "edit-location" channel — real, ~10pp-scale, but bounded; it will not substitute for the agent still running/reproducing.

### SWE Context Bench — arXiv 2602.08316v3 (2026, HTML fetched)

Context-learning ablation (Claude Sonnet 4.5):

- Oracle **Summary** Learning: **34.34%** (best) vs No-Context 26.26% vs Oracle full-trajectory Context 27.27%.
- **Curated compact summaries > raw trajectory retrieval.** But oracle-vs-free gap was huge for summaries (12.12pp) and tiny for trajectories (1.01pp): "concise summaries are highly effective when correctly selected, but **misleading when irrelevant**."
- Conclusion: retrieval/curation _quality_ dominates over mechanism type. → A curated map helps only to the extent it is correct and relevant; a wrong/stale map is actively harmful. Direct justification for drift-check tooling.

### Agentless — arXiv 2407.01489 (2024-07, FSE 2025)

- Uses a **hierarchical localization** (repo tree → files → edit locations) with NO autonomous tool use, then simple diff repair. Achieved 27.33% on SWE-bench with lowest cost ($0.34/problem) at publication — evidence that _structured hierarchical narrowing_ is competitive with agentic loops and far cheaper.
- Caveat later raised by agent-approach authors: Agentless's narrow fixed scope caps recall when a broader candidate set is needed — echoes SWE-Explore's "missing context is the dominant failure" finding.

---

## Synthesis for the framework decision (adopt vs build)

- **Do NOT build a vector/embedding index.** Measured evidence (SWE-Explore) + vendor practice (Anthropic removed theirs) both say static embedding retrieval underperforms on-demand agentic search for code. This is the most robust finding in the corpus.
- **A curated hierarchical map DOES have measured value, via the "edit-location" / "concise-summary" channel (~+10–12pp), NOT via replacing exploration.** Frame the framework's map as _navigation priors that shorten the agent's search_, not as a retrieval substitute.
- **Recall over precision in the map.** Missing the load-bearing file is the dominant failure; redundant context is tolerated. Compress by stripping implementation bodies (repomix/aider tree-sitter pattern, ~70% reduction) while KEEPING structural coverage, rather than pruning files.
- **Freshness is load-bearing, not cosmetic.** A wrong summary hurts about as much as a right one helps (SWE Context Bench). This is the empirical mandate for the generator/drift-check deliverable — an unmaintained map is measurably net-negative, not merely neutral.
- **Adopt existing map mechanics rather than invent:** tree-sitter tag extraction + graph-centrality ranking + token-budget binary search is a solved, proven pattern (aider); repomix's `--token-budget` CI guard and signature-compression are reusable. Build the _hierarchical/AGENTS.md layering + drift-check_, not the low-level map extractor.

## Caveats / confidence

- Aider publishes NO first-party map-ablation benchmark; its "map helps" claim is mechanistic/qualitative. Treat as unquantified.
- Anthropic/Claude posts give zero numbers — directional vendor testimony only.
- The strongest quantified crux evidence is ORACLE-SWE (edit-location +12pp) and SWE Context Bench (summaries +12pp / harmful-when-wrong). Both are 2026 arXiv, single-benchmark, model-specific — treat magnitudes as order-of-magnitude, direction as reliable.
- ~70% token-reduction (repomix) and PageRank-beats-naive (aider) are vendor/third-party claims, not independent benchmarks.

# Claims ledger

## C1: AGENTS.md is the cross-vendor standard — OpenAI-originated (2025-08), Linux Foundation/AAIF-stewarded (2025-12-09), 60k+ repos, ~30 tools

status: established
for: agents.md (spec, fetched); LF press release 2025-12-09; OpenAI announcement 2025-12-09
note: spec is deliberately schema-free Markdown; "nearest file wins"; formal merge semantics live per-harness. v1.1 proposal (#135) would make nesting/progressive-disclosure explicit — spec direction, not yet spec.

## C2: Claude Code does NOT natively read AGENTS.md; bridge = CLAUDE.md containing `@AGENTS.md` (or symlink)

status: reported (official docs, fetched 2026-07-19)
for: code.claude.com/docs/en/memory
note: negative capability claim — re-verify at design time (platform may add native support).

## C3: Claude Code loading — ancestors full at launch; subdir CLAUDE.md on-demand on file read (NOT re-injected post-compaction); @-imports all load at launch, max 4 hops, NO context savings; path-scoped .claude/rules = native progressive disclosure

status: established (official docs, fetched; corroborated by Anthropic steering blog)
for: code.claude.com/docs/en/memory; claude.com/blog/steering-claude-code…
note: design-critical — "unpacking" must lean on path-scoping/on-demand nesting, never @-imports.

## C4: Codex loading diverges — static root→cwd chain only, concatenated, 32 KiB cap (`project_doc_max_bytes`), no on-demand deep/sibling discovery

status: reported (OpenAI vendor docs, fetched)
for: learn.chatgpt.com/docs/agent-configuration/agents-md; codegateway playbook
note: openai/codex issue #12115 confirms dynamic nesting is an unshipped feature request (Wix/Stripe/Clay asking). ⇒ portability requires an explicit root router, not reliance on lazy loading.

## C5: For FINDING code, agentic search (grep/read) ≫ embeddings/lexical retrieval, which is near-random on repo localization

status: established
for: SWE-Explore arXiv 2606.07297 (BM25 HitFile 0.079 vs Claude Code 0.667); Anthropic removed Claude Code vector index 2025-05 (claude.com large-codebases post + engineering post)
note: kills any "build an embedding index" component.

## C6: Curated, correct, COMPACT pointers/summaries lift resolve rate ~+10–12pp (edit-location, concise summaries) — but wrong/misleading summaries actively hurt

status: reported (two 2026 single-benchmark papers; direction reliable, magnitudes order-of-magnitude)
for: ORACLE-SWE arXiv 2604.07789 (edit-location +12pp); SWE Context Bench arXiv 2602.08316 (oracle summaries 34.3% vs no-context 26.3%; "misleading when irrelevant")
note: value channel = navigation priors, not retrieval substitute. Missing context is the dominant failure (SWE-Explore r≈0.95) ⇒ maps favor recall over minimalism.

## C7: LLM-generated context files hurt marginally (−0.5% SWE-bench Lite / −2% AGENTbench; intro headline −3%) at +20–23% cost; human-written +4% avg (but NOT beating none for Claude Code)

status: established (PRIMARY fetched wave 2: arXiv 2602.11988, Gloaguen et al., ETH Zurich SRI Lab + LogicStar, ICML tag)
for: arxiv.org/html/2602.11988v1 (full body); corroborating secondary dair.ai + InfoQ
note: effects MARGINAL both directions. Per-cell rates only in Figure 3 bar chart (unrecoverable). Biggest hedge: Python-only — parametric knowledge may nullify context files; effect could be larger for niche stacks.

## C17: Mechanism (ETH primary): overviews are INERT even when human-written; only specific tooling/build/test instructions transmit value; redundancy + unnecessary requirements are the harm channel

status: established (same primary, content analysis + ablations)
for: arXiv 2602.11988 — time-to-relevant-file unchanged by context files (Fig 4); uv used 1.6×/instance when mentioned vs <0.01× when not, repo tools 2.5× vs <0.05×; LLM-generated files become +2.7% (beating human!) when all other docs are stripped — redundancy explains the null; "unnecessary requirements from context files make tasks harder"
note: RECONCILIATION with C6 — ORACLE-SWE's +12pp is task-specific oracle pointers; ETH tests static repo-level files. Task-relevant pointers help; static overviews don't. The framework's root must NOT be a prose overview; value = non-derivable knowledge + lazy structural navigation, never restating what README/code already say.

## C8: Curated skills +16.2pp avg but software-engineering domain only +4.5pp; self-generated −1.3pp; 2–3 focused docs optimal, comprehensive (−2.9pp) hurts

status: reported (primary fetched)
for: SkillsBench arXiv 2602.12670
note: crux hedge — SE uplift is the weakest measured domain; validate on target repos.

## C9: Long context degrades task performance even with PERFECT retrieval; measured mitigation = retrieve-then-solve (+31.2% Mistral, +4% GPT-4o)

status: established
for: arXiv 2510.05381 (EMNLP 2025 Findings, fetched); Chroma context-rot report 2025-07-14 (fetched, 18 models)
note: Chroma found NO U-shape in its NIAH runs — U-shape is Liu et al. (TACL 2024), magnitudes there secondary-sourced. Also Chroma: shuffled haystacks beat coherent ones (NIAH artifact, hedge). Correction to prior IA-playbook framing: attribute U-shape to Liu, not Chroma.

## C10: Stale comments/docs are measurably harmful — inconsistent comments ~1.52× odds of preceding a bug-introducing commit (7-day window, 8 Apache Java projects)

status: reported (primary fetched)
for: arXiv 2409.10781
note: measured on humans, transfer to agents plausible-not-measured. Empirical mandate for drift-checking; with C6's "misleading summaries hurt" ⇒ unmaintained docs are net-negative, not neutral.

## C11: llms.txt has no measured effect on AI citation/crawler behavior (300k domains, SE Ranking; 0.1% crawler touch, OtterlyAI)

status: established (multiple independent measurements)
for: SEJ 2025-11-20 relaying SE Ranking; OtterlyAI 90-day crawler logs; Google docs 2026-06
note: web-crawler context — does NOT directly transfer to on-disk repo agents; lesson = pointer files aren't consumed without an enforcement/loading mechanism.

## C12: Practitioner + vendor convergence: root = compressed always-loaded map/router (≤200 lines, ~≤5 KiB); nested detail on-demand; explicit router (Datadog `@`-table) is the portable pattern

status: established
for: Anthropic best-practices docs (fetched); Anthropic steering blog ("under 200 lines"); HumanLayer (<60-line root, ~15 rules, 150–200 instruction ceiling — folklore-grade numbers); Datadog monorepo post; Codex 1–3 KiB/file guidance
note: instruction-count ceilings (150–200) are unsourced heuristics — cite as folklore. HN: factual content + observed-failure rules get followed; invented preventative rules ignored; positive framing beats negative.

## C13 (revised post-adversarial + Charter audit): No tool both generates CODE-ISOMORPHIC hierarchical agent-facing docs in-repo AND verifies docs-AGAINST-CODE in CI — as of 2026-07; both halves exist separately and maturely

status: established (adversarial refutation attempted wave 2, upheld; Charter audited directly in main session 2026-07-19)
for: w2-whitespace-adversarial matrix (11 candidates, each fails ≥1 conjunct): hcc = hierarchical agent-facing gen + GH Action, NO drift mode; RepoAgent = gen+auto-regen but human-facing, pre-commit not CI; agents-lint / context-drift = mature CI drift-lint, no generation; Loki docs = staleness gate but human-facing
against (near-refuter): Charter (Stackbilt-dev/charter, Apache-2.0, npm @stackbilt/cli) — ADF modular .ai/ source, trigger-keyword on-demand loading, compiles to CLAUDE.md/AGENTS.md/.cursorrules/GEMINI.md, `adf compile --check` CI gate, `charter score` freshness audit
note: Charter's drift gate checks compiled-artifact-vs-ADF-source, not docs-vs-code; its modules are trigger-based rules, not a code-tree-isomorphic doc hierarchy; 0 stars, created 2026-02, surfaced via vendor self-promotion in spec issue #135. It is prior art to steal from (single-source compile-to-vendor-formats, trigger index, score), not a refuter. The whitespace is real but THIN — hcc + `git diff --exit-code` ≈ 5 lines; differentiation must come from the verified-against-code layer and the curation bar (C17), not mere assembly.

## C18: GitHub's "2,500 repositories" AGENTS.md post is editorial, not a study — sample size is its only quantitative datum

status: established (primary fetched wave 2)
for: github.blog, Matt Nigh, 2025-11-19
note: do not cite as measured evidence; its six-core-areas taxonomy (commands/testing/structure/style/git/boundaries) is advice, consistent with C12/C17. Copilot docs DO formalize nested AGENTS.md nearest-wins + @path includes (support since 2025-08-28), nesting uneven across surfaces.

## C19: Spec direction — AGENTS.md v1.1 proposal (issue #135): jurisdiction/accumulation/inheritance nesting + progressive disclosure (index → inject → load-on-demand, optional description/tags frontmatter); OPEN, unmerged, community-authored, no maintainer signal

status: established as a proposal-state fact (issue fetched, incl. reactions/comments)
for: github.com/agentsmd/agents.md/issues/135 (created 2026-01-08, 5 👍, comments all authorAssociation=NONE)
note: framework aligns with spec direction, not against it; ≤500-line guideline stated in draft. Don't treat as ratified.

## C20: Claude Code ships the drift-loop primitives natively: InstructionsLoaded hook (which instruction files loaded + load_reason: session_start/nested_traversal/path_glob_match/include/compact; observe-only) + Stop hook (propose doc updates from transcript) + claudeMdExcludes

status: established (official docs fetched wave 2)
for: code.claude.com/docs/en/hooks; code.claude.com/docs/en/large-codebases
note: large-codebases page explicitly prescribes root router + per-dir on-demand + path-scoped rules, and names staleness maintenance (PR-review CLAUDE.md edits, revisit after model releases, Stop-hook update proposals). Build ON these, don't rebuild them.

## C21: Supporting numbers (comment-sourced, NOT independently verified): optimal context-file length 335–535 words (arXiv 2511.12884); file localization = #1 agent failure mode (ContextBench 2602.05892); well-structured AGENTS.md → 28.64% runtime / 16.58% token reduction on SWE-bench Verified even where success flat (Lulla 2601.20404)

status: unverified (single GitHub comment relay, papers not fetched)
for: dgenio comment on issue #135, 2026-03-02
note: Lulla efficiency-not-success framing fits C7/C17 perfectly (docs pay in cost/speed even when resolve rate is flat) — verify before citing as fact; report only with attribution.

## C14: C4, arc42 §5, and Diátaxis independently converge: doc tree isomorphic to system structure; one altitude/one mode per doc; L4/code level = generate, never hand-maintain

status: established (three primary framework sites fetched)
for: c4model.com; arc42.org; diataxis.fr
note: plus ADR conventions (Nygard 2011: numbered, immutable, in-repo) and Architecture Haiku (Fairbanks: one-page root discipline); Backstage TechDocs = production precedent for co-located per-component docs + root manifest.

## C15: Placement matters within loaded files — relevant info at beginning/end beats middle (U-shape)

status: reported (shape primary via ACL abstract; magnitudes ~20pp secondary)
for: Liu et al. TACL 2024 / arXiv 2307.03172
note: load-bearing content at top of root file.

## C16: "openai/codex has ~88 AGENTS.md files" is FALSE for the public repo — actual count: 2

status: established (verified directly, 2026-07-19)
for: GitHub git-trees API, repos/openai/codex main branch recursive: 2 matches
against: morphllm/buildbetter/codersera guides claiming ~88 (secondhand, likely garbled or internal-repo)
note: do not cite the 88 figure; the public canonical nesting example claim is refuted.

## C22: Graph extraction/interchange layers are commodity and Rust-embeddable — tree-sitter-graph (MIT/Apache, arbitrary attributed graphs from ASTs) + SCIP (Apache protobuf, rust-analyzer emits natively); stack-graphs ARCHIVED 2025-09; CodeQL engine not OSS; Glean is Haskell server-side

status: established (wave 3, primaries fetched)
for: w3-code-knowledge-graphs findings
note: Glean is the only prior system shipping the graph→doc-gen/agent-RAG thesis. Adopt tree-sitter-graph as extraction layer, SCIP as import path.

## C23: The agent-facing code-graph category (~18 tools, 2025-26) is crowded on ONE axis only — derived AST/LSP graphs over MCP. codebase-memory-mcp (DeusData, C single binary) is the near-twin: get_architecture progressive-disclosure root, emits AGENTS.md, 15 MCP tools + Cypher — but purely derived, no intent typing, CI checks its own binary not the graph

status: established (wave 3; star counts/token-reduction figures vendor-claimed, UNVERIFIED — repo growth flagged suspicious)
for: w3-agent-graph-tools findings; rywalker.com 18-tool survey (2026-03/06): "none document AGENTS.md emission, CI self-verification, or progressive disclosure"
note: potpie = fastest-converging threat (authored `record` entries + `doctor` drift, but additive layer over derived graph). No tool exposes GraphQL. Others emit AGENTS.md as a POINTER to their required server — inverse of engine-free fallback.

## C24: Architecture conformance tooling is mature FORWARD-only (undeclared dep fails: ArchUnit family, dependency-cruiser, eslint-boundaries, go-arch-lint); the REVERSE direction (declared-but-unused edge flagged) is absent everywhere; Rust has NO ArchUnit equivalent; Structurizr generates models FROM code, never checks code AGAINST model

status: established (wave 3)
for: w3-arch-assertions findings
note: doc-as-enforced-signature (agent-readable declaration + bidirectional check) shipped NOWHERE. Steal: dependency-cruiser `required` rule (closest reverse primitive), eslint boundaries/no-unknown (exhaustiveness), ArchUnit FreezeArchRule (adoption ratchet), go-arch-lint YAML (doc-adjacent declaration shape).

## C25: Comment-grammar prior art — AIDEV-NOTE (origin: Diwank Singh Tomer, diwank.space, 2025-06-07) is descriptive breadcrumbs, no validation; todocheck is the one shipped CI-validated comment-claim tool (TODO↔issue-tracker liveness, both directions); doxygen \xrefitem/\invariant = aggregate-tags + typed-claim ancestry

status: established (wave 3, primaries fetched incl. origin post)
for: w3-comment-grammars findings
note: claims-not-descriptions grammar + graph round-trip + CI self-assertion = unoccupied. JML/design-by-contract is the formal ancestor (lead, unfetched).

## C26: Schema-typed docs — ADF types STRUCTURE not semantics (spec: "no schema on what instructions... only how they're structured and when they load"); Graphify/codegraph tag edges EXTRACTED vs INFERRED (provenance precedent); Backstage types the metadata graph (dependsOn edges) but never CI-checks it against imports; semantic node kinds (commands/invariants/hazards) shipped nowhere

status: established (wave 3, ADF SPEC.md audited)
for: w3-typed-doc-schemas findings
note: THE GAP, precisely positioned — "authored-intent-vs-extraction reconciliation over a semantically-typed graph, degrading to plain AGENTS.md." Not "first typed agent docs" (ADF holds structure+budgets+compile) nor "first code graph" (commodity). Four unclaimed conjuncts: authored-first source, intent/C4 typing, bidirectional CI self-assertion, engine-free fallback.

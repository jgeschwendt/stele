# Wild ideas — synthesis of the 10-lens ideation fleet

2026-07-19 · 10 opus ideators (compilers, stigmergy, cartography, epistemology, economics, gamedesign, database, empiricist, vianegativa, historian) · 58 ideas · judged by the session model. Raw pool: task output `w2puz0nh5`; per-lens details preserved there.

## The meta-finding

Seven of ten lenses independently converged on the same blind spot: **every existing tool measures docs against CODE; nothing measures docs against READERS.** The entire ecosystem is supply-side (author → verify → serve). The unclaimed territory is demand-side: docs provisioned, routed, priced, decayed, and evicted by observed agent behavior. Independent convergence across adversarially-separated lenses is the strongest novelty signal this method can produce.

## The seven territories (clustered from 58 ideas)

### T1 — Demand-side docs (the convergence)

Agents reveal demand: successful navigation trails, grep-storms/backtracking (confusion), loads-then-bypassed (distrust), never-loaded (dead weight).

- **Pheromone Router / Bloodstains / Desire-Path Paving** — the router table is deposited by outcome-verified agent traversals, decays when unused; CI checks the committed router against the trail data. The map becomes the fossil record of successful navigation.
- **Confusion Market / Confusion Cartography** — mine flailing (wide repeated searches, re-reads, backtracks) into a ranked doc-writing backlog: write docs only where agents measurably get lost; flag docs on cold paths as speculative.
- **Doc Circulation Records / Fog-of-War audit** — readership lifecycle: never-loaded → archive; loaded-then-code-spelunked-anyway → distrusted, queue for regeneration; dark tiles (hot traffic, no doc) → recall gaps.
- Shared risks: cold-start (seed from static structure, refine with traffic); attribution noise (gate on outcome-verified sessions only); rare-but-critical paths read as dead (pin flag).

### T2 — The proving ground (measure on YOUR repo)

- **Resolve-Rate Ratchet** — distill your merged PRs into a held-out task suite (AGENTbench-from-PRs); doc PRs gate on bootstrap-CI resolve-rate delta. Closes the loop ETH left open; the report's "validate on your own repos" made standing infrastructure.
- **Docmut (doc mutation testing)** — inject plausible lies (flag swap, path corruption, claim negation) chunk by chunk; if resolve-rate doesn't drop, the chunk was never causally read (cut it); if agents follow the lie into failure, it's load-bearing (keep + pin). The cleanest anti-Goodhart instrument in the pool — measures causal readership, which presence-based checks cannot.
- **Token Ledger** — per-chunk Shapley/leave-one-out contribution stamps (`<!-- contrib +3.1pp ±1.8 · model=opus-4.8 -->`); model-version-keyed, so a model upgrade SUNSETs chunks the smarter model stopped needing.
- **Regret Miner** — after each failed benchmark task, ask the counterfactual "what one fact would have saved this?"; admit the candidate doc only if a re-run proves uplift past a pre-registered CI. Generation inverted: from measured failure, gated on measured lift — subverts the ETH reject-generation verdict legitimately.
- **Rot Radar / Blindfold Legibility Gate** — behavioral staleness (contribution sign-flip with zero text change); docs-only agent draws the codebase from the map, divergence heatmap = where the map lies.
- Shared risks: statistical power on low-traffic repos (must say "insufficient evidence", never fake a verdict); benchmark overfit (rotate fixtures); cost (nightly, not per-commit).

### T3 — Time as first-class (actuarial trust)

- **Watermark & Resume-Delta** — docs carry `verified_sha` + jurisdiction globs; an agent gets the doc PLUS `git log verified_sha..HEAD -- <jurisdiction>` distilled ("since this was verified: login handler moved"). A delta is non-derivable by construction — the cleverest clean subversion of the ETH redundancy null in the pool.
- **Half-Life Docs / epistemic-status types** — per-claim confidence decays on the git clock (commits-past-anchor since verification); rendered inline as graded trust, not binary pass/fail. Extends the user's existing `(verified DATE)` stamp convention into a computed model.
- **Self-Redacting Doc** — past a churn threshold the loader serves a stub ("STALE: 40 commits since verified — re-derive from these paths") instead of the body. If stale-and-served is measurably negative and absent is zero, self-redaction strictly dominates. Knife-edge thresholds; structural router exempt.
- **Doc Bisect / As-Of** — the drift predicate run across history: find the commit where a claim started lying; assemble briefs at snapshot consistency for branch/bisect work.

### T4 — Docs as enforced contracts (run the arrow backwards)

- **The .mli Inversion** — AGENTS.md `arch` blocks declare components + allowed edges; CI diffs the real import graph against the declaration both ways (undeclared edge = code broke the signature; unbacked edge = doc lied). The doc IS the architecture, not a description of it.
- **Gotchas to Guardrails** — ban the gotcha prose section; compile each gotcha into a lint rule / pre-commit hook / `test_does_not_<mistake>`. A guard can't go stale silently. Residue that resists mechanization stays prose — the exception that proves the bar.
- **Bonfires** — every doc node carries one `checkpoint:` command scoping tests to its region; CI lights all bonfires; failures render an "on fire" hazard in the router. Rides the single strongest ETH positive (tooling commands are the only content that transmits).
- **RefAction / Double-entry ledger / Hearsay gate** — typed FKs doc→symbol with ON DELETE semantics (restrict/cascade/set-null); every claim needs a credit (anchor) or it's flagged hearsay; intentionally unanchored facts booked to an explicit "equity" account so the non-derivable bar is a booking discipline, not reviewer vibes.

### T5 — Negative-space documentation (what bites, what's false)

- **Antibody Docs** — mine correction signals (fast reverts, test flip-flops within one session, "no, don't…" in transcripts) into gotchas with a titer that rises on recurrence and decays to retirement when the trap's code moves. Docs authored by the pathogen, dosed by recurrence. The report names gotchas as a top value channel but gives no production mechanism — this is it.
- **Refuted-beliefs ledger** — document the plausible-but-false ("you'd expect X; it's not, because…"), each entry carrying a falsification probe CI re-runs so a belief that becomes true gets flagged. Negative claims rot hardest; the probe is the survival condition.
- **Gotcha Ledger with self-retiring tripwires** — each mined gotcha bound to a regression test; no live tripwire → auto-retire. Staleness structurally impossible.
- **Hazard Tiles / Topographic Relief** — git-mined danger overlay (fix-density × low coverage × churn) injected as decaying ⚠ blocks; two independent signals must co-fire to avoid crying wolf.

### T6 — Territory over map (via negativa)

- **Landmark Registry with Grep-Cardinality CI** — landmark = symbol whose `rg -w` returns exactly one definition + non-dictionary contrast, enforced in CI; docs anchor to landmark IDs, not paths, so moves never break anchors. Renames over prose: shape the territory for the search that measurably works.
- **Path-as-Router Grammar** — enforce a naming grammar strict enough that `tree` output IS the router; maintain by moving files, not editing prose. Dies on cross-cutting concerns; scope to the tree-shaped 80%.
- **Ephemeral Read-Time AGENTS.md** — commit no router at all; synthesize it per-session from the tree + guards + hot set. Zero artifact, zero drift; only derivable structure though — non-derivable content still needs a persisted source.
- **Doc Budget Ratchet / Doc seigniorage** — the always-loaded root has a fixed token money supply; minting requires burning equal weight in the same PR, or an explicit owned cap-raise. Anti-bloat as monetary policy rather than review vibes.

### T7 — Archaeology & provenance

- **Archaeology Generation** — compile per-subsystem WHY.md from commit bodies + PR text + ADRs, every sentence footnoted to a SHA; CI validates footnotes. Rationale is the purest non-derivable content and it's already written in the exhaust. Ceiling = commit-message quality; confabulation checkable via footnotes but not prevented.
- **Distortion-Honest Projections** — multiple generated maps of the same territory (flow / data-lineage / blast-radius / churn), each with a mandatory Distortion Legend stating what it hides; root is a projection selector enforcing load-exactly-one.
- **World 1-1** — tutorialize the doc SYSTEM itself: a first-quest that has a fresh agent perform one verified change in a sandbox so it learns the map grammar by doing (procedural content is what measurably helps; overview prose is what doesn't).

## Session-model additions (gaps the fleet missed)

- **The Librarian Daemon** — nobody proposed the _role_: a standing background curator agent (launchd/cron, exactly like the existing memory sweep) that owns the doc tree — processes the confusion backlog, runs circulation reports, retires antibodies, refreshes watermarks, opens doc PRs. The framework's tools are its instruments; the daemon is the loop that plays them. This user already runs this exact infrastructure shape for memory.
- **Memory-pipeline coupling** — session learnings that are _repo facts_ (not personal facts) should route out of the personal `@memory` pipeline and into the repo's gotcha/antibody queue as PR candidates. The dissolve/sweep extraction already classifies memories; add a `repo-durable` type whose destination is the repo's `.agents/` intake, closing the loop between session experience and committed repo knowledge — for every contributor's agents, not just this machine's.
- **Dissolving scaffold (model-keyed docs)** — generalize Token Ledger's SUNSET: every doc chunk is scaffolding for a capability the current model lacks; chunks carry the model class they were written for, and a model upgrade triggers a re-audit pass that proposes deletions. The doc tree is designed to shrink as models improve — a framework that plans its own obsolescence.

## Three coherent visions (commit to one center of gravity; per the Compromise principle, don't blend)

### Vision A — The Metabolic Map (demand-side, living)

Center: T1 + T3 + T5, librarian daemon as the heartbeat. The doc tree is an organism: routers deposited from outcome-verified trails, gotchas formed as antibodies from failures, claims decaying on the git clock, self-redacting past threshold, resume-deltas giving each reader its blind spot, circulation pruning dead weight. Authored-on-faith content is the exception requiring justification. Radical bet: telemetry volume exists and attribution is tractable. Fits this user: hooks + transcript archive + sweep infra already exist.

### Vision B — The Signature (enforced contracts, static)

Center: T4 + T6. The doc tree is a typed signature the code must satisfy: arch blocks checked against the import graph, gotchas compiled to guards, per-node checkpoints, FK anchors with referential actions, grep-cardinality landmarks, fixed root money supply, tree-as-router grammar. Drift is structurally impossible rather than detected. No telemetry needed — works day one on any repo, degrades nowhere. Radical bet: enough knowledge is mechanizable; the judgment residue stays small.

### Vision C — The Proving Ground (empirical, benchmark-gated)

Center: T2. The repo carries its own agent benchmark distilled from PR history; every chunk carries a measured contribution stamp; doc PRs gate on resolve-rate CI; mutation testing finds dead and dangerous lines; regret mining generates candidates admitted only on proven uplift; nightly rot radar catches behavioral staleness. Docs are hypotheses under permanent experiment. Radical bet: per-repo statistical power is reachable (realistic for active monorepos, not small repos).

### Recommended composition (deliberate layering, not a blend)

**B is the skeleton, A is the metabolism, C is the laboratory.** Ship the Signature layer as the universal spec (works with zero traffic, zero benchmark — the portable standard). The Metabolic layer is the first-class extension where telemetry exists (this machine qualifies today). The Proving Ground is the validation harness the framework uses on itself — run C's ratchet on 2–3 real repos to earn the numbers the whole field lacks, which is also the framework's public differentiation: nobody else can say "measured on our own repos, per-chunk."

## Standout shortlist (novelty × mechanism × evidence-fit)

1. Docmut — doc mutation testing (T2)
2. Watermark & Resume-Delta (T3)
3. Antibody Docs + self-retiring tripwires (T5)
4. Pheromone Router / Bloodstains (T1)
5. Gotchas to Guardrails (T4)
6. Resolve-Rate Ratchet (T2)
7. Landmark Registry with grep-cardinality CI (T6)
8. The .mli Inversion (T4)
9. Self-Redacting Doc (T3)
10. Confusion Market (T1)
11. Regret Miner (T2)
12. Archaeology Generation (T7)
13. Librarian Daemon (session addition)
14. Doc seigniorage / budget ratchet (T6)
15. Blindfold Legibility Gate (T2)

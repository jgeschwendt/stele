# stele

```stele
kind: system
purpose: stele — typed agent-doc graph reconciled against extracted code truth. SPEC.md is the contract; the Rust engine implements it and self-hosts on this repo.
commands:
  test: cargo test
  lint: cargo clippy --all-targets -- -D warnings && cargo fmt --check
  graph: cargo run -- build && cargo run -- emit   # local only; CI never runs stele build (§5.1)
invariants:
  - claim: SPEC.md outranks everything here — code and examples conform to it; changing behavior means revising the spec (and its decision log) first
    anchor: SPEC.md#decision-log
  - claim: every [Cn] citation must stay licensed by research/claims.md's status for that claim — never state a `reported` claim as settled fact
    anchor: research/claims.md#c1
```

This repo eats its own convention: the block above is a real stele node — `stele build` compiles it into `.stele/graph.lock`, CI runs `stele check` + `stele emit --check` on it, and the file stays readable with zero engine (the plain-file degradation the spec guarantees).

## Map

| where | what |
| --- | --- |
| GUIDE.md | the adoption path — install → init → anchor → build → CI |
| SPEC.md | the v1 specification (read §1 constraints + §2 model first) |
| EXAMPLE.md | worked example — read this first to feel the design |
| research/ | evidence base: report, claim ledger, primary-source findings |

## Working here

- Design decisions in SPEC.md's decision log and §10 are settled — relitigate only with new evidence, in a PR that updates the log.
- Doc edits own the whole file: verify external claims (harness behavior, tool capabilities) against live sources, and date-stamp negative claims — several in SPEC.md carry `(as of 2026-07…)` markers that rot by design.
- The §4 assertion suite is the test-fixture contract; EXAMPLE.md's CI failure gallery is the expected-output oracle (`tests/gallery.rs` encodes it).

<!-- stele:begin router -->

## Hazards (0 active)


## Map

| node     | kind      | purpose                                                                                                                                                                                    | unfold                                                 |
| -------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------ |
| research | container | Evidence base for SPEC's [Cn] citations. claims.md is the ledger every citation stays licensed by (status per claim); report.md the synthesis, findings/ the primary-source notes.         | `stele unfold research` · or read `research/AGENTS.md` |
| scripts  | container | Distribution scripts. install.sh/uninstall.sh land or remove the single stele binary; release.sh assembles the per-platform tarball + sha256 the release workflow publishes on a v* tag.   | `stele unfold scripts` · or read `scripts/AGENTS.md`   |
| src      | container | The stele engine. cli.rs is the command pipeline (build/check/emit); model.rs the typed graph, anchors.rs the §2.5 scanner, assert.rs the six §4 checks, extract.rs the import extractors. | `stele unfold src` · or read `src/AGENTS.md`           |
| tests    | container | Integration + unit suite and the CI oracle. common/mod.rs is the git-repo harness; gallery.rs mirrors EXAMPLE §8's failure gallery. fixtures/ (steleignored) is the acme worked example.   | `stele unfold tests` · or read `tests/AGENTS.md`       |

## Indexes

All invariants: `.stele/index/invariants.md` · all hazards: `.stele/index/hazards.md`

## Engine

`stele` CLI available → `stele root | unfold <id> | invariants --touching <path> | hazards | nodes --kind <k>`. MCP: `stele serve`.
No engine → everything above is complete; nested AGENTS.md files carry the detail (nearest file wins).
<!-- stele:end -->

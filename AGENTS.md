# stele

```stele
kind: system
purpose: Spec repo for stele — typed agent-doc graph reconciled against extracted code truth. Pre-implementation; SPEC.md is the contract.
invariants:
  - claim: SPEC.md outranks everything here — code and examples conform to it; changing behavior means revising the spec (and its decision log) first
    anchor: SPEC.md#decision-log
  - claim: every [Cn] citation must stay licensed by research/claims.md's status for that claim — never state a `reported` claim as settled fact
    anchor: research/claims.md#c1
```

This repo eats its own convention: the block above is a stele node in the plain-file degradation the spec guarantees (no engine exists yet — this file is what "works with zero engine" means).

## Map

| where | what |
| --- | --- |
| SPEC.md | the v1 specification (read §1 constraints + §2 model first) |
| EXAMPLE.md | worked example — read this first to feel the design |
| research/ | evidence base: report, claim ledger, primary-source findings |

## Working here

- Design decisions in SPEC.md's decision log and §10 are settled — relitigate only with new evidence, in a PR that updates the log.
- Doc edits own the whole file: verify external claims (harness behavior, tool capabilities) against live sources, and date-stamp negative claims — several in SPEC.md carry `(as of 2026-07…)` markers that rot by design.
- When implementation starts: the §4 assertion suite is the test-fixture contract; EXAMPLE.md's CI failure gallery is the expected-output oracle.

<!-- stele:begin router -->
<!-- stele:end -->

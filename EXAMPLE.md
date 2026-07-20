# stele — worked example

Companion to SPEC.md draft 0.8. A fictional-but-realistic monorepo (`acme`): Phoenix web app + background worker + shared TS package. Artifacts shown in full or as labeled excerpts so the design can be attacked concretely. Hole-poking prompts marked ⚔ throughout.

## 1. The repo

```
acme/
├── AGENTS.md                      ← system node (source + rendering, §3.1)
├── CLAUDE.md                      ← one line: @AGENTS.md
├── .stele/
│   ├── graph.lock                 ← compiled graph, committed
│   └── index/
│       ├── invariants.md          ← generated transpose (spec §6.1)
│       └── hazards.md
├── adr/
│   └── 0007-integer-cents.md
├── apps/
│   ├── web/
│   │   ├── AGENTS.md              ← container node
│   │   └── lib/
│   │       ├── billing/
│   │       │   ├── AGENTS.md      ← component node
│   │       │   ├── charge.ex
│   │       │   └── refund.ex
│   │       └── store/
│   │           └── AGENTS.md      ← component node
│   └── worker/
│       └── AGENTS.md              ← container node
└── packages/
    └── shared/
        └── AGENTS.md              ← container node (TypeScript)
```

## 2. Root `AGENTS.md` (system node) — the initialContext

````markdown
# acme

```stele
kind: system
purpose: Subscription billing platform — Phoenix web app, job worker, shared TS client lib.
commands:
  setup: mise install && mix deps.get
  test: MIX_ENV=test mix test
  check: mix precommit          # compile --warnings-as-errors, format, test
  db-reset: mix ecto.reset      # DESTRUCTIVE — drops local db
invariants:
  - claim: all money amounts are integer cents end-to-end; floats never represent currency
    anchor: lm:money-type
    enforced_by: packages/shared/test/money.test.ts
edges:
  decided_by: [adr/0007]
budget: 900
```

<!-- stele:begin router · generated, checked by `stele emit --check` · do not hand-edit -->

## Hazards (2 active)

- ⚠ `apps/worker`: dunning job is NOT idempotent per-invoice — re-running a failed batch double-emails (→ lm:dunning-batch)
- ⚠ `apps/web/lib/billing`: Stripe webhook handler must never write inside the signature-verification transaction (→ lm:webhook-verify)

## Map

| node            | kind      | purpose                                           | unfold                                                               |
| --------------- | --------- | ------------------------------------------------- | -------------------------------------------------------------------- |
| apps/web        | container | Phoenix app: API, checkout UI, webhooks           | `stele unfold apps/web` · or read `apps/web/AGENTS.md`               |
| apps/worker     | container | Oban jobs: dunning, invoice PDFs, webhook retries | `stele unfold apps/worker` · or read `apps/worker/AGENTS.md`         |
| packages/shared | container | TS client lib + money type shared with web        | `stele unfold packages/shared` · or read `packages/shared/AGENTS.md` |

## Indexes

All invariants: `.stele/index/invariants.md` · all hazards: `.stele/index/hazards.md`

## Engine

`stele` CLI available → `stele root | unfold <id> | invariants --touching <path> | hazards | nodes --kind <k>`. MCP: `stele serve`.
No engine → everything above is complete; nested AGENTS.md files carry the detail (nearest file wins).
<!-- stele:end -->
````

Rendered size: ~500 tokens (cl100k-class approx, ±10%). That is the _entire_ always-loaded cost of this repo for a Claude session.

⚔ **Poke here:** is the hazard banner at root pulling hazards up from child nodes a duplication-of-fact violation of constraint 6? (Current answer: no — the generated region is a _projection_ of the child's single source, checked by `emit --check`. But it means a stale regeneration shows stale hazards; the check only fails when someone forgets to re-run `emit`.)

## 3. `apps/web/lib/billing/AGENTS.md` (component node)

````markdown
# billing

```stele
kind: component
purpose: Charges, refunds, Stripe webhook intake. The only module allowed to call Stripe.
commands:
  test: MIX_ENV=test mix test apps/web/test/billing
invariants:
  - claim: every mutation is idempotent by (account_id, idempotency_key) — retries must be safe
    anchor: lm:billing-idempotency
    enforced_by: apps/web/test/billing/idempotency_test.exs
  - claim: refunds never exceed captured amount, enforced at the changeset, not the controller
    anchor: lm:refund-cap
hazards:
  - claim: Stripe webhook handler must never write inside the signature-verification transaction
    anchor: lm:webhook-verify
edges:
  depends: [apps/web/lib/store, packages/shared]
  decided_by: [adr/0007]
budget: 600
```

<!-- stele:begin router -->

## Anchors in this territory

- lm:billing-idempotency → charge.ex:41
- lm:refund-cap → refund.ex:18
- lm:webhook-verify → charge.ex:112

<!-- stele:end -->
````

⚔ **Poke here:** `refund-cap` has no `enforced_by`. Per §2.4 it compiles but is flagged (`check` reports "1 prose-only claim") and gets the short freshness leash. Is a nag-report the right teeth, or should prose-only claims decay harder?

## 4. Anchored code — `apps/web/lib/billing/refund.ex`

```elixir
defmodule AcmeWeb.Billing.Refund do
  # stele:landmark refund-cap
  # stele:claim apps/web/lib/billing/refund-cap
  @doc "Caps refund at remaining captured amount. See adr/0007 for integer-cents."
  def changeset(refund, attrs) do
    refund
    |> cast(attrs, [:amount_cents, :charge_id])
    |> validate_refund_cap()
  end
  ...
end
```

Note what the anchors do **not** say: no description of the cap logic (that's the code's job), no restated invariant (that's the AGENTS.md block's job). The anchor is an address + a binding.

⚔ **Poke here:** two lines of comment ceremony per claim. Acceptable? The alternative (`anchor: refund.ex#changeset`) needs zero comments but breaks on rename/move — landmark ids survive both.

## 5. `adr/0007-integer-cents.md`

```markdown
# 7. Money is integer cents everywhere

Date: 2026-03-02 · Status: Accepted

## Context

Float rounding produced a $0.01 reconciliation drift across ~40k invoices (incident 2026-02-28).

## Decision

All money = integer cents, at every boundary: DB, API, TS client. The `Money` type in
packages/shared is the only constructor.

## Consequences

(+) reconciliation exact; (−) every external API needs an explicit cents↔decimal edge adapter.
```

## 6. `.stele/graph.lock` (excerpt)

```json
{
  "version": 1,
  "nodes": {
    "apps/web/lib/billing": {
      "kind": "component",
      "declared": {
        "depends": ["apps/web/lib/store", "packages/shared"],
        "decided_by": ["adr/0007"],
        "allow": []
      },
      "extracted": { "imports": ["apps/web/lib/store", "packages/shared"] },
      "contains": [],
      "claims": [
        {
          "id": "billing-idempotency",
          "kind": "invariant",
          "text": "every mutation is idempotent by (account_id, idempotency_key) — retries must be safe",
          "anchor": "lm:billing-idempotency",
          "resolved": "apps/web/lib/billing/charge.ex:41",
          "enforced_by": "apps/web/test/billing/idempotency_test.exs",
          "verified": { "sha": "e3f19ac…", "digest": "1f3c…" }
        },
        {
          "id": "refund-cap",
          "kind": "invariant",
          "text": "refunds never exceed captured amount, enforced at the changeset, not the controller",
          "anchor": "lm:refund-cap",
          "resolved": "apps/web/lib/billing/refund.ex:18",
          "enforced_by": null,
          "verified": { "sha": "e3f19ac…", "digest": "a187…" }
        }
      ]
    }
  }
}
```

## 7. An agent session — "add partial-refund support"

What a Claude Code session actually loads, step by step:

| step | action                                                                                                                                                               | context cost    |
| ---- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| 0    | session start: root AGENTS.md (via `@AGENTS.md` shim)                                                                                                                | ~500 tok        |
| 1    | task mentions refunds → router points at apps/web → `stele unfold apps/web`                                                                                          | ~100 tok        |
| 2    | `stele unfold apps/web/lib/billing` → full billing node: commands, 2 invariants, 1 hazard, anchor table                                                              | ~250 tok        |
| 3    | `stele invariants --touching apps/web/lib/billing` — pulls the money invariant from the SYSTEM node too (cross-cutting: it lives at root, billing inherits exposure) | ~110 tok        |
| 4    | `rg -n "stele:landmark refund-cap"` → jump straight to refund.ex:18, read the region                                                                                 | code, on demand |
| 5    | implement; run the node's own `test` command from step 2                                                                                                             | —               |

Total doc overhead: **~950 tokens** (same basis), every one of them non-derivable (commands, invariants, hazards, addresses). The flat-file equivalent of this repo's knowledge is a typical 300–600 line CLAUDE.md (~3–6k tokens) loaded on _every_ session including the ones about CSS.

The agent never read an architecture overview, and never needed one: step 4 is agentic grep, the thing that measurably works [C5]; steps 1–3 are navigation priors — the channel a single 2026 benchmark (ORACLE-SWE) put at ~+12pp; direction reliable, magnitude order-of-magnitude [C6].

⚔ **Resolved (spec 0.2, §6.1):** the former engine-only cross-cutting gap — a no-engine harness reads `.stele/index/invariants.md` directly (root carries the pointer shown in §2, never the claims dump) [C6 vs C17 reconciled].

## 8. CI failure gallery — `stele check` output, one per assertion class

**8.1 Structural / violation (forward)** — someone adds `alias AcmeWeb.Billing` inside `apps/web/lib/store/subscription.ex`:

```
✗ structural: apps/web/lib/store imports apps/web/lib/billing — edge not declared
    apps/web/lib/store/subscription.ex:9  alias AcmeWeb.Billing.Charge
  declared depends of apps/web/lib/store: [packages/shared]
  fix: remove the import, or declare it in apps/web/lib/store/AGENTS.md (and mean it), or allow: {edge: apps/web/lib/billing, reason: "..."} for dynamic/DI cases
exit 1
```

**8.2 Structural / vestigial (reverse — the direction nobody else checks)** — billing stops importing store after a refactor:

```
✗ structural: apps/web/lib/billing declares depends on apps/web/lib/store — no import found
  the signature promises a dependency the code no longer has (doc lied, or dependency died)
  fix: remove the edge from apps/web/lib/billing/AGENTS.md, or allow: with reason if the dep is runtime-dynamic
exit 1
```

**8.3 Referential** — someone renames the landmark comment during a refactor:

```
✗ referential: anchor lm:refund-cap unresolved (0 occurrences of "stele:landmark refund-cap")
  claim "refunds never exceed captured amount, enforced at the changeset, not the controller" is now unanchored — provenance broken
✗ referential: landmark lm:money-type has slug-match cardinality 2
    packages/shared/src/legacy/money.ts:7   ← duplicated in a copy-paste refactor
    packages/shared/src/money.ts:3
exit 1
```

**8.4 Freshness** — someone edits `changeset/2` (the digested region) to loosen the cap, without touching the claim:

```
✗ freshness: claim billing/refund-cap — AST digest of enclosing region changed
  verified at e3f19ac (digest a187…), region changeset/2 now digests 4f0d…
  staling commit: b8e02d1 "loosen cap for partial captures" — `stele blame billing/refund-cap`
  fix: re-read the region, re-affirm or amend the claim, `stele build` re-stamps {sha, digest}
  note: 9 formatting/comment commits in the same range did NOT fire — AST digest ignores them
  note: billing-idempotency's region also changed but is NOT flagged — its enforced_by
        test passed in this run; the guard is the freshness proof
exit 1
```

**8.5 Budget** — a teammate pastes a style guide into the root file's free-prose area:

```
✗ budget[codex]: root chain apps/web/lib/billing → 34.1 KiB > 32 KiB default cap (project_doc_max_bytes) — Codex truncates the overflow (vendor docs report silent)
✗ budget[node]: / renders 1,410 tokens > declared budget 900
  largest contributor: unmanaged prose block "## Style notes" (612 tokens)
exit 1
```

**8.6 Exhaustiveness + liveness** — a new `apps/api/` directory lands; a mix task is deleted:

```
✗ exhaustiveness: apps/api (14 files) is covered by no node — unreachable via any router
✗ liveness: command / :db-reset → `mix ecto.reset` — task not found in mix.exs (removed in a1b2c3d)
exit 1
```

⚔ **Poke at the gallery:** (a) 8.1's `allow:` escape — what stops it becoming `# noqa` spam? (Current answer: `allow` requires `reason:` and shows up in `check --report`; nothing else.) (b) 8.4 — residual weakness after the AST-digest upgrade: a semantic change _outside_ the digested region (e.g. a caller starts bypassing the changeset) still slips through — `enforced_by` remains the real proof (§4.5). (c) 8.5 — **Resolved (0.3, §4.4):** bundled cl100k-class approximation, ±10%, cap margins absorb it.

## 9. The degradation ladder, shown

| harness                          | what it gets                                                                     | what it loses                                            |
| -------------------------------- | -------------------------------------------------------------------------------- | -------------------------------------------------------- |
| Codex / anything AGENTS.md-aware | root + nearest-file-wins chain, complete typed content, hazards, router-as-prose | cross-cutting queries; lazy unfold (its chain is static) |
| Any harness + Bash               | above + `stele unfold/invariants/check` as tool calls                            | nothing material                                         |
| Claude Code + MCP                | above + typed tools, no shell round-trips                                        | —                                                        |
| Human with an editor             | readable markdown files, one per directory, plus one YAML block each             | nothing — that's the point                               |

## 10. Standing holes (author-acknowledged, wanting attack)

1. **The extractor's blind spots are the signature's blind spots.** Elixir dynamic dispatch (`apply/3`, behaviours, Oban job names as strings) produces imports the extractor can't see → `allow:` escapes accumulate exactly where the architecture is most dynamic — which may be where the signature matters most.
2. **Two-file ceremony per node.** Every component worth documenting costs an AGENTS.md with a YAML block. `stele init` scaffolds it, but the marginal cost per node is real; if teams only fill the root, stele degrades to a checked router — is that still worth the binary? (I claim yes: 8.5 + 8.6 alone pay for it.)
3. **`purpose` is unverifiable prose.** 200 chars of scent that no assertion can check. It's the one field that can lie undetected. Kept because routers need scent [C19]; capped because lying scales with length.
4. **Generated-region merge conflicts.** Two branches both re-run `emit` → conflicting generated blocks. Mitigation: deterministic rendering order makes conflicts rare and mechanical (re-run `emit`), but they will happen.
5. **The lockfile in review.** graph.lock diffs are noisy JSON in PRs. Option: make it `.gitattributes`-collapsed and rely on `check` in CI, at the cost of reviewers not seeing graph changes. Resolved by default (spec §3.2): pretty-printed canonical JSON keeps lock diffs mechanical; collapsing in review is a team choice.

```

```

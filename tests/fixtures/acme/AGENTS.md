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

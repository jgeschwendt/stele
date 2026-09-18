# Notation sample · ※ ⊨ § @stele

| slot       | glyph               | written                                  |
| ---------- | ------------------- | ---------------------------------------- |
| landmark   | ※                   | `# ※ refund-cap`                         |
| claim      | ⊨                   | `# ⊨ billing/refund-cap`                 |
| anchor:    | ※                   | `anchor: ※ refund-cap`                    |
| decided_by | §                   | `decided_by: [§ 0007]`                    |
| region     | `@stele` … `@end`   | `<!-- @stele -->` … `<!-- @end -->`      |

## apps/web/lib/billing/AGENTS.md

````markdown
# billing

```stele
kind: component
purpose: Charges, refunds and the money type. Integer cents only; refunds are capped at the original charge.
commands:
  test: MIX_ENV=test mix test apps/web/lib/billing
invariants:
  - claim: a refund never exceeds its original charge
    anchor: ※ refund-cap
    enforced_by: test/billing/refund_test.exs
  - claim: money is integer cents, never a float
    anchor: money.ex#Money
hazards:
  - claim: refund/2 is idempotent only per charge id; a retried webhook with a new id double-refunds
    anchor: ※ refund-entry
edges:
  depends: [apps/web/lib/store, packages/shared]
  decided_by: [§ 0007]
```

<!-- @stele -->
| anchor            | resolves to                  |
| ----------------- | ---------------------------- |
| ※ refund-cap      | refund.ex:18                 |
| ※ refund-entry    | refund.ex:9                  |
| money.ex#Money    | money.ex:3                   |
<!-- @end -->
````

## apps/web/lib/billing/refund.ex

```elixir
defmodule Acme.Billing.Refund do
  # ※ refund-entry
  def refund(charge, amount) do
    charge
    |> changeset(%{amount: amount})
    |> Repo.insert()
  end

  # ※ refund-cap
  # ⊨ billing/refund-cap
  defp changeset(refund, attrs) do
    refund
    |> cast(attrs, [:amount])
    |> validate_number(:amount, less_than_or_equal_to: refund.charge.amount)
  end
end
```

## packages/shared/src/money.ts

```ts
// ⊨ billing/money-is-integer-cents
export type Money = { cents: number };
```

## adr/0007-integer-cents.md

```markdown
# 7. Integer cents

Status: accepted

Money is `{cents: integer}`. Floats never enter the domain.
```

## Root AGENTS.md, tail

```markdown
<!-- @stele -->
## Map
| node    | kind      | purpose | unfold |
| ------- | --------- | ------- | ------ |
| billing | component | …       | `stele unfold billing` · or read `apps/web/lib/billing/AGENTS.md` |

## Engine
`stele` CLI available → `stele root | unfold <id> | invariants --touching <path> | hazards`.
No engine → everything above is complete; nested AGENTS.md files carry the detail (nearest file wins).
<!-- @end -->
```

## Grep surface

```
rg '※ '          # every landmark
rg '⊨ '          # every claim binding
rg '※ '          # landmarks and their anchor references, one grep
rg '§[0-9]'      # every decision reference
rg '@stele'      # every generated region
```

## Notes

- `✻` stays the human-notes glyph; the anchor field carries the landmark token verbatim, so one grep finds both sides.
- `@stele` / `@end` drops the region name. `router` was the only name v1 emits, so nothing is lost until a second region kind exists.
- Claim addresses keep the slash; the glyph carries the kind, so the slash is no longer load-bearing.

# billing

```stele
kind: component
purpose: Charges, refunds, Stripe webhook intake. The only module allowed to call Stripe.
commands:
  test: MIX_ENV=test mix test apps/web/test/billing
invariants:
  - claim: every mutation is idempotent by (account_id, idempotency_key) — retries must be safe
    anchor: ※ billing-idempotency
    enforced_by: apps/web/test/billing/idempotency_test.exs
  - claim: refunds never exceed captured amount, enforced at the changeset, not the controller
    anchor: ※ refund-cap
hazards:
  - claim: Stripe webhook handler must never write inside the signature-verification transaction
    anchor: ※ webhook-verify
edges:
  depends: [apps/web/lib/store, packages/shared]
  decided_by: [§ 0007]
budget: 600
```

<!-- @stele -->

## Anchors in this territory

- ※ billing-idempotency → charge.ex:41
- ※ refund-cap → refund.ex:18
- ※ webhook-verify → charge.ex:112

<!-- @end -->

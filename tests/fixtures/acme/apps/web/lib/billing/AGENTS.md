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

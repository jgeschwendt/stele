# worker

```stele
kind: container
purpose: "Oban jobs: dunning, invoice PDFs, webhook retries"
hazards:
  - claim: dunning job is NOT idempotent per-invoice — re-running a failed batch double-emails
    anchor: lm:dunning-batch
```

<!-- stele:begin router -->
<!-- stele:end -->

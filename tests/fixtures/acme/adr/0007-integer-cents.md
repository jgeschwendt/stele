# 7. Money is integer cents everywhere

Date: 2026-03-02 · Status: Accepted

## Context

Float rounding produced a $0.01 reconciliation drift across ~40k invoices (incident 2026-02-28).

## Decision

All money = integer cents, at every boundary: DB, API, TS client. The `Money` type in
packages/shared is the only constructor.

## Consequences

(+) reconciliation exact; (−) every external API needs an explicit cents↔decimal edge adapter.

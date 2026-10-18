# Improve soak (v4 B battle-test)

Mandatory receipt before any public self-improvement claim.

## Arms

| Arm | Meaning |
|---|---|
| `learn` | Isolated cache. Route memory, prewarm, quality prior on. Outcome feedback stays off (default). |
| `off` | Isolated cache. `DONSETCH_NO_ROUTE_MEMORY`, `DONSETCH_NO_PREWARM`, `DONSETCH_NO_QUALITY_PRIOR`, `DONSETCH_NO_EGRESS_PERSIST`. |

Same binary, same 30 hosts, same cadence.

## Run

```bash
# smoke (harness only)
DONSETCH_BIN=target/release/donsetch \
  python3 bench/improve/soak.py --cycles 3 --interval 120 --limit 5

# real 24h (both arms, sequential)
DONSETCH_BIN=target/release/donsetch \
  python3 bench/improve/soak.py --hours 24 --interval 1800
DONSETCH_BIN=target/release/donsetch \
  python3 bench/improve/soak.py --mode off --hours 24 --interval 1800
```

Output: `bench/improve/out/<run-id>/` with `report.md`, `meta.json`,
`events.jsonl`, and the isolated `cache/`.

## Prove

Learning is proven only if, comparing the two arms' full-duration reports:

1. `warm_rate_late` (ON) > `warm_rate_late` (OFF)
2. `mean_ms_late` (ON) < `mean_ms_late` (OFF)

No public improve claim without both receipts. Keep this directory;
it is the evidence, not a product surface.

# Implementation roadmap

Numbered, risk-ordered phases. Each is reversible-first where possible (read-only / empty=legacy). Phases name the [issue-tracker](../issues/) items (R/P/A) they deliver or unblock.

## Phases

| # | Phase | Delivers | Unblocks |
|---|-------|----------|----------|
| [00](00-overview.md) | Overview & principles | — | all |
| [01](01-observability-capacity-signal.md) | Observability: capacity/load signal | R3, P4 | 02, 06, 07 |
| [02](02-bw-aware-weights.md) | BW-aware weights + damping | S1, S8 | 05, 07 |
| [03](03-segment-stack-ext.md) | Segment-stack wire ext | P2 | 04, 08 |
| [04](04-fast-reroute-liveness.md) | Fast-reroute + fast liveness | R1, R6, S2, S4 | — |
| [05](05-priority-mtr.md) | Priority multi-topology routing | (uses P_MASK) | — |
| [06](06-gbr-declaration-policing.md) | GBR declaration + ingress policing | P1, R2, R4, R5 | 07 |
| [07](07-deterministic-admission.md) | Deterministic admission + derived load | A, E, I, J | 08, 09 |
| [08](08-multicast-scoped-detour.md) | Multicast scoped-detour | P3, A-MC1, A-MC2, S5 | 09 |
| [09](09-flow-preemption.md) | Flow preemption + edge notify | P5, D, S3 | — |

## Dependency graph

```
01 ─┬─► 02 ─┬─► 05
    │       └─► 07 ─┬─► 08 ─► 09
    ├─► 06 ─────────┘        ▲
03 ─┴─► 04                   │
03 ──────────────► 08 ───────┘
```

## Phase grouping by horizon

- **Near (engineering, reversible):** 01, 02, 03 — measure, then bounded BW-routing, then the keystone wire ext. All empty=legacy / read-only.
- **Mid (resilience + QoS):** 04, 05 — fast-reroute (needs liveness for silent failure, [R1](../issues/class-R-runtime.md)), priority paths.
- **Far (hard guarantees, research-gated):** 06, 07, 08, 09 — reservation, deterministic admission, multicast, preemption. **Scoped to the low-churn router backbone**; gated on Class-A research ([issues/class-A](../issues/class-A-algorithm.md)).

## Invariants across all phases
- Empty header / absent declaration = today's behavior (opt-in, reversible).
- Hard guarantees only at steady state on the backbone; edge best-effort.
- Integer/quantized arithmetic on any path that must be deterministic ([J](../issues/class-A-algorithm.md)).
- Make-before-break for every reroute ([oscillation.md](../issues/oscillation.md)).

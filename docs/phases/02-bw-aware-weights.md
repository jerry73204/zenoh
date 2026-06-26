# Phase 02 — BW-aware weights + damping

**Goal:** route around congestion using a **bounded, stable** congestion metric. Single-topology. Reversible (revert to static weight 100).

**Depends on:** [01](01-observability-capacity-signal.md).
**Delivers:** [S1](../issues/oscillation.md), [S8](../issues/oscillation.md) damping built-in.
**Unblocks:** 05, 07.

## Deliverables
1. Replace static weight 100 with a **bounded** metric:
   `w_e = static_bias + α·util_e`, `util_e = λ_e/C_e`, **α capped** (HN-SPF style).
   **Not** `1/(C_e−λ_e)` — unbounded slope = guaranteed oscillation ([S1](../issues/oscillation.md)).
2. Tune the **static/dynamic ratio** (`bias` vs `α`) — the documented stability knob.
3. Dijkstra over `w_e` (weights ≥0 → drop Bellman-Ford; integer).
4. Damping: EWMA (from 01) + quantize + hold-down/hysteresis `κ` + **partial response** (shift a fraction of flows, loop gain <1).
5. Optional: widest-path (max-min bottleneck) variant for bulk flows.

## Exit criteria
- Under synthetic congestion, traffic shifts to underused links **without flapping** (measure swing amplitude/period → damped).
- Revert flag returns to baseline static routing.

## Risks / issues
- [S1](../issues/oscillation.md) route flap if metric unbounded or ratio mis-tuned — the central risk; verify stability empirically ([open-questions](../issues/open-questions.md)).
- Higher churn → control-plane cost → shifts gossip↔linkstate threshold ([economy §3](../design/auto-routing-economics.md)).

## Design refs
[qos §2.2–2.4](../design/qos-fault-routing.md), [qos §10.9 S1](../design/qos-fault-routing.md).

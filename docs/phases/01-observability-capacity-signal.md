# Phase 01 — Observability: capacity / load signal

**Goal:** make per-link capacity and load *visible*. Read-only; no routing behavior change. Measure first.

**Depends on:** none.
**Delivers:** [R3](../issues/class-R-runtime.md) (measured-capacity export), [P4](../issues/class-P-protocol.md) (capacity in LSP).
**Unblocks:** 02, 06, 07.

## Deliverables
1. Export per-link residual-BW `R_e = C_e − λ_e` from TX pipeline counters (`pipeline.rs` `congested`/`pending` already track pressure; turn into a smoothed rate).
2. **EWMA-smooth + quantize** the load signal (anti-oscillation from day one; [S8](../issues/oscillation.md)).
3. Carry per-link capacity `C_e` and load in the existing `link_weights` LSP field (`network.rs:148`) as **integer** units ([J](../issues/class-A-algorithm.md)).
4. Capacity `C_e` from config initially; flag that real capacity ≠ config ([R3](../issues/class-R-runtime.md)).

## Exit criteria
- Every node can read a smoothed, quantized residual-BW for each of its links.
- LSP carries capacity; cross-node views converge at quiescence.
- No path selection change yet (verify routes identical to baseline).

## Why this matters beyond weights
Flooding per-link load is what lets a **TE detour retract** ([qos §8.7](../design/qos-fault-routing.md)): it is to bandwidth rerouting what topology withdrawal is to failure. Without it, a BW/priority SR detour becomes permanent standing per-flow state (stateless-core violation). This phase is the precondition for convergent TE.

## Risks / issues
- [R3](../issues/class-R-runtime.md): no link-layer rate on TCP/QUIC → estimate biased; document the bias.
- Adds churn `λ` to link-state → budget re-advertisement rate ([economy §3](../design/auto-routing-economics.md)).
- Load must be **damped/quantized** or the tree flaps and SR never settles ([S1](../issues/oscillation.md)).

## Design refs
[qos §2.1](../design/qos-fault-routing.md), [qos §12.2](../design/qos-fault-routing.md).

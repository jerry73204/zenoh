# Phase 04 — Fast-reroute + fast liveness

**Goal:** survive single link/node failure with sub-second detour, not the current up-to-10 s black hole.

**Depends on:** [03](03-segment-stack-ext.md) (segment stack for repair path).
**Delivers:** [R1](../issues/class-R-runtime.md), [R6](../issues/class-R-runtime.md), [S2](../issues/oscillation.md), [S4](../issues/oscillation.md).

## Deliverables
1. **TI-LFA**: precompute per-(dst, protected-link) backup; on failure push a repair segment stack. Repair path == post-convergence path → no tree↔repair swing.
2. **Fast liveness for silent failure** ([R1](../issues/class-R-runtime.md)): the dominant gap. TCP-close is ~ms, but silent failure waits the **10 s lease**. Add a BFD-like sub-second probe, or lower keepalive for GBR domains. *Without this, sub-second guarantees are impossible.*
3. Use **multilink failover** signal as an FRR trigger; enable multilink (off by default), add node-disjoint (not just link) redundancy ([R6](../issues/class-R-runtime.md)).
4. **Make-before-break** + **route-flap damping** (exp penalty + hold-down) + link-up debounce ([S2](../issues/oscillation.md), [S4](../issues/oscillation.md)).
5. Optional 1+1 disjoint-path duplication for `Reliable`+high-priority critical streams.

## Exit criteria
- Graceful (TCP-close) failure: detour within ~100 ms, no loss.
- **Silent** failure: detour within the liveness-probe interval (target sub-second), measured — not 10 s.
- A flapping link does not cause repeated reconvergence (damping verified).

## Risks / issues
- [R1](../issues/class-R-runtime.md) is 🔴: without fast liveness the whole "fault-tolerant" claim degrades to 10 s on silent failure.
- Lower keepalive ↑ control traffic — tune per domain.

## Design refs
[qos §4](../design/qos-fault-routing.md), [qos §12.1](../design/qos-fault-routing.md).

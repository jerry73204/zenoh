# Phase 08 — Multicast scoped-detour

**Goal:** BW-aware multicast without putting destinations on the packet. Keep Zenoh's implicit key-expr forwarding; tag only the *exception* (the detoured branch). Dup-free, gap-free.

**Depends on:** [03](03-segment-stack-ext.md) (segment stack), [07](07-deterministic-admission.md) (admission decides when to detour).
**Delivers:** [P3](../issues/class-P-protocol.md), [A-MC1](../issues/class-A-algorithm.md), [A-MC2](../issues/class-A-algorithm.md), [S5](../issues/oscillation.md).
**Unblocks:** 09.

## Deliverables
1. **Normal path unchanged**: key-expr + tree/source context → resolve to children-in-source-tree (`compute_data_route`). No bitmap, no destination list.
2. **Detour = key-expr + coarse scope + detour-flag** ([P3](../issues/class-P-protocol.md)): when a branch is blocked, tunnel `(key-expr, scope=blocked-branch region, flag)` via segment stack to an alternate node; it resumes forwarding **restricted to scope**.
3. **Correctness invariant**: `out(pkt) = children_in_source_tree(key-expr) ∩ scope(if flag)`.
   - Normal: single per-source tree + RPF → no dup.
   - Detour: scope overrides proximity, no greedy fan-out, **RPF-exempt** (tunneled), flag rides to region-local delivery.
4. **Deterministic scope/region assignment** ([A-MC2](../issues/class-A-algorithm.md)) + **sticky parent** ([S5](../issues/oscillation.md)).
5. Gossip peers that can't compute scope **detour upward to a router**.
6. (Deferred) BIER fallback ([P6](../issues/class-P-protocol.md)) only for huge sparse flat sets.

## Exit criteria
- Worked case (P→A direct, P→M→B, M detours to N): A delivered once, B delivered once, no dup/gap.
- Detour-flag honored end-to-end; dropping it early causes re-fan-out (negative test).

## Risks / issues
- 🔴 Inconsistent tree under churn → dup/gap (best-effort during convergence, [A](../issues/class-A-algorithm.md)).
- [A-MC1](../issues/class-A-algorithm.md): deterministic BW-Steiner approximation — quality bound open ([open](../issues/open-questions.md)).

## Design refs
[qos §8.5–8.6](../design/qos-fault-routing.md).

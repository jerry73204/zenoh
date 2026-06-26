# Phase 05 — Priority multi-topology routing

**Goal:** different traffic classes take different *paths*, not just different TX queues. RT → low-latency, bulk → spare-capacity.

**Depends on:** [02](02-bw-aware-weights.md) (per-class weight sets).
**Delivers:** path-level priority (today priority is TX-scheduling only).

## Deliverables
1. **Multi-Topology Routing (MTR)**: a weight set + SPF tree per routing class.
   - RT → latency weights (shortest-delay tree).
   - Data → balanced.
   - Bulk → spare-capacity / cost (widest-path).
2. **Collapse 8 priorities → 3 routing classes** {RT, Data, Bulk}. Trees are `O(n²)` each ([economy §3](../design/auto-routing-economics.md)); `K·n²` with K=8 is uneconomic. Keep all 8 for TX scheduling (cheap, local), 3 for path computation (expensive, global).
3. Forwarding indexes the per-class tree by the 3-bit `P_MASK` already on the wire (`network/mod.rs:460`).
4. Weighted fairness across classes (`β_class`); RT large, Background scavenger.

## Exit criteria
- An RT flow and a bulk flow between the same endpoints take measurably different paths.
- Tree-computation cost stays bounded (3 classes, only split a class when its traffic share justifies).

## Risks / issues
- `K·n²` cost — guard the class count; ties to the economy `n_max` budget.
- More trees → more recompute on churn ([E](../issues/class-A-algorithm.md)).

## Design refs
[qos §3](../design/qos-fault-routing.md).

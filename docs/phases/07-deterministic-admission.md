# Phase 07 — Deterministic admission + derived load

**Goal:** decide which GBR flows fit each link, **decentralized, no central PCE, no per-hop signaling** — by every router computing the same function on the flooded DB. **Backbone-scoped** (research-gated).

**Depends on:** [01](01-observability-capacity-signal.md) (capacity), [06](06-gbr-declaration-policing.md) (GBR declarations).
**Delivers:** [A](../issues/class-A-algorithm.md), [E](../issues/class-A-algorithm.md), [I](../issues/class-A-algorithm.md), [J](../issues/class-A-algorithm.md).
**Unblocks:** 08, 09.

## Deliverables
1. **Derived load**: `load(e) = Σ_{f: e∈tree(f,DB)} b_f` — pure function of the flooded DB (Zenoh already computes trees deterministically). No reservation state stored.
2. **Total-order admission** on **immutable** keys: `f ≺ g ⇔ (prio,ZID)` lex. **Never seqno** (refresh bumps it → [S3](../issues/oscillation.md)). Admit in `≺` order, each consuming residual.
3. **Integer/fixed-point** path computation ([J](../issues/class-A-algorithm.md)): replace the current `f64` graph (`network.rs:144-145`) — f64 diverges across arch → breaks consensus.
4. **Versioned admission function** ([I](../issues/class-A-algorithm.md)): flood version, activate-on-all-agree → no rolling-upgrade split-brain.
5. **Scope to the low-churn backbone** ([A](../issues/class-A-algorithm.md), [E](../issues/class-A-algorithm.md)): bounds the `O(flows×CSPF)` recompute and the consistency window. Edge = best-effort.
6. CSPF admission for unicast-ish flows first; multicast in [08](08-multicast-scoped-detour.md).

## Exit criteria
- All backbone routers reach **identical** admit/reject decisions on a converged DB (test cross-arch determinism).
- Admission converges within target time for the backbone's flow count.
- Under churn, guarantees degrade gracefully to best-effort (no oversubscription crash).

## Risks / issues
- 🔴 [A](../issues/class-A-algorithm.md)/[B](../issues/class-A-algorithm.md): never one DB under churn → divergent concurrent admission. Steady-state-hard only. The load-bearing risk — [open](../issues/open-questions.md).
- [E](../issues/class-A-algorithm.md): sequential recompute scaling → keep domain small.

## Design refs
[qos §8.1–8.4](../design/qos-fault-routing.md), [qos §9](../design/qos-fault-routing.md).

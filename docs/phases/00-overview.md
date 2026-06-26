# Phase 00 — Overview & principles

**Goal:** frame the program; fix the invariants every later phase obeys.

## Why
Routing role and QoS in Zenoh today are static config. The design makes them adaptive: bandwidth-aware paths, priority-differentiated paths, fast failure detour, bandwidth guarantees — auto-configured per topology/economy. Rationale: [design/](../design/).

## Non-negotiable invariants
1. **Opt-in / reversible.** Empty segment header and absent GBR declaration == today's behavior. Every phase ships dark.
2. **Steady-state-hard, transient-best-effort.** Hard guarantees only at quiescence on the **low-churn router backbone**; edge is best-effort. (Root cause: [issues/README](../issues/README.md).)
3. **Determinism on the fixed point, damping on the path to it.** Integer arithmetic where deterministic ([J](../issues/class-A-algorithm.md)); jittered timing ([S7](../issues/oscillation.md)).
4. **Make-before-break** for every reroute/detour/preemption.
5. **Measure before enforce.** Read-only observability lands before any control loop.

## Scope boundaries
- Backbone = the elected router tier ([economy §7–8](../design/auto-routing-economics.md)): small, low-churn, holds reservation/admission state.
- Edge = peers/clients: best-effort, detour upward to a router when they can't compute scope/admission.

## Exit criteria
- Invariants agreed; backbone/edge boundary defined for the target deployment.
- Constants baseline measured (lease, churn rate, flow count) to size domains.

## Risks
- Treating this as global hard-QoS — it is not. See [issues/class-A](../issues/class-A-algorithm.md).

# Phase 09 — Flow preemption + edge notify

**Goal:** when a link is oversubscribed, a higher-priority GBR flow preempts lower-priority ones; preempted flows reroute. Backbone-internal; edge flows notified or dropped.

**Depends on:** [07](07-deterministic-admission.md) (admission/`≺` order), [08](08-multicast-scoped-detour.md) (detour to reroute preempted flow).
**Delivers:** [P5](../issues/class-P-protocol.md), [D](../issues/class-A-algorithm.md), [S3](../issues/oscillation.md).

## Deliverables
1. **Deterministic preemption**: on an oversubscribed link, the flow losing the `≺` contest is preempted — every router computes the same outcome.
2. **No-notification, backbone-internal**: a preempted backbone source runs the same admission function → *independently* learns it lost → reroutes via scoped detour. No PathErr loop.
3. **Edge preemption-notification** ([P5](../issues/class-P-protocol.md)): edge publishers run no admission and get no backpressure → otherwise they just suffer silent drops. Add an explicit "preempted, reroute" message backbone→edge, **or** accept edge drops (best-effort).
4. **Anti-flap / anti-starvation** ([S3](../issues/oscillation.md), [D](../issues/class-A-algorithm.md)): `setup ≤ holding` priority, **minimum holding time** before preemptible, aging to prevent starvation, **soft-preempt via make-before-break**.
5. Map the 7 public priorities (Control=0 internal) to RSVP-style setup/holding levels.

## Exit criteria
- A high-priority flow preempts a low-priority one on a full link; the preempted backbone flow reroutes (MBB, no gap).
- No preemption ping-pong under steady DB (verify with churn → damped, not oscillating).
- Edge flow either receives a notify or degrades cleanly to best-effort.

## Risks / issues
- Preemption cascade / ping-pong ([S3](../issues/oscillation.md), [D](../issues/class-A-algorithm.md)) — needs immutable order + holding time + MBB.
- Edge can't be cleanly rerouted without [P5](../issues/class-P-protocol.md) — scope hard preemption to backbone.

## Design refs
[qos §12.3](../design/qos-fault-routing.md), [qos §8.2](../design/qos-fault-routing.md).

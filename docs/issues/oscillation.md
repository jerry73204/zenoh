# Oscillation modes (Class-A sub-family)

Every swing = a control action (reroute/detour/preempt/role-flip) changing the signal (load/order/topology) that triggered it, loop gain ≥1 with delay ⇒ limit cycle. Determinism *worsens* herding. Full discussion + citations: [qos §10](../design/qos-fault-routing.md).

| ID | Swing | Status | Documented fix | Source |
|----|-------|--------|----------------|--------|
| S1 | BW-weight route flap (idle link → herd → congest → back) | mitigation-known | metric normalization + static bias; **bounded** slope (not `1/(C−λ)`); ratio = stability knob | ARPANET HN-SPF; *Dynamics of load-sensitive routing* |
| S2 | Detour relocates congestion → swings back | mitigation-known | flow-pinning + make-before-break (MBB) | FAMTAR; RFC 5712 |
| S3 | Preemption ping-pong (refresh bumps seqno → order flips) | mitigation-known | order on immutable `(prio, HLC-birth-epoch, ZID)` — never link-state `sn`; HLC epoch gives stable incumbency (see [A-D](class-A-algorithm.md)); `setup ≤ holding`; min holding time; soft-preempt (MBB) | RFC 5712; H3C; US 9667559 |
| S4 | Link-flap reconvergence | mitigation-known | route-flap damping (exp penalty/decay); SPF exp backoff; link-up debounce | RFC 2439 + RFC 7196; RFC 8541 |
| S5 | Multicast parent/branch flip (ECMP tie) | mitigation-known | consistent/sticky hashing (remap only affected); sticky parent | Cisco Sticky ECMP; US 8595239 |
| S6 | Role ↔ admission cross-layer | mitigation-known | timescale separation: role ≫ admission ≫ weight ≫ packet | economy §6.2 |
| S7 | Determinism herd sync (lockstep moves) | mitigation-known | jitter the *apply* time, not the result (fixed point is order-independent) | Floyd & Jacobson 1994 |
| S8 | Estimator resonance (EWMA ≈ loop delay) | mitigation-known | exp backoff; estimator ≫ loop; bounded gain | IGRP stability |

## Damping principles (apply across all)
dead-band (κ margin) · dwell/hold-down · **immutable ordering keys** · EWMA + flap-damping · timescale separation · partial response (gain<1) · stickiness · **desync-without-divergence** (jitter apply, keep result identical).

## Cross-cutting fix
**Make-before-break (MBB)** — RFC 5712 / RFC 4090 — build new path before tearing old: no gap, and the overlap absorbs transient DB disagreement ([A-A](class-A-algorithm.md)). Adopt for every reroute/detour/preemption.

## The determinism paradox
The property giving consensus (identical computation) causes lockstep herding (S7) and seqno-sensitivity (S3). Resolution: **deterministic on the fixed point, damped on the path to it** — stable inputs (immutable keys, smoothed signals), jittered timing.

## References
See [qos §10.9 References](../design/qos-fault-routing.md).

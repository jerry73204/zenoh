# Design specs

The *why* and the *math*. Self-contained design notes.

| Doc | Scope |
|-----|-------|
| [auto-routing-economics.md](auto-routing-economics.md) | Role selection (router/peer/client, gossip↔linkstate) as a cost/economy optimization. Control(n²) vs data(multicast Steiner) tradeoff; hierarchy-forced-at-scale; facility-location / distributed-PCE; role-selection game; honest mechanism-design limits. |
| [qos-fault-routing.md](qos-fault-routing.md) | Bandwidth-aware + priority + fault-tolerant routing. SR/TI-LFA, bounded congestion metric, multi-topology priority, **deterministic distributed admission**, **implicit-multicast + scoped-detour** (with the dup-free correctness invariant), two-timescale rerouting, oscillation modes + literature-backed damping, code reality-check, gap classification. |
| [sr-stateless-features.md](sr-stateless-features.md) | Catalog of features SR + stateless design unlocks beyond the core: in-network processing/SFC (RFC 8986), per-slice constraint topologies (Flex-Algo), stateless telemetry (IOAM), bounded latency (DetNet), stateless mobility (LISP), policy/jurisdiction, anycast replica, proof-of-transit. Plus the stateful(NDN)-vs-stateless design axis. Related-work grounded. |
| [mixed-criticality-rt.md](mixed-criticality-rt.md) | Deterministic RT for mixed flows (control + HF sensor + background) — the robotics/AV case. Isolation's 3 attack surfaces, network-calculus latency bounds, FRER for control, freshness-drop for sensor, ATS over TAS (clock-free), and Zenoh's RT gaps (no preemption / drop-newest / no shaping). Grounded in TSN/DetNet. |

Canonical issue tracking lives in [`../issues/`](../issues/); the build roadmap in [`../phases/`](../phases/). These design docs keep the full discussion (issue tables in qos §9/§10/§13 are the rationale; the issue dir is the actionable list).

# Design specs

The *why* and the *math*. Self-contained design notes.

| Doc | Scope |
|-----|-------|
| [auto-routing-economics.md](auto-routing-economics.md) | Role selection (router/peer/client, gossip↔linkstate) as a cost/economy optimization. Control(n²) vs data(multicast Steiner) tradeoff; hierarchy-forced-at-scale; facility-location / distributed-PCE; role-selection game; honest mechanism-design limits. |
| [qos-fault-routing.md](qos-fault-routing.md) | Bandwidth-aware + priority + fault-tolerant routing. SR/TI-LFA, bounded congestion metric, multi-topology priority, **deterministic distributed admission**, **implicit-multicast + scoped-detour** (with the dup-free correctness invariant), oscillation modes + literature-backed damping, code reality-check, gap classification. |

Canonical issue tracking lives in [`../issues/`](../issues/); the build roadmap in [`../phases/`](../phases/). These design docs keep the full discussion (issue tables in qos §9/§10/§13 are the rationale; the issue dir is the actionable list).

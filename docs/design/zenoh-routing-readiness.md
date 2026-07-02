# Zenoh routing readiness: deterministic / bounded-latency / BW-guaranteed / mixed-flow

**Status:** review. Assesses **Zenoh's actual routing design** (code, July 2026) against four goals, with the specific blockers and the minimal change per goal. Consolidates the grounded findings scattered across [issues](../issues/) and [qos §12](qos-fault-routing.md).

## Zenoh routing as-is (grounded)

- **Forwarding = key-expr → cached face-set.** `get_data_route` resolves a key-expr against the resource tree to an output face set, cached per (resource, region, tree-context/`NodeId`), invalidated by `routes_version` (`dispatcher/pubsub.rs:182`). Multicast is implicit (one per-source tree, each subscriber one parent).
- **Single topology.** `ext_qos` (priority/reliability/congestion) is **carried on the wire and preserved on forward** (`pubsub.rs:337`) but only drives **TX scheduling** — it **never enters route computation**. RT and bulk take the **same path**.
- **One next-hop per destination.** Trees store `directions: Vec<Option<NodeIndex>>` — a single next hop, **no ECMP** (`network.rs:131`). Path ≈ min-hop (static weight 100).
- **Tree computation:** Bellman-Ford over `StableUnGraph<Node, f64>` (`network.rs:144`), 100 ms debounce.
- **State:** hard-state declarations (no refresh) + pull-on-reconnect (`initial_interest`); publishers **implicit** (no declaration).
- **Failure:** socket-close ~ms / silent-failure lease **10 s** → `sn++` reflood → 100 ms recompute.
- **QoS knobs that exist:** 8 strict-priority TX queues (`pipeline.rs:843`, non-preemptive, starvation-possible), Reliable/BestEffort, Drop(newest)/Block(5 s). **Absent:** bandwidth/capacity notion, reservation, rate shaping/policing, preemption, frame preemption, latency metric, clock sync.

**One-line:** best-effort, single-topology, key-expr-multicast, hard-state, eventually-consistent routing. Good bones, **zero** of the four guarantees today.

## Scorecard

### Goal 1 — Deterministic routing
| | |
|--|--|
| Today | Routes are a pure function of the graph → *deterministic at quiescence*. **But** `f64` Bellman-Ford risks cross-arch divergence ([J](../issues/class-A-algorithm.md)); equal-cost tie-break is graph-iteration-order, not a stable rule; under churn nodes hold divergent DB snapshots ([A](../issues/class-A-algorithm.md)). |
| Blockers | f64 arithmetic; no explicit tie-break; eventual consistency. |
| Minimal change | **Integer/fixed-point weights** + **stable tie-break** (lowest-ZID predecessor) + accept **quiescence-scoped** determinism. |
| Where | [phase 07](../phases/07-deterministic-admission.md), issues J / A. |

### Goal 2 — Bounded latency
| | |
|--|--|
| Today | **No bound.** Path is min-hop, not latency-aware. TX strict-priority helps but is **non-preemptive** (a low-prio batch blocks control — HoL, [R7](../issues/class-R-runtime.md)). No admission → unbounded queueing. 100 ms recompute + 10 s silent-failure = large transients. |
| Blockers | no latency metric; no frame preemption (R7); no admission cap; no shaping (R9); no path pinning. |
| Minimal change | **latency Flex-Algo** + **network-calculus admission** ([A-NC](../issues/class-A-algorithm.md)) + **preemption / bounded low-prio batch** (R7) + **pinned SR path** for RT + **ATS** shaping. |
| Where | [mixed-criticality-rt.md](mixed-criticality-rt.md), phases 05/07. |

### Goal 3 — Bandwidth guarantee
| | |
|--|--|
| Today | **None.** Static weight 100, no capacity signal, no reservation, no policing, no e2e backpressure. |
| Blockers | no capacity in LSP ([P4](../issues/class-P-protocol.md)/[R3](../issues/class-R-runtime.md)); no publisher GBR declaration ([P1](../issues/class-P-protocol.md)); no admission ([phase 07](../phases/07-deterministic-admission.md)); no policing ([R2](../issues/class-R-runtime.md)/[R9](../issues/class-R-runtime.md)); hop-by-hop only ([R4](../issues/class-R-runtime.md)). |
| Minimal change | **capacity signal → GBR declaration → derived-load admission → ingress policing**, **backbone-scoped**. |
| Where | phases 01 → 06 → 07. |

### Goal 4 — Mixed-flow support
| | |
|--|--|
| Today | **Partial primitives, no isolation.** 8 priorities + Reliable/BestEffort + Drop/Block exist and ride the wire, but affect only **TX scheduling / local queue policy** — not routing, not isolation. Single-topology (no per-class path); no rate isolation; **Drop=newest breaks freshness** ([R8](../issues/class-R-runtime.md)); strict-priority **starvation** possible. |
| Blockers | single-topology routing; no shaping/isolation (R9); Drop-newest (R8); starvation ([D](../issues/class-A-algorithm.md)). |
| Minimal change | **per-class Flex-Algo topologies** (control/sensor/bulk) + **per-class shaping+reserve** (R9) + **drop-oldest** (R8) + **preemption** ([phase 09](../phases/09-flow-preemption.md)). |
| Where | [mixed-criticality-rt.md](mixed-criticality-rt.md), phases 05/09. |

## The goals are one program, not four features

They share a single substrate:

```
capacity signal (01) ──► BW-aware weights (02) ──► per-class Flex-Algo (05)
        │                                                │
        └──► GBR declaration + policing (06) ──► deterministic admission (07)
                                                   │        │
                              network-calculus (A-NC) ◄─────┘
                                   │
        SR ext (03) ──► FRR + liveness (04) ──► multicast scoped-detour (08) ──► preemption (09)
```

- **Deterministic routing** (goal 1) is the *precondition* for admission (goal 3) and per-class paths (goal 4) — you can't guarantee what you can't reproduce.
- **Bounded latency** (goal 2) = deterministic routing + latency metric + admission + isolation → it's goals 1+3+4 composed, plus preemption/ATS.
- **Mixed-flow** (goal 4) is the *reason* for per-class topologies and isolation, which the other goals also need.

So: **one coherent build (phases 01–09), not four parallel efforts.**

## Verdict

Zenoh routing today provides **none** of the four guarantees — it is deliberately best-effort. But the **foundation is sound**: deterministic tree computation, QoS already on the wire, the `NodeId` SR hook, hard-state + pull rejoin, implicit-multicast. All four goals are **achievable**, with two hard truths:

1. **Scoped, not global.** Hard guarantees live on the **low-churn router backbone at steady state**; the edge stays best-effort. Determinism holds only at quiescence ([issue A](../issues/class-A-algorithm.md)).
2. **RT bought differently.** For the safety-critical class, hardness comes from **over-provisioning + path pinning + FRER replication + ATS shaping** — spatial redundancy and reservation, *not* reliance on transient consensus. Trades reroute-agility for jitter-freedom.

**Gating realities to clear first:** silent-failure 10 s detection ([R1](../issues/class-R-runtime.md)), f64 determinism ([J](../issues/class-A-algorithm.md)), no shaping/preemption/policing ([R2](../issues/class-R-runtime.md)/[R7](../issues/class-R-runtime.md)/[R9](../issues/class-R-runtime.md)), single-topology routing. None fatal; all sequenced in the [phases](../phases/).

**Recommended first cut for an RT deployment:** phases 01 (measure) → 03 (SR ext) → 04 (FRR + fast liveness) → 05 (3-class topology + drop-oldest + bounded batch) → 06/07 (GBR + deadline-aware admission) on a **bounded backbone**, RT flows pinned + FRER-replicated. That yields deterministic, bounded-latency, BW-guaranteed, mixed-flow routing for the critical class while the edge degrades gracefully.

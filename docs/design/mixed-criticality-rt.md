# Deterministic RT for mixed-criticality flows

**Status:** exploration. How the design serves a *mix* of flows with different criticality on one network — the robotics / autonomous-vehicle case (Zenoh runs under ROS 2 / Autoware). Grounded in TSN / DetNet / network calculus.

## 1. The flow archetypes

| Flow | Latency | Jitter | Loss | Bandwidth | Pattern | Criticality |
|------|---------|--------|------|-----------|---------|-------------|
| **Critical control** (cmd, actuation, e-stop) | **hard bound** (ms) | low | **zero** | low (kbps) | periodic / sporadic | safety |
| **HF sensor** (lidar, camera, radar) | soft bound | medium | **tolerant** — freshness ≫ completeness | high (Mbps–Gbps) | periodic high-rate | important |
| **Background** (map / OTA / logs) | none | n/a | zero but **elastic** | high volume | bulk | low |
| **Events / alerts** | urgent | low | low | very low | sporadic | high |

The requirement vector is `(latency-bound, jitter, loss-tolerance, bandwidth, deadline, criticality)`. **Mixed-criticality = isolation + per-class objective**: a sensor burst or a bulk download must never break the control flow's bound.

## 2. Per-class mechanism mapping

| Flow | Zenoh priority | Reliability | Congestion | This design | TSN/DetNet analog |
|------|----------------|-------------|------------|-------------|-------------------|
| control | RealTime / InteractiveHigh | Reliable | never-full (via admission) | **pinned** SR path + reserve + **FRER 1+1** + deadline admission | TAS/preemption + 802.1CB + DetNet |
| sensor | DataHigh | BestEffort | Drop (**want drop-oldest**) | bounded-rate reserve + loose path + optional FEC | CBS shaping + best-effort |
| background | Background | Reliable | Block/Drop | scavenger, spare-capacity Flex-Algo, no reserve | best-effort class |
| events | InteractiveHigh | Reliable | Block-short | sporadic, high incumbency | scheduled/express |

## 3. The isolation problem — 3 attack surfaces on control's bound

Control's latency bound can be broken three ways; each needs a mechanism:

1. **Queueing behind equal/higher priority** → **strict priority** (Zenoh has it, `pipeline.rs:843`) **+ admission cap** on the higher-priority *aggregate rate* (so the bound's residual rate stays positive).
2. **Head-of-line blocking by an in-flight lower-priority frame** (non-preemptive serialization — a big sensor/map batch is mid-transmission when a control frame arrives) → **frame preemption** (802.1Qbu/802.3br: blocking 12.5 µs → ~1 µs @1 Gb) **or** bound the low-priority frame/batch size. **Zenoh gap** (§6): no preemption; a low-prio *batch* blocks.
3. **Link congestion** → **admission + BW reservation** + **rate-limit the sensor** (token bucket, [R2](../issues/class-R-runtime.md)) so a sensor burst can't eat control's headroom.

## 4. Deterministic latency = network calculus

The formal tool (DetNet/TSN use it): shape each flow at ingress to a **token-bucket arrival curve** `α(t) = r·t + b`; each hop offers a **rate-latency service curve** `β(t) = R·(t − T)₊`. Worst-case per-hop delay = max horizontal distance between them:

$$
D_i \;=\; T_i + \frac{b_i}{R_i},\qquad
\underbrace{R_i = C - \!\!\sum_{j\,\text{higher prio}}\!\! r_j}_{\text{residual rate}},\qquad
\underbrace{T_i = \frac{L^{\text{lower}}_{\max}}{C} + \frac{\sum_{j\,\text{higher}} b_j}{C}}_{\text{blocking + higher burst}}
$$

End-to-end bound over a **pinned** path:
$$
D^{\text{e2e}} = \sum_{\text{hops}} D_i + \text{prop},\qquad \le \text{deadline ?}
$$

What each mechanism does to the bound:
- **Frame preemption** → `L^{lower}_max → ~1 fragment` → kills the blocking term `T_i`.
- **Admission cap** on higher-priority `Σ r_j` → keeps `R_i > 0` and finite → bound exists.
- **Ingress shaping** (token bucket) → bounds `b_i`, `r_i` → bound computable.
- **Pinned path** (strict SR segments, DetNet "explicit routes that don't change with topology") → fixes the hop set → `Σ_hops` is bounded and stable (no reroute mid-flow).

**This makes admission *deadline-aware*, not just bandwidth-aware** ([phase 07](../phases/07-deterministic-admission.md)): admit a control flow iff every control flow's `D^{e2e} ≤ deadline` still holds under the shaped load — a network-calculus feasibility check, deterministically computable from the flooded shaped-rate DB. Same deterministic-DB trick, now carrying `(r,b)` per flow.

## 5. Loss differentiation

- **Control → zero loss:** `Reliable` + **FRER** (802.1CB): replicate over **disjoint** paths, sequence-number, eliminate duplicates at the receiver. Survives congestion *and* link failure with no gap. This is our §4.3 duplication / DetNet **PREOF** — name it FRER. Costs 2× BW on a tiny flow → cheap.
- **Sensor → loss-tolerant, freshness-first:** `BestEffort` + `Drop`. But **drop the *oldest*, not the newest** — a fresh lidar frame matters more than a stale queued one. **Zenoh gap** (§6): `Drop` discards the *incoming* (newest) message. Optional FEC for partial recovery without retransmit latency.
- **Background → reliable elastic:** `Reliable`, scavenger priority, retransmit fine, no deadline. Rides spare capacity (widest-path Flex-Algo), yields to everything.

## 6. Zenoh reality gaps for hard-RT (grounded)

| Gap | Today | Impact | Fix |
|-----|-------|--------|-----|
| **No frame preemption** | 8 strict-priority queues but non-preemptive at batch level | control jitter bounded by max low-prio batch serialization (the `T_i` blocking term) | cap low-prio batch size, or interleaved fragmentation (preemption-like) — [new R7](../issues/class-R-runtime.md) |
| **Drop = newest** | `pipeline.rs` drops the incoming msg on full queue | sensor keeps **stale**, drops **fresh** — wrong for freshness | drop-oldest mode per class — [new R8](../issues/class-R-runtime.md) |
| **Block waits 5 s** | `wait_before_close` = 5 s | useless for an RT deadline | control uses priority+admission (never full), not Block |
| **2 batches/queue** | shallow | low latency (good) but bursty sensor drops fast | per-class queue depth |
| **No time-aware shaper (TAS)** | — | no TDMA-style time-gated isolation | **prefer async**: strict priority + preemption + **ATS** (802.1Qcr) — clock-free, fits Zenoh's no-sync stance |
| **No clock sync** | — | TAS needs synced clocks | **feature, not bug** — use **Asynchronous Traffic Shaping**; no global clock, like our no-central-admin / HLC stance |
| **Hop-by-hop, no e2e** | local backpressure only | no runtime e2e enforcement | network-calculus admission precomputes the bound (§4) |

**Why ATS over TAS for Zenoh:** TAS (802.1Qbv) needs network-wide synchronized clocks + a computed gate schedule (802.1Qcc, centrally). That fights Zenoh's decentralized, no-sync, no-admin grain. **Asynchronous Traffic Shaping (802.1Qcr)** gives bounded latency *without* synchronized clocks — the same philosophical fit as deterministic-from-flooded-DB and HLC (no central clock). Recent in-car-network work shows ATS + redundancy avoids unbounded latency. Recommend ATS, not TAS.

## 7. The recipe (putting it together)

1. **Classify** flows → priorities: control→RealTime/InteractiveHigh, sensor→DataHigh, background→Background, events→InteractiveHigh.
2. **Per-class topology** ([Flex-Algo](sr-stateless-features.md#b), phase 05): control→min-latency **pinned** tree; sensor→high-BW tree; background→spare-capacity tree.
3. **Shape at ingress** (token bucket per flow, [R2](../issues/class-R-runtime.md)): control `(r_c,b_c)` tiny; sensor `(r_s,b_s)` bounded; background unshaped scavenger.
4. **Admit deadline-aware** (network calculus §4, phase 07): control reserved with top **incumbency** (HLC epoch); sensor bounded-rate reserve; background best-effort.
5. **Protect control**: strict SR-pinned path (no reroute) + **FRER 1+1** disjoint replication + frame preemption / bounded low-prio batch.
6. **Sensor**: loose path, drop-oldest, optional FEC, yields to control.
7. **Background**: scavenger, spare capacity, retransmit, no reservation.

Result: control gets a **provable** `D^{e2e} ≤ deadline` and zero loss; sensor gets bounded-rate high throughput with freshness; background soaks leftover — all isolated, all on one Zenoh network.

## 8. Caveats (the recurring thesis)
- Hard bounds need the **3 isolation mechanisms all present** — Zenoh has strict priority but **lacks preemption, drop-oldest, and shaping** (§6). Without them, control's bound is *soft*, not hard.
- The network-calculus bound assumes flows stay within their shaped `(r,b)` → **enforcement (policing) is mandatory** ([R2](../issues/class-R-runtime.md)), else the math is fiction.
- Pinned paths give determinism but **forgo the two-timescale reconvergence** for control — on failure, FRER (spatial redundancy) covers the gap instead of reroute. Trade reroute-agility for jitter-freedom on the critical class.
- Everything else inherits the **steady-state-hard, transient-best-effort** caveat; for control we buy hardness with *over-provisioning + replication* rather than relying on transient consensus.

## References
- TSN Time-Aware Shaper (802.1Qbv) + Frame Preemption (802.1Qbu/802.3br) — survey https://arxiv.org/pdf/1803.07673
- Asynchronous Traffic Shaping (802.1Qcr) + redundancy in-car — https://arxiv.org/pdf/2504.01946
- Network calculus for TSN/DetNet (arrival/service curves, delay bounds) — https://arxiv.org/pdf/2204.10906 ; deadline-aware admission — https://arxiv.org/pdf/2503.09093
- IEEE 802.1CB FRER (replication/elimination) — https://en.wikipedia.org/wiki/Time-Sensitive_Networking ; worst-case delay with PREOF — https://arxiv.org/pdf/2110.05808
- RFC 8655 DetNet — https://www.rfc-editor.org/rfc/rfc8655.html
- Zenoh + Autoware — https://autoware.org/driving-autoware-with-zenoh/

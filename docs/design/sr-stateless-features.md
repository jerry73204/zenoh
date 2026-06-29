# Features enabled by Segment Routing + stateless design

**Status:** exploration. Catalog of capabilities the SR + deterministic-DB + two-timescale design unlocks *beyond* the core bandwidth/priority/fault work ([qos-fault-routing.md](qos-fault-routing.md)). Each grounded in related work.

Two primitives generate almost everything below:
- **SID = a function, not just a location** (SRv6 network programming, RFC 8986) — a segment can mean "go to X" *or* "do Y".
- **Deterministic computation from a flooded DB** (IGP Flex-Algo) — every node derives the same constrained paths from flooded definitions, no controller.
…composed with the **two-timescale** model (SR transient, link-state permanent, [qos §8.7](qos-fault-routing.md)).

---

## A. In-network processing / service function chaining
**Related work:** SRv6 Network Programming (RFC 8986) — a SID = Locator + Function + Args; a SID can bind to a VM/container applying arbitrary processing. Segment list = an ordered service chain; packet traverses it by SR forwarding mechanics.

**Zenoh mapping:** route a publication through a pipeline of function-SIDs — `downsample → encrypt → aggregate → deliver`. Or steer a query through compute nodes. This realizes Zenoh's "data in motion **+ computation**" pillar via the same segment stack already used for detours. The chain lives in the packet ⇒ processing nodes hold **no per-flow state**.

**Builds on:** P2 (segment stack). **New:** a SID namespace where some SIDs map to local functions (Zenoh plugins / queryables). **Caveat:** function-SIDs are stateful *locally* (the function may be); the *forwarding* stays stateless.

## B. Per-class / per-slice constraint topologies (this validates our model)
**Related work:** **IGP Flexible Algorithm** (`draft-ietf-lsr-flex-algo`). A *Flexible Algorithm Definition* (FAD = calc-type + metric-type + constraints) is **flooded** in IGP TLVs; every router independently computes the constrained paths. Used for **network slicing**.

**This is the standardized form of our design** — deterministic constrained-path computation from a flooded definition, no controller. Our 3-class MTR (phase 05) *is* three Flex-Algos. **Adopt FAD-style flooded constraint definitions instead of inventing** — latency-algo, avoid-region-algo, low-loss-algo, each a slice. Per-tenant/per-app slices fall out directly.

**Builds on:** phase 05, phase 07 (deterministic compute). **New:** flooded FAD-like definitions + per-algo SID space.

## C. Stateless in-band telemetry
**Related work:** **IOAM** in SRv6 (RFC 9259) and **multicast on-path telemetry** (RFC 9630). Packets record path + per-hop latency/load **in-band**; selective — only nodes whose SID has the IOAM argument insert data.

**Zenoh mapping:** packets accumulate path + per-hop load as they traverse → **measure the overlay with zero per-flow counters.** This directly feeds the **capacity signal** (P4 / R3) and the **economy model's observability gap** (unbiased `n̂`, capacity estimation — [open-questions](../issues/open-questions.md)). RFC 9630 measures *multicast trees* — exactly Zenoh's pub/sub distribution trees.

**Builds on:** P2/P4; produces input for phase 01. **Strong fit** — turns the hardest measurement problems into a stateless data-plane byproduct.

## D. Deterministic / bounded-latency delivery
**Related work:** **DetNet** (RFC 8655): bounded latency + near-zero loss via (1) resource reservation, (2) **explicit routes that don't change with topology**, (3) replication over space. SR steers DetNet flows and gives per-hop instructions for bounded latency. "1+1 protected DetNet flows" = **PREOF** (Packet Replication, Elimination, Ordering).

**Zenoh mapping:** explicit SR path + admission (phase 07) + per-hop priority = **bounded e2e latency** for real-time pub/sub (robotics, industrial). Our §4.3 disjoint-path duplication **is** DetNet PREOF — name it so. **Tension to resolve:** DetNet wants paths *pinned* (don't reconverge); our two-timescale model *does* reconverge. ⇒ for hard-RT flows use **strict** segments (pinned, no reroute); for elastic use **loose**. Per-flow choice.

**Builds on:** phases 06/07 + §4.3. **Caveat:** hard bounds need bounded jitter per hop → ties to the queue/scheduling layer (8 priorities, but only 2 batches/queue — small buffers help latency).

## E. Stateless mobility
**Related work:** **LISP** loc/ID split (RFC 9300/9301): identity separated from location; an endpoint roams, the mapping updates, traffic redirects **without per-flow tunnel state**.

**Zenoh mapping:** `ZID` = identifier (stable across reconnect), face/locator = location (changes on roam). Subscriber/publisher moves → DB updates → trees recompute → **no per-flow tunnel to migrate**. SR adds the **handoff bridge**: during convergence, segment-redirect in-flight messages to the new attachment (the two-timescale model applied to mobility — SR bridges until the DB catches up).

**Builds on:** existing ZID stability + P2. **Fit:** Zenoh already has half of this; SR closes the handoff gap.

## F. Path-pinned ordering (no reorder)
Pin a flow to **one** explicit path (strict segments) → no reordering → strengthens Zenoh `Reliable` ordered delivery and sidesteps the ECMP-reorder problem ([S5](../issues/oscillation.md)). Cheap, per-flow opt-in.

## G. Policy / jurisdiction / geofencing routing
Loose segments **forbidding** SIDs outside a region → keep data within a jurisdiction (GDPR-style data residency), avoid untrusted nodes, stay in a cost domain. Declaratively = Flex-Algo **exclude-link / affinity** constraints (B). Stateless: the policy is in the path/definition, not per-node ACL state.

## H. Anycast / nearest-replica selection
**Related work:** anycast SID = nearest instance. **Zenoh mapping:** route a query to the **nearest storage replica** (storage-manager plugin) or nearest queryable via an anycast SID resolved from the DB — no session state. Consistent-hash ([S5](../issues/oscillation.md)) for replica-choice stability.

## I. Proof-of-transit / path verification
**Related work:** SRv6 SFC verification (ordered proof-of-transit). Verify a packet **actually traversed** the required chain → compliance/security (did this data really pass the encryption/audit function?). Stateless: in-packet ordered proof.

---

## The design axis: stateful (NDN) vs stateless (us)
**Related work — the counterpoint:** Named Data Networking forwards by *name* (like Zenoh key-exprs) but keeps **per-Interest state** — the PIT (Pending Interest Table) — at every hop. That state buys native multipath, loop-freedom, fast local failure detection, and in-network caching ("A Case for Stateful Forwarding", NDN TR). The opposite camp ("A Case Against Stateful Forwarding in CCN") argues the per-packet state cost isn't worth it.

**Where this design sits:** Zenoh names data like NDN, but the SR + deterministic-DB choice is **stateless-leaning** — adaptivity comes from the flooded DB + packet-carried path, not per-hop PIT state. The trade is explicit:

| | NDN (stateful PIT) | This design (SR + deterministic DB) |
|--|--------------------|-------------------------------------|
| forwarding state | per-Interest, per-hop | none in transit (path in packet) |
| multipath / failure adapt | native, per-hop local | via DB reconverge + SR bridge ([§8.7](qos-fault-routing.md)) |
| consistency | always (local state) | only at quiescence (issue A) |
| crash rejoin | rebuild PIT | pull DB, recompute ([§12](qos-fault-routing.md)) |
| caching | native (ContentStore) | separate (storage plugin) |

So the stateless choice **buys fast rejoin + low core state**, at the cost of **transient-only consistency** — the recurring thesis. NDN trades the other way. Neither dominates; this documents *why* Zenoh's grain (low-state, embedded targets) favors stateless.

---

## Feasibility summary

| Feature | Related work | Builds on | New wire | Difficulty |
|---------|--------------|-----------|----------|------------|
| A in-network processing/SFC | RFC 8986 | P2 | function-SID space | medium |
| B per-slice topologies | IGP Flex-Algo | ph05, ph07 | flooded FAD + algo-SIDs | medium |
| C stateless telemetry | RFC 9259/9630 (IOAM) | P2/P4 | IOAM header | **low, high value** |
| D bounded latency (DetNet) | RFC 8655, PREOF | ph06/07, §4.3 | strict-segment flag | high |
| E stateless mobility | LISP RFC 9300 | ZID + P2 | (handoff redirect) | medium |
| F path-pinned ordering | SR strict path | P2 | strict flag | low |
| G policy/jurisdiction | Flex-Algo affinity | B | constraint defs | low–medium |
| H anycast replica | anycast SID | DB + storage plugin | anycast SID | medium |
| I proof-of-transit | SRv6 SFC verify | A | proof field | medium |

**Highest value, lowest cost: C (stateless telemetry)** — it makes the design's hardest measurement problems (capacity, `n̂`, tree load) a free data-plane byproduct, feeding phase 01 and the economy model. Recommend exploring it next.

## Cross-cutting observations
- Most features = **"SID = function"** (A, H, I) or **"flooded constraint definition"** (B, D, G) — two ideas, many features.
- All inherit the same caveat: **deterministic/stateless = consistent at quiescence only** (issue A). Telemetry (C) helps by measuring the actual transient state.
- Flex-Algo (B) is independent validation that the core design (deterministic constrained-path from flooded DB) is a real, standardized architecture — not a novel gamble.

## References
- RFC 8986 — SRv6 Network Programming — https://www.rfc-editor.org/info/rfc8986/
- IGP Flexible Algorithm — https://datatracker.ietf.org/doc/draft-ietf-lsr-flex-algo/
- RFC 9259 — OAM in SRv6 (IOAM) — https://www.rfc-editor.org/rfc/rfc9259.html ; RFC 9630 — Multicast On-Path Telemetry (IOAM) — https://datatracker.ietf.org/doc/rfc9630/
- RFC 8655 — Deterministic Networking Architecture — https://www.rfc-editor.org/rfc/rfc8655.html ; SR for Enhanced DetNet — https://www.ietf.org/archive/id/draft-geng-spring-sr-enhanced-detnet-00.html
- RFC 9300 — LISP (Locator/ID Separation) — https://datatracker.ietf.org/doc/rfc9300/
- NDN stateful forwarding — https://named-data.net/wp-content/uploads/TRforward.pdf ; A Case Against Stateful Forwarding in CCN — https://arxiv.org/pdf/1512.07755

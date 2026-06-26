# Bandwidth-Aware, Priority-Enabled, Fault-Tolerant Routing

**Status:** exploration / design note. Not implemented. Companion to [auto-routing-economics.md](auto-routing-economics.md).

## 0. The remembered idea — name it

> "push a destination stack on the packet when we want to detour it"

That is **Segment Routing (SR / SRv6)**: the source (or a repair node) pushes an ordered *list of segments* — waypoints/destinations — onto the packet; each segment endpoint pops the top and forwards toward the next. The detour-on-failure special case is **TI-LFA** (Topology-Independent Loop-Free Alternate): when the normal backup would loop, push a segment stack that steers the packet around the failed element to a safe node, then on to the destination.

Same family, for reference:

| Mechanism | What's on the packet | Detour fit |
|-----------|----------------------|-----------|
| **Segment Routing (SR/SRv6)** | ordered segment list (node/adjacency SIDs) | **exact** — insert a waypoint before dst |
| **MPLS label stack** | label stack, swap/pop per hop | label-switched equivalent |
| **IP loose source route (LSRR)** | list of loose hops | the "loose detour" semantics |
| **Pathlet routing** | sequence of pathlet IDs | detour = swap a pathlet |
| **TI-LFA / FRR** | repair segment stack pushed on failure | the fault-tolerance + detour combo |

**Zenoh already has the 1-element version.** The `NodeId` routing-context extension (`commons/zenoh-protocol/src/network/push.rs:63`, ext `0x3`, `NodeIdType.node_id: u16`) carries a single routing hint on Push/Request/Interest/Declare. SR generalizes that one `u16` into a **stack of `u16` segment IDs**. This is the smallest wire change that unlocks all three features below — they all reduce to "steer this packet along an explicit partial path."

---

## 1. Current state (grounded)

| Concern | Today | Gap |
|---------|-------|-----|
| **Priority** | 8 levels (`core/mod.rs:332`); **strict-priority TX scheduling only** (`pipeline.rs:843` `trailing_zeros()`); 3-bit `P_MASK` on wire (`network/mod.rs:460`) | priority does **not** affect *path* — same tree for RT and bulk |
| **Reliability/Congestion** | BestEffort/Reliable, Drop/Block/BlockFirst — **local queue policy** (`core/mod.rs:508,617`) | no path-level reliability (no backup path, no FEC across paths) |
| **Bandwidth** | static link weight `u16`, default 100 (`linkstate.rs:69`); SPF minimizes ~hop-count | **no capacity/utilization signal**; congested links chosen freely |
| **Faults** | link drop → `sn++` → LSP flood → `compute_trees()` 100 ms debounce (`network.rs:952,1015`) | **no fast-reroute**; packets dropped during reconvergence (flood RTT + 100 ms) |
| **Multilink** | multiple links/transport, failover (`unicast/.../multilink.rs`) | link-level only; no BW split, no path-level use |
| **Source routing** | single `NodeId` hint, minimal remap (`hat/mod.rs:212`) | not a path; no waypoint stack |

Summary: QoS today is **scheduling**, not **routing**. All three asks require pushing QoS into path selection.

---

## 2. Bandwidth-aware routing

### 2.1 Need a capacity/load signal
Static weight 100 can't express congestion. Add per-link measured residual bandwidth `R_e = C_e − λ_e` (capacity − load), sampled from TX pipeline counters (`pipeline.rs` per-priority `congested`/`pending` bits already track pressure). Propagate in the existing `link_weights` LSP field (`network.rs:148`).

### 2.2 Two routing objectives
- **Min-cost (additive):** convex congestion cost per link. The textbook M/M/1 delay `w_e = 1/(C_e−λ_e)` has **unbounded slope** at saturation — this is the ARPANET metric known to **oscillate** (§10.9, S1). Use instead a **bounded** metric with a static floor:
  $$w_e=\text{bias}+\alpha\cdot\text{util}_e,\quad \text{util}_e=\lambda_e/C_e,\ \ \alpha\ \text{capped (HN-SPF style)}$$
  The static-to-dynamic ratio (`bias` vs `α`) is the stability knob. SPF (Dijkstra, weights ≥0 → drop Bellman-Ford, `O(e\log n)`) over `w_e` = load-balanced path.
- **Widest-path (bottleneck):** maximize the min residual capacity along the path
  $$\text{path value}=\min_{e\in P}R_e,\qquad \text{maximize over }P$$
  Modified Dijkstra (max-min relaxation). Use for elastic/bulk flows that want throughput, not latency.

Lexicographic combine: *feasible bandwidth first, hop-count tiebreak* — avoids picking a 10-hop detour to save 1% load.

### 2.3 Oscillation — the classic trap
Dynamic weights cause **routing oscillation** (ARPANET 1987 delay-metric flapping): everyone moves to the idle link → it congests → everyone moves back. Mitigations, all cheap:
- **EWMA smoothing** of `R_e` (slow the signal).
- **Quantize** weight into few bands (the `u16` already forces this) — coarse weights don't chase noise.
- **Hold-down / hysteresis** band `κ` (same device as the economy model §6.2) before re-advertising.
- **Partial response:** shift only a fraction of flows (à la TeXCP / MATE) rather than all-or-nothing.

### 2.4 Economy tie-in
Dynamic weights raise the churn rate `λ` in [auto-routing-economics.md §3](auto-routing-economics.md) → more LSP floods + tree recomputes → pushes the gossip↔linkstate threshold **toward gossip**. So BW-awareness is not free: it taxes the control plane. Budget it: cap re-advertisement rate, or only BW-route the heavy classes (§3).

Dual view: the link weight `1/(C_e−λ_e)` is the **shadow price** (Lagrange multiplier) of the capacity constraint. Routing to min total priced cost is the dual of max-utility flow assignment (Kelly / NUM). Priority = utility weight (§3.3).

---

## 3. Priority-enabled routing

### 3.1 Multi-Topology Routing (MTR)
Named concept: **OSPF/IS-IS Multi-Topology Routing**. Compute a *separate* weight set + SPF tree per traffic class:
- **RealTime class** → weights = latency (`w_e = delay_e`), shortest-delay tree.
- **Data class** → weights = balanced cost.
- **Bulk/Background** → weights = spare-capacity / monetary cost (widest-path, route through cheap idle links even if longer).

Forwarding reads the 3-bit `P_MASK` already on the wire and indexes the per-class tree.

### 3.2 Don't keep 8 classes — collapse to ~3
`compute_trees()` is `O(n²)` memory/CPU per class (verified in economy note §3). `K` classes → `K·n²`. With 8 priorities that's 8× the dominant cost — uneconomic. Map 8 priorities → **3 routing classes** {RT, Data, Bulk}; keep all 8 for *TX scheduling* (cheap, local) but only 3 for *path computation* (expensive, global). The mapping is a small table; tune by per-class traffic share (only split a class out when its `T_class` clears the §-threshold).

### 3.3 Fairness / economy
Priority in routing = **weighted fairness**. Weighted max-min or weighted proportional fairness with weight `β_class`:
$$\max\ \sum_{\text{flows }f} \beta_{class(f)}\,U(x_f)\quad\text{s.t. } \sum_{f\ni e}x_f\le C_e\ \forall e$$
RT gets large `β` (served first / shortest path), Background small `β` (scavenger, only spare capacity). This is the global analogue of the local strict-priority scheduler already in `pipeline.rs`.

---

## 4. Fault-tolerant routing

### 4.1 Problem with today's model
Failure recovery = detect → `sn++` → flood LSP → wait `≤100 ms` debounce → `compute_trees()` → install. During that window (flood propagation + 100 ms) traffic on the broken tree is **dropped**. No backup.

### 4.2 Fast ReRoute (precompute the backup)
Compute, *ahead of failure*, a backup next-hop per (destination, protected-link):
- **LFA (Loop-Free Alternate):** a neighbor `N` is a valid backup for dst `D` if `d(N,D) < d(N,me)+d(me,D)` (N won't loop back). Pure local check, precomputed.
- **Remote-LFA:** when no direct LFA, tunnel to a remote node that has one.
- **TI-LFA (the destination-stack idea):** when topology offers no LFA, **push a segment stack** routing the packet to the post-convergence path's safe node (P-space∩Q-space), then to dst. Guarantees 100% coverage for single failures, and the repair path == the eventual converged path (no transient churn).

On detection (link-down event, or transport multilink failover signal), switch to the precomputed backup in ~microseconds — *no waiting for reconvergence*. Reconvergence then happens in the background to restore the optimal tree.

### 4.3 Synergy with what exists
- **Multilink** (`transport/.../multilink.rs`) already gives link-level failover — use its failure signal as the FRR trigger.
- **Reliable** delivery + FRR = packets survive single failures without app-visible loss.
- For critical streams: **disjoint-path duplication** — push two segment stacks over edge-disjoint trees, dedup at receiver (1+1 protection / FEC). Costs 2× bandwidth; gate on the `Reliable`+high-priority combo only.

---

## 5. Segment Routing unifies all three

One mechanism — a **NodeId/segment stack** on the packet — serves BW-TE, priority steering, and fast-reroute:

```
wire change: NodeIdType { node_id: u16 }   →   SegmentList { segs: SmallVec<u16> }
             (ext 0x3, push.rs:63)              top = next waypoint; pop on arrival
```

| Use | How the stack is used |
|-----|----------------------|
| **BW-aware TE** | source pins flow through under-utilized waypoint(s), bypassing congested core |
| **Priority steering** | RT flow gets explicit low-latency waypoint list; bulk gets empty stack (default SPF) |
| **Fault detour (TI-LFA)** | repair node pushes waypoint(s) around the failure |
| **Policy / fault domain** | route around an untrusted/failing region by forbidding its SIDs |

Why it fits Zenoh:
- **Stateless-ish core** — path state moves from nodes (per-flow tables) into the packet. Matches Zenoh's zero-overhead, low-state ethos and *reduces* the `n²` per-node state pressure from the economy note.
- **Loose segments** ("reach waypoint W somehow", SPF fills the gaps) → small stack, survives intra-segment failures, cheap. Prefer loose over strict.
- **Incremental** — empty stack == today's behavior. Deploy per-flow, opt-in.

Cost: extra header bytes = `segdepth × 2`. Tradeoff is depth vs state — SR's entire value proposition. Keep stacks shallow (1–3 segments covers most TE/FRR).

---

## 6. What to build (risk-ordered)

1. **Capacity signal:** export residual-BW from TX pipeline counters; carry in `link_weights` LSP. Read-only, no behavior change — measure first.
2. **BW-aware weights + damping** (§2): swap static 100 for EWMA-smoothed congestion cost, Dijkstra, hold-down. Single-topology. Reversible.
3. **Segment stack wire ext** (§5): extend ext `0x3` to a list; empty == legacy. Forwarding pops top. Enables everything else.
4. **TI-LFA fast-reroute** (§4): precompute LFA backups; on failure push repair stack. Biggest resilience win.
5. **3-class MTR** (§3): per-class weight sets + trees, mapped from `P_MASK`. Most expensive (`K·n²`) — last, and only for classes that earn it.

## 7. Costs & tradeoffs (feed back into economy model)

- BW-aware weights ↑ churn `λ` → ↑ control cost → shifts gossip↔linkstate threshold ([economy §3](auto-routing-economics.md)). Budget re-advertisement rate.
- MTR multiplies tree cost by `K` → only 3 classes, only where traffic justifies.
- Segment stacks **lower** per-node state (path in packet) but **raise** per-packet bytes — net win when flows are long-lived (amortize header over many packets), loss for tiny one-shot messages.
- FRR backups cost precompute (extra SPF runs) + backup-table memory `O(n)` per protected dst — modest vs the `n²` base.

## 8. BW-guaranteed multicast — deterministic distributed design

Goal: per-flow guaranteed bitrate over **multicast** trees, **stateless** (fast crash-rejoin), **no central admin** (routers OK, no SPOF).

### 8.1 Two pillars

**Pillar 1 — forwarding state lives in the packet** (SR segment stack for unicast detour/repair; **implicit key-expr forwarding** for multicast distribution — see §8.5, *not* a per-packet destination set). Transit peers hold no per-flow state; they forward by reading the packet (or re-resolving the key-expr). Crash → resume from topology DB alone.

**Pillar 2 — admission state is *derived*, not reserved.** Make tree computation a deterministic pure function of the flooded declaration DB (Zenoh already does this in `compute_trees()`). Extend the DB so declarations carry: publisher GBR `b_f` (soft-state, refreshed), subscriber locations (already present), per-link capacity `C_e` (config, flooded). Then per-link committed load is a pure function of the DB:

$$
\text{load}(e)=\sum_{f\,:\,e\in \text{tree}(f,\,DB)} b_f
$$

Multicast-correct: a flow counts **once per link** its tree crosses, not once per subscriber.

### 8.2 Determinism replaces signaling

No central PCE and no RSVP-style hop-by-hop signaling. Instead every router runs the **same** admission function on the DB. Flows are admitted in a global total order on **immutable** keys only:

$$
f \prec g \iff (\text{prio}_f,\ \text{ZID}_f) <_{\text{lex}} (\text{prio}_g,\ \text{ZID}_g)
$$

**Not seqno** — seqno changes on every soft-state refresh, and putting it in the order makes the preempted flow re-win on refresh → ping-pong (§10, S3). seqno is used only for staleness/freshness detection, never for ordering. Admit flows in `≺` order, each consuming residual capacity left by predecessors. Every node independently reaches the **same** admit/reject/preempt decision ⇒ routers are **replicated deterministic PCEs**, consistent by determinism not coordination. No SPOF: any router dies, others already hold identical derived state.

- **Preemption with no notification:** a lower-priority flow that loses the `≺` contest on an oversubscribed link is preempted; its source computes the same function → independently knows → recomputes its own detour. No PathErr loop.
- **Fast rejoin:** restarted peer floods "back" → neighbors re-advertise DB → peer recomputes trees+loads+forwarding in one flood cycle.
- **Failure detour:** TI-LFA repair stack (packet-carried, deterministic) + per-class protection BW reserved deterministically so the detour isn't itself congested.

### 8.3 The determinism invariant (load-bearing — and fragile)

Consistency holds **iff every node computes a byte-identical function on an identical DB**. Both halves are violable:

- **Identical function** → integer/quantized BW only (no f64: nondeterministic across archs/compilers). Algorithm must be **versioned**; mixed versions during rolling upgrade = split-brain admission (§9, issue I).
- **Identical DB** → false in general. Link-state floods are **eventually** consistent; at any instant nodes hold different snapshots ⇒ they compute different admissions *concurrently*, not just transiently (§9, issue A). **Hard guarantee holds only at quiescence**; under churn the guarantee is statistical, degrading as `churn_rate × convergence_time` grows.

### 8.4 Guarantee strength is scoped, not global

Because §8.3, hard guarantees survive only in a **bounded, low-churn admission domain**. Map that to the **router backbone** (small `n_R`, bounded flow count, slow topology change) where the user already accepts routers. Edge peers/clients get **best-effort** BW. This also bounds the recompute cost (§9 issue E). Realistic framing: *hard guarantee on the backbone at steady state; best-effort at the edge and during reconvergence.*

### 8.5 Multicast = implicit key-expr forwarding + scoped SR repair

Don't put destinations on the packet (BIER bitmap) — it scales with `|receivers|`. Instead keep Zenoh's **implicit multicast** and tag only the *exception*.

**Normal path = unchanged Zenoh.** Packet carries the **key-expr** (resource ID + suffix) and a **tree/source context** (the existing `NodeId` routing-context, ext `0x3`). Each node resolves key-expr → its **children in that source's distribution tree** (`compute_data_route`), duplicates to those faces. Destination is implicit in the key-expr; the single per-source tree assigns each subscriber exactly one parent. No bitmap, no segment list, no per-flow state.

**Detour = key-expr + coarse scope.** When node `X` can't forward a branch (link full/down), it does **not** carry the subscriber list. It carries a **scope** = the edge-node-set / region of the blocked branch, computed from its link-state distribution tree (`compute_trees` — `X` knows which edge nodes sit behind each output face because subscriber declarations propagate). `X` SR-tunnels `(key-expr, scope, detour-flag)` to a node `Y` on an alternate path toward that scope. `Y` resumes normal forwarding **restricted to scope**.

Destination identity is therefore:
$$
S=(\text{key-expr match})\cap(\text{topological scope})
$$
Key-expr = identity (reused resolution); scope = which partition. Header cost: **nothing** on normal packets; a **coarse region scope** only on detoured ones — independent of `|receivers|`.

Prereq: computing scope and picking `Y` needs subscriber *locations* at edge-node granularity → only the **linkstate/router tier**. A **gossip peer that can't compute scope detours upward to its router**, which can.

*(BIER remains a fallback only for very large, sparse, flat receiver sets where even edge-node-scoped trees are unwieldy — see §10. For the common case, scoped detour is smaller and reuses existing machinery.)*

### 8.6 Detour correctness — no duplicates, no gaps

The governing rule: **forward by delegated responsibility, never by greedy key-expr match.** Output set of any packet at any node:
$$
\boxed{\ \text{out}(pkt)=\underbrace{\text{children\_in\_source\_tree}(\text{key-expr})}_{\text{prevents dup on normal path}}\ \cap\ \underbrace{\text{scope}\ (\text{if detour-flag})}_{\text{prevents dup on detour}}\ }
$$

Worked example — publisher `P`, subscribers `A`,`B`; `P→A` direct, `P→M→B`:

**(1) Why `M` doesn't duplicate to `A`.** One tree per `(P, key-expr)` assigns `A` to `P`'s direct branch, `B` under `M`. `A` is **not in `M`'s subtree**. `M` forwards only to its **children in `P`'s tree** (keyed by the tree-context on the packet) — not to all local key-expr matches. `M`'s children = {toward `B`}; `A` is never considered. **RPF backstop:** `M` accepts the packet only from the expected upstream face for `P`'s tree and forwards only downstream. A tree gives each leaf one path ⇒ duplication is structurally impossible. *(This already works in Zenoh today — the design must preserve the tree-context on the packet.)*

**(2) Why `Y`/`N` sends to `B` only, not the closer `A`.** Without scope, `N` would resolve key-expr → {`A`,`B`}, pick the closer `A`, and duplicate. The scope tag forbids it:
- detour packet carries `scope = {B's region}` + detour-flag;
- `N` forwards by `(key-expr match) ∩ scope = {A,B} ∩ {B} = {B}` — scope overrides proximity;
- `N` is in scoped-detour mode → **no greedy fan-out**, it completes only the delegated branch;
- **RPF exemption:** the detoured packet arrives via the SR tunnel, *not* `P`'s tree, so normal RPF would reject it → detour-flag bypasses RPF and trusts scope instead;
- scope + flag ride end-to-end until `B`'s region does local delivery; `A`'s region never sees this copy.

**Disjointness.** `P`'s single tree partitions {`A`,`B`}: `A`←served by `P` directly, `B`←served by `N` (scope-restricted). Union = {`A`,`B`}, disjoint ⇒ no dup, no gap. `A` cannot appear in `N`'s scope **because the same tree already gave `A` to `P`'s branch**, so `P` never delegated `A` ⇒ `scope ∌ A`.

**Two failure modes:**
1. **Inconsistent tree** (issue A — `P`,`M`,`N` hold divergent DB views) → the partition disagrees → `A` in two branches (dup) or zero (gap). Hard guarantee only at quiescence; best-effort during convergence. Load-bearing assumption.
2. **Reverting to greedy match on a detour packet.** If a downstream node drops the detour-flag/scope early, it re-fans-out to `A`. Enforce: **flag + scope ride until region-local delivery.**

## 9. Design issues (audit)

Severity: 🔴 fundamental · 🟠 major · 🟡 manageable.

| # | Issue | Severity | Note / mitigation |
|---|-------|----------|-------------------|
| A | **Eventual-consistency breaks the "same DB" premise.** Nodes never share an instantaneous DB under churn → concurrent *divergent* admission, not just transient. Ingress admits on its view; transit accounts on its view; they differ → oversubscription/black-hole. | 🔴 | Guarantee ∝ `1/(churn×convergence)`. Scope hard guarantees to low-churn backbone (§8.4). Quantify: require refresh/flood period ≪ inter-flow-change time. |
| B | **Accounting (predicted) vs forwarding (packet-carried) can diverge.** Packet follows ingress's pushed path; transit node's recomputed tree may differ → `load(e)` wrong where views differ. | 🔴 | Account load from **observed** stacks/bitmaps on packets (consistent) — but that's *reactive*, too late for *predictive* admission. Predictive admission needs consistent view (issue A). Inherent tension. |
| C | **Stale hard-state reservation after ungraceful crash** (revised — see §12). Zenoh is *hard-state, no refresh*, so there is **no** refresh-flood scaling problem. Instead a dead publisher's GBR lingers until the **face-close** cleanup, which is **lease-bound**: ~ms on TCP-close (fast path) but up to **10 s** on silent failure (lease timeout). During that window the dead flow's reservation wrongly rejects/preempts live flows. | 🟠 | Tie GBR lifetime to the declaring face (auto-undeclare on face close — already how subs are cleaned, `face.rs:728`). Lower lease/keepalive for GBR domains. No periodic refresh needed. |
| D | **Starvation & route-flap under churn.** `≺` lets high-prio churn permanently starve low-prio; preemption set changes every DB change → flap. "Unique fixed point" holds only for a *fixed* DB, which never occurs under churn. | 🟠 | Hysteresis/hold-down on re-admission; minimum holding time before a flow can be preempted; aging to prevent starvation. |
| E | **Admission recompute is sequential in flows.** Admitting flow k depends on residual from flows 1..k−1 (the `≺` order) → can't parallelize; one link flap re-runs the whole cascade `O(flows × CSPF)`. At scale, can't keep up with churn → never converges (feeds A). | 🟠 | Incremental recompute only for flows touching the changed link (approx; breaks strict global order). Scope to small backbone (§8.4). |
| F | **Capacity from config is fragile.** Real capacity varies (wireless, shared NIC, non-Zenoh cross-traffic). Configured `C_e` ≠ available → guarantees on wrong numbers. Measured capacity is non-deterministic → breaks consensus (§8.3). | 🟠 | Use **conservative** static `C_e`, accept under-utilization; or measure but quantize+smooth+flood as DB input (adds churn, issue A). |
| G | **No policing authority without central admin.** Publisher self-declares GBR; malicious/buggy pub over-declares (starves) or under-declares then floods (no shaping). | 🟠 | Ingress token-bucket policing (local, enforceable). Per-publisher quota needs decentralized agreement — open. |
| H | **Per-packet header overhead** (segment stack / BIER bitmap) is large for tiny Zenoh messages (sensor samples). | 🟡 | Empty header = legacy best-effort default; only GBR streams (long-lived, amortizing) carry it. |
| I | **Rolling-upgrade split-brain.** Can't atomically upgrade all routers; any admission-algo change → mixed versions → divergent admission → silent guarantee violation. | 🟠 | Version the admission function; gate activation on network-wide version agreement (flood version, activate new only when all-agree). Freezes evolvability — a real cost. |
| J | **Determinism vs floats/HW.** Any f64 or arch-dependent arithmetic diverges admission silently. **Confirmed conflict (§12):** the existing path computation graph is `StableUnGraph<Node, f64>` and `distances: Vec<f64>` (`network.rs:144-145`) — Bellman-Ford runs in **f64 today**. Deterministic admission can't ride on it as-is. | 🟠 | Replace the f64 graph with **integer/fixed-point** weights for the admission path; link weight is already `u16` (`linkstate.rs:69`) but the graph/distances are f64. Test cross-arch determinism in CI. |

### Net assessment
The design is **coherent and elegant at steady state** — "distributed deterministic PCE + derive-don't-reserve + implicit key-expr multicast + scoped SR repair" genuinely drops both the central controller and per-hop signaling. But its **hard-guarantee strength is inversely tied to churn × scale** (issues A, B, E): it is *not* a global hard-QoS system. It is a **backbone-scoped hard guarantee + best-effort edge** system. That matches the constraints (routers OK, no central admin, fast rejoin) **if** hard guarantees are explicitly scoped to the low-churn router backbone and the edge accepts best-effort. Key resolutions: **(1) multicast reuses Zenoh's implicit key-expr forwarding (§8.5) — no per-packet destination set; detours carry only a coarse region scope, correctness by the tree∩scope invariant (§8.6); (2) "stateless" means *reconstructible*, and the reconstruction is only consistent at quiescence.** BIER stays a fallback for huge sparse flat sets only.

## 10. Oscillation modes & damping

Every swing here is the same shape: **a control action (reroute / detour / preempt / role-flip) changes the signal (load / order / topology) that triggered it**, with loop gain ≥ 1 and delay ⇒ limit cycle. The design has five nested loops; determinism makes herding *worse* (identical nodes move in lockstep).

| # | Swing case | Mechanism | Prevention |
|---|-----------|-----------|-----------|
| S1 | **BW-weight route flap** (ARPANET 1987) | weight=f(load); all flows move to idle link → it congests → weight rises → move back | EWMA-smooth load; **quantize** weights (`u16` helps); **partial response** (shift a *fraction* of flows → loop gain <1); hold-down before re-advertise (§2.3) |
| S2 | **Detour relocates congestion** | flow detours off full link → its load fills the detour → another flow detours back → original frees → swings | hysteresis band `κ` on the admission test (don't revert unless better by `κ`); **minimum path dwell time** before re-eval |
| S3 | **Preemption ping-pong** | preempted flow refreshes soft-state → new seqno → `≺` order flips → it wins → preempts back → refreshes… | **order by immutable `(prio, ZID)` only — never seqno**; seqno = staleness only. Preempt lowest-prio-first; **minimum holding time** before a flow is preemptible (RSVP holding priority) |
| S4 | **Link-flap reconvergence** | link up/down/up → re-flood + `compute_trees` each cycle → traffic swings tree↔detour | **route-flap damping** (exponential penalty + hold-down on a flapping link, BGP-style); debounce link-*up* (stable for `T` before use); TI-LFA repair == post-convergence path → no tree↔repair swing |
| S5 | **Multicast parent/branch flip** | equal-cost tie in tree → subscriber's parent flips on tiny DB change → reorder + transient dup/gap | **stable deterministic tie-break** (lowest-ZID parent); **sticky parent** — keep current unless new better by margin |
| S6 | **Role ↔ admission (cross-layer)** | node flips gossip↔linkstate → its admission ability appears/vanishes → upstream reroutes → load shifts → role threshold re-fires | **timescale separation**: role hysteresis with long dwell ≫ admission timescale ≫ weight-update ≫ packet (economy §6.2) |
| S7 | **Determinism herd sync** | identical nodes recompute on the same flood → move simultaneously → amplifies S1–S5 | **desync the apply, not the result**: jitter when the (identical) computed state is *installed*; fixed point is order-independent so consensus survives |
| S8 | **Estimator resonance** | EWMA time-constant ≈ control-loop delay → control-theory resonance | damping factor below critical; estimator slower than loop; separate timescales (S6) |

### Unifying damping principles
1. **Dead-band** — never reverse a decision unless the alternative wins by margin `κ` (S2, S5).
2. **Dwell / hold-down** — commit to a choice for `T_min` before re-evaluating (S2, S3, S4).
3. **Immutable ordering keys** — decisions tie-break on `(prio, ZID)`; mutable signals (seqno, load) never enter the *order*, only the *feasibility* (S3).
4. **Damping** — EWMA on signals, exponential flap penalty on repeatedly-changing elements (S1, S4, S8).
5. **Timescale separation** — role ≫ admission ≫ weight ≫ packet; estimator ≫ loop delay (S6, S8).
6. **Partial response** — move a fraction, not all → loop gain <1 (S1).
7. **Stickiness** — prefer the current path/parent/role; switching needs to overcome inertia (S2, S5, S6).
8. **Desync-without-divergence** — stagger *application* time; keep the *computed* fixed point identical (S7).

### The determinism paradox
The same property that gives consensus (every node computes identically) causes lockstep herding (S7) and makes the `≺`-order sensitive to any mutable input (S3). So: **keep the computed *result* deterministic, but make the *inputs* stable (immutable keys, smoothed signals) and the *timing* jittered.** Determinism on the fixed point; damping on the path to it.

### 10.9 Documented fixes (from the literature)

Every swing here is a known, solved problem in IP/MPLS routing. Map each to its established technique:

| Case | Established fix | Source | How to apply here |
|------|-----------------|--------|-------------------|
| **S1** BW-weight flap | **Metric normalization + static bias**; stability is set by the *ratio* of traffic-sensitive to traffic-insensitive weight — too little static component → oscillate between two worst cases; enough → converge to unique equilibrium. ARPANET's 1987 **HN-SPF** made the metric depend *linearly* (bounded slope) on utilization. | ARPANET HN-SPF; *Dynamics of load-sensitive adaptive routing*; LSAR thesis | `w_e = static_bias + α·util_e` with **bounded** `α` — **not** `1/(C−λ)` (unbounded slope at saturation → guaranteed oscillation, see §2.2 correction) |
| **S2** detour relocates congestion | **Flow-pinning** (keep admitted flows on their path; only new/failed flows reroute) + **make-before-break (MBB)** | FAMTAR adaptive multipath; RFC 5712 | pin admitted flows; reroute only the blocked branch; build detour before tearing old (MBB) |
| **S3** preemption ping-pong | **`setup_prio ≤ holding_prio`** (a path can't be preempted by its own re-setup) + **soft preemption** (MBB, not intrusive teardown) + **tunnel flap dampening** (track reroute history, dampen over a frequency limit) | RFC 5712 (Soft Preemption); H3C MPLS-TE; US 9667559 | immutable `(prio,ZID)` order (§8.2); **minimum holding time** before preemptible; soft-preempt via MBB |
| **S4** link flap | **Route-flap damping** (additive penalty per flap, exponential decay, suppress above cutoff, reuse below) + **SPF exponential backoff** (fast mode → backoff mode → fast when stable) + link-*up* debounce | RFC 2439 + RFC 7196 (revised thresholds); RFC 8541 (SPF trigger/delay, micro-loops) | per-link flap penalty + hold-down; exponential backoff on `compute_trees`; **RFC 7196 higher thresholds** to avoid over-suppressing transient multi-path |
| **S5** parent/branch flip | **Consistent / resilient ("sticky") hashing** — on a failure remap *only* the affected flows; survivors keep their path (no global rehash) | Cisco NCS5500 Sticky ECMP; US 8595239 (minimally-disruptive hash) | consistent-hash subscriber→branch so a failed branch remaps only its own subs; **sticky parent** (keep current unless better by margin) |
| **S7** herd synchronization | **Timer randomization** — draw next fire from uniform `[T−Tr, T+Tr]`; enough jitter provably breaks synchronized clusters. SPF/LSA timers jitter for the same reason ("thundering herd"). | Floyd & Jacobson 1994 (*Synchronization of Periodic Routing Messages*); SPF throttle+jitter | **jitter the *apply* time, not the computed result** — fixed point is order-independent, so consensus survives while the herd disperses |
| **S8** estimator resonance | **Exponential backoff** + bounded loop gain; metric weight-ratio governs convergence (control-theoretic) | IGRP stability analysis; SPF backoff | estimator time-constant ≫ loop delay; damping factor below critical |

**Recurring cross-cutting theme: make-before-break (MBB).** RFC 5712 (soft preemption) and RFC 4090 (FRR) both establish the new path *before* tearing the old → no delivery gap, and the brief overlap absorbs transient disagreement. Adopt MBB for every reroute/detour/preemption here: the old branch keeps forwarding until the new one is confirmed.

**Key correction surfaced by the research:** my §2.2 congestion weight `w_e = 1/(C_e−λ_e)` is exactly the **unbounded-slope metric known to oscillate** (slope → ∞ as the link saturates = the ARPANET failure mode). Replace with a **bounded** metric: static hop-bias + load term with capped slope (HN-SPF style). The static-to-dynamic weight ratio is the stability knob (S1).

### References
- ARPANET HN-SPF / metric normalization (1987); *Dynamics of load-sensitive adaptive routing*; H. Wang, *LSAR* MSc thesis — https://people.ece.ubc.ca/haow/papers/MasterThesis.pdf
- FAMTAR adaptive multipath routing — https://arxiv.org/pdf/1808.03209
- RFC 2439 — BGP Route Flap Damping — https://www.rfc-editor.org/rfc/rfc2439.html ; RFC 7196 (revised RFD configs)
- RFC 8541 — SPF Trigger and Delay Strategies / IGP micro-loops — https://datatracker.ietf.org/doc/html/rfc8541
- RFC 5712 — MPLS-TE Soft Preemption (make-before-break) — https://www.rfc-editor.org/rfc/rfc5712.html
- Floyd & Jacobson, *The Synchronization of Periodic Routing Messages* (1994) — https://ee.lbl.gov/papers/sync_94.pdf
- Sticky / consistent-hash ECMP — https://xrdocs.io/ncs5500/blogs/2020-09-04-persistent-load-balancing-or-sticky-ecmp/
- *Stability of a class of dynamic routing protocol (IGRP)* — https://www.researchgate.net/publication/3515978

## 11. Open problems

- Capacity estimation accuracy from local TX counters (no link-layer rate visibility on TCP/QUIC).
- Oscillation stability proof under EWMA + hold-down + partial-response — tune constants empirically.
- Segment stack depth bound vs detour coverage — what max depth gives ≥99% single-failure coverage on real topos?
- Multicast + SR resolved via §8.5–8.6 (implicit key-expr forwarding + scoped repair). Remaining: deterministic, low-churn **scope/region assignment** at scale; and the BIER fallback for huge sparse flat receiver sets (edge-node-scoped bitmap, region subdomains) — ties to the multicast/Steiner gap in [economy §4](auto-routing-economics.md).
- Loop prevention with loose segments under concurrent failures.

## 12. Code reality-check: failure / reservation / preemption

Grounded against the actual code + protocol (June 2026). Real timing constants:

| Constant | Value | Source |
|----------|-------|--------|
| Transport lease | **10 s** | `config/defaults.rs:257` |
| Keepalive interval | lease/4 = **2.5 s** | `defaults.rs:258` |
| Socket-close detection (TCP RST/EOF) | **~ms** | `zenoh-link-tcp/.../unicast.rs:156` |
| Tree recompute delay | **100 ms** | `hat/mod.rs:65` |
| Per-priority TX queue | **2 batches** | `defaults.rs:266-283` |
| Block-mode wait before close | **5 s** | `manager.rs`, `defaults.rs:286-299` |
| Drop-mode wait before drop | 1 ms (50 ms fragments) | `defaults.rs:286-299` |
| Multilink | **off by default** (`max_links:1`) | `defaults.rs` |

### 12.1 Node failure — ⚠️ detection is the bottleneck, not reconvergence

- **Fast path (graceful / TCP RST):** socket close → `del_link` (`link.rs:202`) in ~ms → face close → `compute_trees` after 100 ms (`router/mod.rs:314`) + `sn++` flood (`network.rs:966`). TI-LFA could trigger on the socket-close signal — **the design's fast detour works here.**
- **🔴 Slow path (silent failure: power loss, partition, wireless drop):** no socket event → detected only by **lease timeout = up to 10 s**. For 10 s the node is presumed alive, packets sent into a **black hole**, dropped. The design's "µs detour" assumed fast detection — **it does not exist for silent failure.** Zenoh has **no BFD / sub-second liveness**. Sub-second failover on silent failure needs a *new* mechanism (lower keepalive = more overhead, or add BFD-like probing).
- **Multilink is off by default and link-level only** (same neighbor pair) — can't be leaned on for *node* failure FRR as §4.3 implied. Correct §4.3: multilink helps redundant links to one neighbor, not node loss.
- **Crash-rejoin:** restarted peer pulls *remote* state via `initial_interest` (fast, good), but its **own local subs/pubs are lost** and apps must re-declare (`gateway.rs:251`). So GBR for *locally-sourced* flows is app-driven re-declaration, not automatic.
- **Verdict:** design copes on the fast path; **the slow-path 10 s black hole is the dominant real risk** and is unaddressed. Lower lease for GBR domains, or add fast liveness.

### 12.2 Bandwidth reservation — ⚠️ three protocol gaps, one assumption corrected

- **🔴 Publishers don't declare at all** — implicit, inferred from traffic; no `DeclarePublisher` message (`declare.rs` has no publisher variant). "Publisher announces GBR" requires a **new protocol message / declaration extension.** Real new work, not a tweak.
- **✅ Assumption corrected — hard-state is *better* for us.** Zenoh declarations are hard-state, no periodic refresh; link-state event-driven, no LSP aging. So the design's "soft-state refresh" framing was wrong — **but the hard-state + pull-on-reconnect model removes the refresh-flood scaling worry entirely** (issue C revised). Carry GBR as a hard-state declaration; reconstruct on reconnect via `initial_interest`. No refresh timer.
- **🔴 No rate enforcement anywhere** — zero token-bucket / shaping / policing in transport (grep clean). A declared GBR is **pure advisory**; the publisher can exceed it and nothing throttles. "Guaranteed" is meaningless without a **new ingress token-bucket** (issue G) — must be built.
- **🟠 Backpressure is hop-by-hop local only**, no end-to-end flow control (`pipeline.rs`). A congested reserved link silently drops (Drop mode) with **no signal to the publisher**. So *predicted* reservation accounting and *actual* drops are disconnected — compounds issue B.
- **🟠 No measured-capacity primitive** — `C_e` must come from config (issue F); TX counters exist (`congested`/`pending` bits) but aren't exported as a rate.
- **🟠 f64 path computation** (`network.rs:144` `StableUnGraph<Node,f64>`) conflicts with deterministic admission — must move to integer (issue J, now 🟠).
- **Verdict:** the *accounting* model (derive load from flooded declarations) is sound and fits hard-state better than expected, but needs **3 new pieces**: a publisher GBR declaration, ingress policing, and integer-weight path computation.

### 12.3 Flow preemption — ⚠️ works backbone-internal, edge flows just drop

- **✅ A local preemption analog already exists:** strict-priority TX scheduling (`pipeline.rs:843` `trailing_zeros`) — a busy high-priority queue **starves** lower ones. But this is *queue scheduling*, not *routing* preemption: it delays/drops the low-prio flow, it does **not reroute** it.
- **🔴 "Preempted flow detours without notification" breaks at the edge.** The design relies on the preempted source *independently computing the same admission function* to learn it lost. That only holds for **router-tier nodes** running admission. An **edge publisher (a client/app) runs no admission and gets no signal** — with hop-by-hop-only backpressure + Drop mode, it just experiences **silent drops and never reroutes.** So clean preempt-and-reroute is **backbone-internal only** (consistent with §8.4 scoping); edge flows need an explicit preemption-notification message or they degrade to drops.
- **✅ 7 usable priority levels** (Control=0 internal; public 1–7) map cleanly to RSVP setup/holding levels for the `setup ≤ holding` rule (§10.9 S3).
- **🟠 Starvation already real** (strict priority) — our routing-level preemption adds to it; needs the min-holding-time + anti-starvation fixes (§10.9 S3, issue D).
- **Verdict:** preemption is feasible **among router-tier nodes**; the "no notification" elegance holds only there. Edge publishers can't be cleanly rerouted on preemption without a new notify path — scope preemption to the backbone, accept edge drops.

### 12.4 Net
The design's **core mechanisms survive contact with the code** (event-driven link-state, hard-state + pull rejoin, strict-priority substrate, `u16` weights, `NodeId` ext hook). The reality gaps are concrete and bounded:
1. **Silent-failure 10 s black hole** — the biggest gap; needs fast liveness for any sub-second guarantee.
2. **Three new protocol/impl pieces** for reservation: publisher GBR declaration, ingress token-bucket policing, integer-weight path computation.
3. **Preemption scoped to backbone** — edge flows drop rather than reroute without a new notify message.

None are fatal; all reinforce the same conclusion — **hard guarantees live on the low-churn router backbone with new admission+policing machinery; the edge stays best-effort.**

## 13. Gap classification

Three classes by *kind of work*, each with a different risk/effort profile.

### Class R — Runtime / mechanism (the node lacks a capability)
New node-local machinery. Mostly independent, incremental, unit-testable in isolation. Engineering, not standardization.

| ID | Gap | Source | Needs |
|----|-----|--------|-------|
| R1 | **Silent-failure detection** — no BFD/fast liveness; 10 s lease black hole | §12.1, issue C | sub-second liveness probe (BFD-like) or lower keepalive for GBR domains |
| R2 | **No rate enforcement** — zero token-bucket/shaping/policing | scout, issue G | ingress token-bucket at publisher's first hop |
| R3 | **No measured-capacity export** — `C_e` only from config | issue F | export TX `congested`/`pending` counters as a rate; smooth+quantize |
| R4 | **No end-to-end backpressure** — hop-by-hop drop only | scout, issue B | optional e2e flow-control signal, or accept reactive-only |
| R5 | **Stale-reservation cleanup is lease-bound** (face-close timing) | issue C | bind GBR lifetime to face; auto-undeclare on close (already done for subs) |
| R6 | **Multilink off by default, link-level only** | §12.1 | enable + a path-level redundancy notion for node (not just link) failure |

### Class P — Protocol / wire (new messages or header fields)
Wire changes. Need version negotiation + backward-compat (empty/absent = legacy) + cross-implementation agreement. Harder to evolve — every addition widens the rolling-upgrade surface (issue I).

| ID | Gap | Source | Needs |
|----|-----|--------|-------|
| P1 | **Publisher GBR declaration** — publishers are implicit, no `DeclarePublisher` | §12.2 | new declaration message / field carrying GBR (hard-state) |
| P2 | **Segment stack** — extend `NodeId` ext `0x3` (single `u16`) → `SegmentList` | §5 | list-typed ext; empty = legacy |
| P3 | **Scope tag + detour-flag** on detoured packets | §8.5–8.6 | coarse region/edge-node scope field + flag |
| P4 | **Per-link capacity `C_e`** in link-state | §2.1, §8.1 | extend LSP (link weights already `u16`; add capacity) |
| P5 | **Edge preemption-notification** — edge publishers get no signal | §12.3 | explicit "you were preempted, reroute" message (backbone→edge) |
| P6 | *(fallback)* **BIER bitmap** header for huge sparse flat sets | §8.5 | bit-indexed header + deterministic bit assignment |

### Class A — Algorithm / design (logic, consistency, stability, scaling)
Cannot be "added" — must be *designed correct*. The load-bearing risk class. Reuses the issue/swing IDs already in this doc.

| ID | Gap | Source | Needs |
|----|-----|--------|-------|
| A (issue) | **Eventual-consistency vs deterministic admission** — never one DB under churn | §9-A | scope to low-churn backbone; quantify churn ≪ convergence; MBB to absorb |
| B (issue) | **Predicted accounting vs actual forwarding** divergence | §9-B | account from observed headers (reactive) vs predict (needs consistency) — inherent |
| D (issue) | **Starvation + preemption flap** | §9-D, S3 | immutable `(prio,ZID)` order; min holding time; aging |
| E (issue) | **Sequential admission recompute** `O(flows×CSPF)` | §9-E | incremental recompute; bound flow count per domain |
| I (issue) | **Rolling-upgrade split-brain** (versioned admission fn) | §9-I | version flood + activate-on-all-agree; freezes evolvability |
| J (issue) | **f64 → integer** determinism (graph is `f64` today) | §9-J, §12.2 | integer/fixed-point weights on the admission path |
| S1–S8 | **Oscillation modes** | §10 | the 8 damping principles (§10.9, literature-backed) |
| — | **Deterministic multicast-tree approximation** (BW-Steiner is NP-hard) | §11 | a deterministic greedy all nodes agree on; quality bound open |
| — | **Scope/region assignment determinism at scale** | §11 | stable, low-churn region IDs derivable from DB |

### Reading the classes
- **Class R** = build it; low coupling, ship incrementally, measure first (R3 before R2 before R1).
- **Class P** = standardize it; each field is forever, gate on version negotiation, keep empty=legacy so deployment is opt-in and reversible.
- **Class A** = prove it; this is where the design lives or dies. R and P are tractable engineering; **A is the research.** Everything in A reduces to one root: *deterministic consensus assumes a DB consistency the flood only provides at quiescence* — so the whole design is **steady-state-hard, transient-best-effort**, scoped to the low-churn backbone.

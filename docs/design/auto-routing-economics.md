# Auto-Routing Configuration: A Cost & Economy Model (v2)

**Status:** exploration / design note. Not implemented. v2 — rewritten after self-review; fixes in §0.

## 0. What changed from v1

v1 had real flaws. v2 fixes them:

1. **Data plane is multicast pub/sub, not unicast flows.** Cost is now distribution-tree weight (Steiner-flavored), not point-to-point path stretch (§4).
2. **Linkstate memory is `O(n²)`, not `O(n)`.** Each linkstate node stores `Vec<Tree>` (≈ n roots) × `directions` length n — verified in `Network` (`zenoh/src/net/protocol/network.rs:144,131`). Both CPU and memory scale super-linearly (§3).
3. **Region size `n` is endogenous** — who runs linkstate depends on who else does. v2 treats role selection as a game with a fixed point, not independent per-node optimization (§6).
4. **Gossip↔linkstate is a reachability step, not a smooth stretch.** Modeled as a discontinuity (§4.2).
5. **Mechanism-design claims corrected.** No truthful + budget-balanced + efficient mechanism exists (Green–Laffont). Drop the strategyproofness and price-of-anarchy overclaims; state honest guarantees (§7).
6. **Dimensions fixed.** Memory is a stock; CPU/bandwidth are flows. v2 amortizes everything to one numeraire (energy or $) per unit time over horizon `H` (§2).
7. **Circular observability acknowledged.** You can’t measure linkstate’s payoff without linkstate. v2 uses biased proxies and says so (§5, §9).

---

## 1. Problem

Zenoh roles are hand-configured (`WhatAmI` in `commons/zenoh-protocol/src/core/whatami.rs`):

| Role | Control plane | State | Data plane |
|------|---------------|-------|------------|
| **client** `C` | one uplink, no routing | O(1) | via uplink |
| **peer-gossip** `Pg` | single-hop dissemination to neighbors | O(d) | direct neighbors; multihop only via backbone |
| **peer-linkstate** `Pn` | full LSDB, Bellman-Ford trees per root | **O(n²)** | shortest-weight tree |
| **router** `R` | linkstate backbone + serves clients | O(n_R²)+O(k) | shortest path on backbone |

Code anchors: gossip/network split `zenoh/src/net/routing/hat/peer/mod.rs` (`Hat::Gossip` / `Hat::Network`); `Network::compute_trees()` (Bellman-Ford) `network.rs:1015`; recompute debounce `TREES_COMPUTATION_DELAY_MS = 100 ms`; link weight u16, default 100, resolved as max of both endpoints; `routing.peer.mode` config key **already deprecated** (project drifting toward implicit selection); regions/gateways `zenoh/src/net/routing/dispatcher/region/`.

Thesis: **role = optimization under uncertainty.** Spend control-plane resources (memory, CPU, control bandwidth) only when the data-plane saving (better distribution trees) pays for it — decided from *local, biased* observations because topology is unknown and large.

---

## 2. Numeraire and horizon (dimensional setup)

All terms reduce to **cost rate** (e.g. joules/s, or \$/s) so they’re summable. Memory is a stock → amortize by a rental price.

$$
\text{cost rate} = \underbrace{p_m \cdot \text{Mem}}_{\text{stock}\times \$\,\text{B}^{-1}\text{s}^{-1}} \;+\; \underbrace{p_c \cdot \text{Ops/s}}_{\text{flow}} \;+\; \underbrace{p_b \cdot \text{Bytes/s}}_{\text{flow}}
$$

`p_m, p_c, p_b` are device-specific prices (an embedded peer’s `p_m,p_c` ≫ a server’s). Heterogeneity lives entirely in the prices; the structural terms are HW-agnostic.

---

## 3. Control plane (per node `i`)

`d_i=deg(i)`, `n=|V_region(i)|`, `e≈n d̄/2`, `λ_i`=topology-change rate seen by `i`, `Δ`=100 ms.

**Memory (stock).** LSDB = nodes+edges `O(nd̄)`; **trees = n roots × n directions `O(n²)`** (dominates):
$$
\text{Mem}(C)=O(1),\quad \text{Mem}(P_g)=O(d_i),\quad \text{Mem}(P_n)=O(n^2),\quad \text{Mem}(R)=O(n_R^2+k_i)
$$

**CPU (flow).** Recompute = one Bellman-Ford `O(ne)` + fill all trees’ directions `O(n²)`, at rate `min(λ_i,1/Δ)`:
$$
\text{Cpu}(P_n)\approx \min\!\big(\lambda_i,\tfrac1\Delta\big)\big(n e + n^2\big)=\min\!\big(\lambda_i,\tfrac1\Delta\big)\,n^2\big(\tfrac{\bar d}{2}+1\big),\qquad \text{Cpu}(C{,}P_g)\approx 0
$$
*Lever (was ignored in v1):* weights are ≥0, so Dijkstra `O(e\log n)` could replace BF `O(ne)` — algorithm choice changes the constant materially.

**Control bandwidth (flow).**
$$
\text{Bw}(P_g)\approx \lambda_i d_i,\qquad \text{Bw}(P_n)\approx \lambda_i d_i \;(\text{forward}) + \lambda_i n \;(\text{ingest LSPs})
$$

**Per-node control cost rate:**
$$
\boxed{\;\gamma_i(r_i)=p_m\,\text{Mem}(r_i)+p_c\,\text{Cpu}(r_i)+p_b\,\text{Bw}(r_i)\;}
$$
For linkstate the leading term is `(p_m + p_c\min(\lambda_i,1/\Delta))\,n^2` — **quadratic in region size.** This forces hierarchy (§8).

---

## 4. Data plane (multicast — the v1 fix)

Zenoh distributes a publication on key-expr `x` from publisher `s` to **the set of matching subscribers** `S_x ⊆ V`. Cost is the weight of the distribution tree `T(s,x)` carrying it, times publish rate `φ_{s,x}` (bytes/s):

$$
C_{\text{data}}=\sum_{(s,x)} \varphi_{s,x}\; W\!\big(T(s,x)\big),\qquad W(T)=\sum_{(u,v)\in T} w_{uv}
$$

The optimal tree connecting `{s}∪S_x` is a **Steiner tree** (NP-hard; metric 2-approx exists). Define per source-keyexpr the achieved tree weight under a role assignment vs optimum:

$$
\eta_{s,x}=\frac{W(T_{\text{achieved}})}{W(T^\*_{\text{Steiner}})}\ \ge 1 \quad(\text{tree inflation, replaces v1’s path stretch }\sigma)
$$

### 4.1 What linkstate buys

A linkstate region computes shortest-weight trees rooted at each source (`compute_trees()`), giving near-optimal `η→` small. Gossip/backbone routing inflates trees (traffic detours through hubs, duplicate delivery). Saving from running linkstate in the region serving `(s,x)`:

$$
\Delta C_{\text{data}}(s,x)=\varphi_{s,x}\,W(T^\*)\,(\eta_{\text{gossip}}-\eta_{\text{linkstate}})
$$

Aggregate at node `i` over the source-keyexprs whose trees traverse `i`:
$$
D_i=\sum_{(s,x)\ni i}\varphi_{s,x}\,W(T^\*)\,(\eta_{\text{gossip}}-\eta_{\text{linkstate}})
$$

### 4.2 Reachability step (the other v1 fix)

Gossip is single-hop. If `S_x` contains a subscriber more than one hop away and no backbone bridges them, the achieved tree **does not exist** — delivery fails, not merely detours. So `Δ C_data` is **discontinuous**, not a smooth `(η−1)`:

$$
\Delta C_{\text{data}}(s,x)=
\begin{cases}
+\infty\ (\text{or SLA penalty }\Pi) & \text{multihop subs unreachable without }P_n/R\\[4pt]
\varphi\,W(T^\*)(\eta_g-\eta_n) & \text{both feasible, linkstate just tighter}
\end{cases}
$$

The model must first satisfy *feasibility* (reachability ≥ 1−ε), then optimize tree weight. Two-stage, not one smooth objective.

### 4.3 Selectivity

Tree size depends on subscriber density for `x`, not raw byte volume. Let `μ_x=|S_x|/|V|` (match selectivity). Sparse `μ_x` → small trees → linkstate matters less; dense `μ_x` (broadcast-like) → big trees → linkstate matters more. `W(T^\*)` already encodes this; flag it because `φ` alone is misleading.

---

## 5. Local threshold (with honest, biased inputs)

Node `i` observes only: `d_i` (faces), `λ_i` (LSP/gossip arrival counter), `φ` it forwards/originates, `μ̂` (matched-sub fraction from interest declarations), `n̂` (region-size estimate). It **cannot** observe `W(T^\*)` or `η` without already running linkstate (circular). Use proxies:

- `n̂` ← gossip-TTL sampling / hop-count histogram (biased low for far nodes).
- `Ŵ(T^\*)` ← passive: sum of measured hop-RTTs or hop-counts × default weight 100.
- `η̂_g` ← compare gossip delivery hop-count vs direct estimate (only available where both seen).

**Stage 1 — feasibility:** if any required subscriber is multihop-unreachable under gossip ⇒ must be `P_n` (or attach to an `R`). Hard.

**Stage 2 — efficiency (only if feasible either way):**
$$
\boxed{\;\hat D_i \;>\; \big[\gamma_i(P_n)-\gamma_i(P_g)\big]+\kappa_i\;}
\quad\Longleftrightarrow\quad
\hat D_i \;>\; (p_m+p_c\min(\lambda_i,\tfrac1\Delta))\,\hat n^2\big(\tfrac{\bar d}{2}+1\big)+p_b\lambda_i\hat n+\kappa_i
$$

`κ_i` = switching cost (§6.2). In code this drives `i`’s own `Hat::Gossip↔Network` / `scouting.gossip.multihop` — local, reversible.

---

## 6. Role selection is a game (the endogeneity fix)

`n` (linkstate region size) depends on how many neighbors also pick `P_n`. So §5 is **best-response in a game**, not isolated optimization.

### 6.1 Fixed point

State = role vector `r`. Node `i`’s best response `BR_i(r_{-i})` uses the §5 rule with `n=n(r)` induced by current choices. A **pure Nash equilibrium** is `r^\*` with `r_i^\*=BR_i(r_{-i}^\*)` ∀i. Existence isn’t guaranteed in general; two adjacent nodes can ping-pong (each assuming the other carries linkstate). Mitigations:

- **Potential-game shaping:** add a small coordination term so a potential function `Φ` decreases on every unilateral improving switch ⇒ best-response dynamics converge. Requires the externality (your switch changes neighbors’ `n`, CPU) to be internalized in `Φ`.
- **Leader election within a candidate region:** elect linkstate carriers by ID/score rather than independent flips — removes symmetric ping-pong.

### 6.2 Switching cost is an externality

A role flip forces neighbors to re-flood LSPs and recompute every tree (`compute_trees` over the region) and re-establish interests (`routing.interests.timeout` latency). So `κ_i` is not purely local:
$$
\kappa_i=\underbrace{\kappa_i^{\text{self}}}_{\text{rebuild own LSDB}}+\sum_{j\in \text{region}}\underbrace{\kappa_{j}^{\text{induced}}}_{\text{neighbors recompute}}
$$
Internalizing the induced term in `Φ` is what makes the dynamics stable (hysteresis band = `(s,S)` rule only after this is accounted).

### 6.3 Nonstationarity

Inputs drift. Per-node role = online decision over `{P_g,P_n,R}` with **tracking regret** (EXP3.S / discounted-UCB), not vanilla EXP3 — best fixed arm is a weak benchmark when topology moves. Rewards = clipped negative cost rate (must be bounded for the regret bound to hold).

---

## 7. Router placement & economy (mechanism-design fix)

If topology known, "who becomes a router" = **uncapacitated facility location**:
$$
\min_{x,y}\sum_j f_j y_j+\sum_{i,j}c_{ij}x_{ij},\quad
f_j=\gamma_j(R)\ \text{(opening = control cost)},\quad
c_{ij}=\text{data cost of }i\text{ served by hub }j
$$
NP-hard; greedy `O(\log n)`, Jain–Vazirani primal-dual **3-approx** — *for the optimization with truthful inputs.*

### Decentralized & strategic — honest guarantees

A router is a **public good** (helps others ⇒ free-riding). Run a cost-sharing election: nodes bid willingness `αᵢ` (their feasibility value `Π` if unreachable, else tree saving), candidate opens when `Σ_i max(0,αᵢ−c_{ij}) ≥ f_j`, split `f_j` by a **cross-monotonic (Moulin) cost-share**.

Honest statement of what’s achievable (v1 overclaimed):

- **Cannot** simultaneously get truthful + budget-balanced + efficient (Green–Laffont; Myerson–Satterthwaite). Pick two.
- **Moulin + cross-monotone** cost-sharing → group-strategyproof + budget-balanced, but **loses efficiency** (approx factor, e.g. `O(\log n)` for FL via JV cross-monotone share).
- **VCG** → truthful + efficient, but **not budget-balanced** (needs external subsidy) — impractical for a P2P fabric with no bank.
- The 3-approx is an *algorithmic* bound; it does **not** bound the game’s price of anarchy. PoA needs separate analysis (open).

Recommendation: cross-monotone cost-share (budget-balanced, group-strategyproof, bounded efficiency loss) — fits a bank-less P2P network better than VCG.

---

## 8. Hierarchy is forced at scale (sharpened)

Control cost `∝ n²` (both `p_m·Mem` and `p_c·Cpu`). Flat linkstate is infeasible past:
$$
n_{\max}=\min\!\Bigg(\underbrace{\sqrt{\tfrac{M_i}{p_m\,c_{\text{tree}}}}}_{\text{memory bound}},\ \underbrace{\sqrt{\tfrac{B_i}{p_c\,f_{\text{rec}}(\bar d/2+1)}}}_{\text{CPU bound}}\Bigg)
$$
per device budgets `M_i,B_i`. Beyond `n_max`: split into regions, gateways bridge, the **small** inter-region carrier set (`n_R ≪ n`) runs linkstate cheaply ⇒ that set *is* the router backbone. So Zenoh’s 3-tier shape (linkstate few routers / gossip many peers / clients at edge) is the **economic optimum derived**, and `n_max` is the auto-grow knob. Heterogeneous prices ⇒ high-budget nodes get larger `n_max` ⇒ **naturally elected as carriers** — placement and capacity fall out of the same inequality.

---

## 9. Decision under uncertainty

- **Feasibility first, robustly:** chance constraint `Pr[reachable(i)] ≥ 1−ε` over the plausible topology set `Θ`; keep a redundant uplink sized to observed failure rate.
- **Then efficiency, with hysteresis** (§5,§6.2).
- **Prior is gossip — but derived, not asserted:** under max uncertainty on `Θ` the *feasibility* term dominates and gossip+backbone guarantees reachability at O(d) cost, while a wrong linkstate promotion risks the `n²` blow-up bounded only by `n_max`. So min-worst-case starts gossip; promote as confidence intervals on `n̂, η̂, μ̂` shrink (Bayesian schedule). If observations reveal the small-net/high-traffic corner, the same schedule promotes early — no fixed bias.
- **`n_max` cap is the safety valve:** even with badly biased `n̂`, capping bounds worst-case CPU/memory so a wrong promotion can’t melt a device.

---

## 10. Prototype path (risk-ordered)

1. **Instrument observables** per node: `d_i`, `λ_i` (LSP counter), `φ` (forwarded bytes), `μ̂` (matched subs from interests), `n̂` (gossip-TTL sample). Most live near `Network` / face stats.
2. **Offline replay** recorded topologies + traffic through the §5 two-stage rule; calibrate `p_m,p_c,p_b,κ` against hand-config cost. Validate the multicast `W(T^\*)` term against measured trees.
3. **Single auto knob:** drive each peer’s `Hat::Gossip↔Network` (`scouting.gossip.multihop`) from §5 with the §6.2 hysteresis. Local, reversible, no wire change. Lowest risk.
4. **Coordinated election** (§6.1) once flapping observed between adjacent nodes.
5. **Distributed router cost-share auction** (§7) — needs an OAM message type for bids; largest change.

## 11. Open problems

- Unbiased `n̂` / `Ŵ(T^\*)` estimation in an unobservable network — the central measurement gap (§5 proxies are biased).
- Price of anarchy of the role-selection game (§6) and the router auction (§7) — unbounded so far.
- Existence/uniqueness of the Nash fixed point; conditions under which potential-game shaping (§6.1) applies.
- Joint optimization with key-expr/interest density — `W(T^\*)` couples to subscriber distribution, which itself shifts with role changes (feedback).
- Correlated `λ`,`φ` under mobility (independence assumed throughout).

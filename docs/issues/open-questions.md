# Open questions (unsolved / research)

Genuinely unresolved — no known correct solution yet. Distinct from `mitigation-known` issues. Source: [qos §11](../design/qos-fault-routing.md), [economy §11](../design/auto-routing-economics.md).

## Consistency & guarantee
- **Quantify the steady-state boundary.** For what `churn_rate × convergence_time` does the hard guarantee actually hold? Need a measured/derived threshold, not just "low-churn backbone" ([A-A](class-A-algorithm.md), [A-B](class-A-algorithm.md)).
- **Predictive admission without a consistent global view** — can a node admit safely from a stale DB without oversubscription? Probably needs conservative margins; bound unknown ([A-B](class-A-algorithm.md)).

## Estimation (the measurement gap)
- **Unbiased `n̂` / region-size estimation** in an unobservable large network (gossip-TTL sampling? hop histograms?) — [economy §9].
- **Capacity estimation** from local TX counters with no link-layer rate visibility on TCP/QUIC ([R3](class-R-runtime.md)).
- Calibrating cost constants (`p_m,p_c,p_b`, `α`, `κ`) across heterogeneous hardware.

## Multicast at scale
- **Deterministic BW-Steiner approximation** with a provable quality bound ([A-MC1](class-A-algorithm.md)).
- **Scope/region assignment** that is stable, low-churn, and DB-derivable; same problem for any BIER bit assignment ([A-MC2](class-A-algorithm.md), [P6](class-P-protocol.md)).
- Segment-stack depth vs detour coverage — max depth for ≥99% single-failure coverage on real topologies.

## Game-theoretic
- **Price of anarchy** of the role-selection game and the router cost-share auction — unbounded so far ([economy §6, §7]).
- Existence/uniqueness of the Nash fixed point; when does potential-game shaping apply ([economy §6.1]).

## Stability proofs
- Oscillation stability proof under EWMA + hold-down + partial-response — currently tuned empirically ([oscillation.md](oscillation.md)).
- Loop prevention with loose segments under *concurrent* failures.

## Security / economy without admin
- Decentralized per-publisher GBR quota enforcement (policing is local, but who sets quotas without a central authority?) ([R2](class-R-runtime.md), [economy §7 issue G]).

## Coupling
- `W(T*)` (data cost) couples to subscriber distribution, which shifts with role changes — a feedback loop not modeled ([economy §11]).
- Correlated churn `λ` and traffic `φ` under mobility (assumed independent throughout).

# Class A — Algorithm / design issues

Logic, consistency, stability, scaling. Cannot be "added" — must be *designed correct*. The load-bearing class. Source: [qos §9](../design/qos-fault-routing.md), [qos §10](../design/qos-fault-routing.md).

**Root cause (all 🔴 reduce to this):** deterministic distributed consensus assumes a DB consistency the flood only provides at quiescence → steady-state-hard, transient-best-effort, backbone-scoped.

## Core issues

| ID | Title | Sev | Status | Fix / mitigation |
|----|-------|-----|--------|------------------|
| A | **Eventual-consistency vs deterministic admission** — never one DB under churn → concurrent *divergent* admission. | 🔴 | research | scope to low-churn backbone; require churn-period ≪ convergence-time; MBB overlap absorbs transient disagreement |
| B | **Predicted accounting vs actual forwarding** divergence — packet follows ingress's path; transit recomputes a different tree → `load(e)` wrong. | 🔴 | research | account from *observed* headers (consistent but reactive) vs *predict* (needs consistency, issue A) — inherent tension |
| D | **Starvation + preemption flap** under churn — order key must be immutable, but `(prio,ZID)`-only loses incumbency (low-ZID newcomer bumps an incumbent). | 🟠 | mitigation-known | order on `(prio, HLC-birth-epoch, ZID)`: HLC epoch = physical-time incumbency, frozen at declaration, skew-tolerant, determinism-safe. Clamp clock-poisoning; restart=newcomer. + min holding time + aging. See [oscillation S3](oscillation.md), qos §8.2 |
| E | **Sequential admission recompute** `O(flows×CSPF)` — flow k depends on residual from 1..k-1; one flap re-runs the cascade. | 🟠 | mitigation-known | incremental recompute (approx, breaks strict order); bound flow count per domain |
| I | **Rolling-upgrade split-brain** — mixed admission-algo versions → divergent admission → silent guarantee violation. | 🟠 | mitigation-known | version flood + activate-on-all-agree; freezes evolvability (real cost) |
| J | **f64 → integer determinism** — path graph is `StableUnGraph<Node,f64>` + `distances:Vec<f64>` today → nondeterministic across arch. | 🟠 | mitigation-known | integer/fixed-point weights on the admission path (`network.rs:144-145`; weight already `u16` `linkstate.rs:69`) |

## Multicast-specific

| ID | Title | Sev | Status | Fix |
|----|-------|-----|--------|-----|
| A-MC1 | **Deterministic multicast-tree approximation** — BW-constrained Steiner tree is NP-hard; need an approximation all nodes compute identically. | 🟠 | research | deterministic greedy (add subs in `≺` order via CSPF to nearest on-tree node); quality bound open |
| A-MC2 | **Scope/region assignment determinism at scale** — detour scope (and any BIER bit) must be stable + derivable from DB. | 🟠 | research | stable low-churn region IDs from DB; sticky assignment |

## Cross-refs
- Oscillation sub-family (S1–S8): [oscillation.md](oscillation.md).
- Unresolved research framing: [open-questions.md](open-questions.md).
- Inherent design tradeoffs (B, A) have no clean fix — they bound what the system can promise, documented as such.

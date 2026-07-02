# Adaptive Routing for Zenoh — documentation

Exploration / design notes for bandwidth-aware, priority-enabled, fault-tolerant, auto-configured routing on Zenoh. **Not implemented.** All grounded in real code (June 2026).

## Layout

| Dir | What | Read for |
|-----|------|----------|
| [`design/`](design/) | Design specs — the *why* and the *math* | rationale, models, full discussion |
| [`phases/`](phases/) | Numbered implementation roadmap | *what to build, in what order* |
| [`issues/`](issues/) | Known-issue tracker (classified R/P/A) + open questions | *what's unsolved / risky* |

## Reading order

1. [`design/auto-routing-economics.md`](design/auto-routing-economics.md) — when to be router/peer/client; cost & economy model.
2. [`design/qos-fault-routing.md`](design/qos-fault-routing.md) — bandwidth/priority/fault design, deterministic admission, multicast scoped-detour, two-timescale rerouting, oscillation, code reality-check.
3. [`design/sr-stateless-features.md`](design/sr-stateless-features.md) — features SR + stateless unlock beyond the core (SFC, Flex-Algo slices, IOAM telemetry, DetNet, LISP mobility, anycast); stateful-vs-stateless (NDN) axis.
4. [`design/mixed-criticality-rt.md`](design/mixed-criticality-rt.md) — deterministic RT for mixed flows (control + sensor + background); network-calculus latency bounds, FRER, ATS; Zenoh RT gaps. The robotics/AV case.
5. [`design/zenoh-routing-readiness.md`](design/zenoh-routing-readiness.md) — **review** of Zenoh's actual routing vs the 4 goals (deterministic / bounded-latency / BW-guaranteed / mixed-flow): scorecard, blockers, first-cut plan. The gap analysis.
6. [`phases/README.md`](phases/README.md) — the build roadmap (dependency graph + phases 0–9).
7. [`issues/README.md`](issues/README.md) — the issue tracker.

## One-line thesis

Routing role and QoS are **optimization under uncertainty**, not config knobs. The design gives **hard guarantees on the low-churn router backbone at steady state, best-effort at the edge** — because deterministic distributed consensus assumes a DB consistency the flood only provides at quiescence.

## Status legend (used across issue/phase docs)

- 🔴 fundamental · 🟠 major · 🟡 manageable
- **Class R** runtime/mechanism (build it) · **Class P** protocol/wire (standardize it) · **Class A** algorithm/design (prove it)
- Status: `open` · `mitigation-known` · `research`

# Known-issue tracker

Issues from the design, classified by *kind of work*. Canonical actionable list; rationale lives in [`../design/qos-fault-routing.md`](../design/qos-fault-routing.md) §9/§10/§12/§13.

## Classes

| Class | Meaning | Effort shape | File |
|-------|---------|--------------|------|
| **R** | Runtime / mechanism — node lacks a capability | build it (low coupling, incremental) | [class-R-runtime.md](class-R-runtime.md) |
| **P** | Protocol / wire — new message or header field | standardize it (version-gate, empty=legacy) | [class-P-protocol.md](class-P-protocol.md) |
| **A** | Algorithm / design — logic, consistency, stability | prove it (research; design lives or dies here) | [class-A-algorithm.md](class-A-algorithm.md) |
| — | Oscillation modes (a sub-family of A) | damping (literature-backed) | [oscillation.md](oscillation.md) |
| — | Genuinely open research questions | unsolved | [open-questions.md](open-questions.md) |

## Severity & status

- 🔴 fundamental · 🟠 major · 🟡 manageable
- `open` (no fix yet) · `mitigation-known` (fix identified, unbuilt) · `research` (no known correct solution)

## The root cause (all Class-A reduces to this)

> Deterministic distributed consensus assumes a DB consistency the link-state flood only provides **at quiescence**. Under churn, nodes compute divergent decisions concurrently.

⇒ the design is **steady-state-hard, transient-best-effort**, scoped to the low-churn router backbone. Every 🔴 in Class A is a face of this.

## Counts

- Class R: 6 (R1–R6) — mostly `mitigation-known`
- Class P: 6 (P1–P6) — `open` (need spec)
- Class A: 8 issues (A,B,D,E,I,J + 2 multicast) + 8 swings (S1–S8) — mix of `mitigation-known` and `research`
- Open research: see [open-questions.md](open-questions.md)

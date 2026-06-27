# Phase 06 — GBR declaration + ingress policing

**Goal:** let a publisher declare a guaranteed bitrate, and **enforce** it. Without enforcement a reservation is fiction.

**Depends on:** [01](01-observability-capacity-signal.md) (capacity signal).
**Delivers:** [P1](../issues/class-P-protocol.md), [R2](../issues/class-R-runtime.md), [R4](../issues/class-R-runtime.md), [R5](../issues/class-R-runtime.md).
**Unblocks:** 07.

## Deliverables
1. **Publisher GBR declaration** ([P1](../issues/class-P-protocol.md)): publishers are implicit today (no `DeclarePublisher`). Add a hard-state declaration carrying GBR **+ an HLC birth-epoch stamped once at declaration** (the admission incumbency key, [07](07-deterministic-admission.md) / [A-D](../issues/class-A-algorithm.md)) — immutable across refresh/reconnect. **Hard-state, no refresh** — rebuilt on reconnect via `initial_interest` pull (Zenoh's actual model; better than soft-state — no refresh flood).
2. **GBR lifetime = declaring face** ([R5](../issues/class-R-runtime.md)): auto-undeclare on face close (as subs already do, `face.rs:728`) so dead-publisher reservations don't linger past failure detection.
3. **Ingress token-bucket policing** ([R2](../issues/class-R-runtime.md)): shape/mark at the publisher's first hop to the declared GBR. New mechanism — none exists today.
4. Optional **end-to-end backpressure** signal ([R4](../issues/class-R-runtime.md)) or accept reactive-only accounting (congested link drops silently today).

## Exit criteria
- A publisher declares GBR; the declaration floods (hard-state) and is reclaimed on disconnect.
- Traffic exceeding GBR is shaped/marked at ingress (policing verified).
- Restarted publisher's GBR rebuilt via pull on reconnect.

## Risks / issues
- [R5](../issues/class-R-runtime.md) stale reservation up to lease (10 s) on silent failure — pair with [04](04-fast-reroute-liveness.md) fast liveness.
- No e2e backpressure ([R4](../issues/class-R-runtime.md)) → predicted vs actual gap ([B](../issues/class-A-algorithm.md)).
- Decentralized per-publisher quota (who limits GBR without central admin?) — [open](../issues/open-questions.md).

## Design refs
[qos §12.2](../design/qos-fault-routing.md), [qos §8.1](../design/qos-fault-routing.md).

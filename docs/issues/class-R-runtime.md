# Class R — Runtime / mechanism issues

The node lacks a capability. New node-local machinery; low coupling, ship incrementally, measure first. Source: [qos §12](../design/qos-fault-routing.md), [qos §13](../design/qos-fault-routing.md).

| ID | Title | Sev | Status | Phase | Code anchor | Fix |
|----|-------|-----|--------|-------|-------------|-----|
| R1 | **Silent-failure detection** — no BFD/fast liveness; up to **10 s** lease black hole on power-loss/partition/wireless-drop. TI-LFA only fast on TCP-close path. | 🔴 | mitigation-known | [P4](../phases/04-fast-reroute-liveness.md) | lease `defaults.rs:257` (10 s), keepalive `:258` (2.5 s) | sub-second liveness probe (BFD-like) or per-domain lower keepalive |
| R2 | **No rate enforcement** — zero token-bucket/shaping/policing in transport. Declared GBR is advisory; publisher can exceed freely. | 🟠 | open | [P6](../phases/06-gbr-declaration-policing.md) | grep clean across `io/zenoh-transport` | ingress token-bucket at publisher's first hop |
| R3 | **No measured-capacity export** — `C_e` only from config; real capacity varies (wireless/shared NIC/cross-traffic). | 🟠 | mitigation-known | [P1](../phases/01-observability-capacity-signal.md) | `pipeline.rs` `congested`/`pending` bits exist but not exported as rate | export TX counters as smoothed+quantized rate into `link_weights` LSP |
| R4 | **No end-to-end backpressure** — hop-by-hop drop only; congested reserved link silently drops (Drop mode), no signal to publisher. | 🟠 | open | [P6](../phases/06-gbr-declaration-policing.md) | `pipeline.rs` local blocking only | optional e2e flow-control signal, or accept reactive-only accounting |
| R5 | **Stale-reservation cleanup is lease-bound** — dead publisher's GBR lingers until face-close (~ms fast / 10 s silent), wrongly rejecting live flows meanwhile. | 🟠 | mitigation-known | [P6](../phases/06-gbr-declaration-policing.md) | face cleanup `face.rs:728` | bind GBR lifetime to declaring face; auto-undeclare on close (as subs already do) |
| R6 | **Multilink off by default, link-level only** (`max_links:1`); helps redundant links to one neighbor, not node failure. | 🟡 | mitigation-known | [P4](../phases/04-fast-reroute-liveness.md) | `defaults.rs` multilink, `tx.rs:110` select | enable + add a path-level (node-disjoint) redundancy notion |

Build order within R: **R3 → R2 → R1** (measure, then enforce, then fast-detect). R5 rides with R2 (both touch GBR lifecycle).

# Class P — Protocol / wire issues

New wire messages or header fields. Each field is forever → version-gate, keep **empty/absent = legacy** so deployment is opt-in and reversible. Every addition widens the rolling-upgrade surface ([A-I](class-A-algorithm.md)). Source: [qos §13](../design/qos-fault-routing.md).

| ID | Title | Status | Phase | Code anchor | Spec needed |
|----|-------|--------|-------|-------------|-------------|
| P1 | **Publisher GBR declaration** — publishers are *implicit* (inferred from traffic); no `DeclarePublisher` message exists. | open | [P6](../phases/06-gbr-declaration-policing.md) | `commons/zenoh-protocol/src/network/declare.rs` (no publisher variant) | new declaration message/field carrying GBR (hard-state, lifetime = face) |
| P2 | **Segment stack** — extend `NodeId` routing-context ext `0x3` (single `u16`) → `SegmentList`. | open | [P3](../phases/03-segment-stack-ext.md) | `network/push.rs:63` (ext 0x3, `NodeIdType.node_id:u16`) | list-typed ext; empty = legacy single-hop behavior |
| P3 | **Scope tag + detour-flag** on detoured packets (coarse region/edge-node set). | open | [P8](../phases/08-multicast-scoped-detour.md) | rides on the P2 ext | scope field + flag; honored end-to-end to region-local delivery |
| P4 | **Per-link capacity `C_e`** carried in link-state. | open | [P1](../phases/01-observability-capacity-signal.md) | `linkstate.rs:69` (weight `u16` exists); `network.rs:148` `link_weights` | extend LSP with capacity (integer) |
| P5 | **Edge preemption-notification** — edge publishers run no admission and get no signal; need explicit "you were preempted, reroute". | open | [P9](../phases/09-flow-preemption.md) | n/a (new) | backbone→edge notify message |
| P6 | *(fallback)* **BIER bitmap** header for huge sparse flat receiver sets. | research | — | n/a | bit-indexed header + deterministic bit assignment (recurses into [A-A](class-A-algorithm.md)) |

Note: P2 is the keystone — P3 rides on it, and it's the smallest change that unlocks detour/TE/repair. P4 rides on the existing `link_weights` LSP machinery.

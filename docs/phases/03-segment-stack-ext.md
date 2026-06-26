# Phase 03 — Segment-stack wire extension (keystone)

**Goal:** generalize the single `NodeId` routing hint into a **segment list** on the packet. The smallest wire change that unlocks detour / TE-pin / repair. Empty = legacy.

**Depends on:** none (parallel with 01/02).
**Delivers:** [P2](../issues/class-P-protocol.md).
**Unblocks:** 04 (TI-LFA), 08 (scoped detour).

## Deliverables
1. Extend ext `0x3` (`network/push.rs:63`, `NodeIdType.node_id:u16`) → `SegmentList { segs: SmallVec<u16> }`.
2. Forwarding: pop top segment on arrival; **empty stack == today's single-hop behavior** (strict backward-compat).
3. Prefer **loose** segments ("reach waypoint somehow", SPF fills gaps) → shallow stack, survives intra-segment failure.
4. Version-negotiate the ext; absent = legacy. Keep depth shallow (1–3 covers most TE/FRR).
5. Preserve the **tree/source context** semantics on the packet (needed for dup-free multicast, [qos §8.6](../design/qos-fault-routing.md)).

## Exit criteria
- Mixed legacy/new nodes interoperate (empty stack path unchanged).
- A manually-injected segment steers a packet through a chosen waypoint and pops correctly.

## Risks / issues
- Header overhead for tiny messages → only non-empty for steered flows ([H](../issues/class-A-algorithm.md)).
- Widens rolling-upgrade surface ([I](../issues/class-A-algorithm.md)) — version-gate.

## Design refs
[qos §0, §5](../design/qos-fault-routing.md).

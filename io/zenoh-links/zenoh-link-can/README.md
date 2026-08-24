# ⚠️ WARNING ⚠️

This crate is intended for Zenoh's internal use.
It is not guaranteed that the API will remain unchanged in any version, including patch updates.
It is highly recommended to depend solely on the zenoh and zenoh-ext crates and to utilize their public APIs.

- [Click here for Zenoh's main repository](https://github.com/eclipse-zenoh/zenoh)
- [Click here for Zenoh's documentation](https://zenoh.io)

## The CAN link

A CAN and CAN FD transport, behind the `transport_can` feature. Linux only: it is built on SocketCAN.

A CAN bus is a broadcast medium — every node hears every frame and filters by identifier — so this is a **multicast** link. Each peer owns one identifier, transmits on it, accepts frames from every other identifier the mask admits, and drops its own. The sender's identifier is that peer's address.

CAN frames are bounded and self-delimiting, so this is a **datagram** link: zenoh's transport fragments anything larger than the MTU, and the link itself needs no segmentation or reassembly.

### Endpoints

```text
can/<device>#bitrate=500000;dbitrate=2000000;id=0x100;match=0;mask=0
```

| key | meaning |
| --- | --- |
| `device` | the CAN interface name, such as `can0` or `vcan0` |
| `bitrate` | arbitration-phase bit rate, and the sole rate for classic CAN |
| `dbitrate` | CAN FD data-phase bit rate. `0` selects classic CAN |
| `id` | **this** peer's identifier, and its address on the bus |
| `match` | accept frames whose `(id & mask) == match` |
| `mask` | `0`, the default, accepts every identifier on the bus |

On Linux the bit rates are advisory: rates are set out of band with `ip link set can0 type can bitrate ...`, and a virtual interface has none at all. `dbitrate` is still load-bearing, because `0` selects classic framing.

The MTU is 63 bytes with CAN FD and 7 with classic CAN — one frame, less a one-byte length prefix. The link reports the mode it actually obtained rather than the one requested.

### Identifier value is bus priority

A **lower identifier wins arbitration**, so `id` is a real-time decision and not a name. The peer that must not be delayed needs the lower identifier.

The defaults are a starting point, not an allocation. Two peers that both accept them differ only by whichever was configured first, which is a priority ordering nobody chose. Priority is also per **peer**, not per message: one identifier carries all of a peer's traffic.

Only 11-bit identifiers are supported. `id`, `match` and `mask` above `0x7FF` are refused at open.

### Testing without hardware

The link runs against a virtual bus, which needs no CAN controller:

```sh
ci/vcan-setup.sh              # create and bring up vcan0, prompting for sudo
ci/vcan-setup.sh --status     # report, changing nothing
ci/vcan-setup.sh --down       # tear it down again
```

which is the equivalent of:

```sh
sudo modprobe vcan
sudo ip link add dev vcan0 type vcan
sudo ip link set up vcan0
```

`candump -td vcan0` then shows every frame. The end-to-end tests live in `io/zenoh-transport/tests/multicast_can.rs` and are `#[ignore]`d, so run them deliberately:

```sh
cargo test -p zenoh-transport --features transport_can --test multicast_can -- --ignored --nocapture
```

//
// Copyright (c) 2026 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
//

//! End-to-end tests for the CAN link, over a virtual bus.
//!
//! These need a `vcan0` interface, which creating requires root:
//!
//! ```sh
//! ci/vcan-setup.sh
//! ```
//!
//! which is the equivalent of `modprobe vcan`, `ip link add dev vcan0 type
//! vcan` and `ip link set up vcan0`.
//!
//! They are `#[ignore]`d so no CI job runs them by accident, and they also skip
//! at runtime with an explanation when the interface is absent, so running them
//! deliberately on a machine without one reports why rather than failing.
//!
//! Run them with:
//!
//! ```sh
//! cargo test -p zenoh-transport --features transport_can --test multicast_can -- --ignored --nocapture
//! ```
//!
//! `candump -td vcan0` in another terminal shows every frame.

#[cfg(all(feature = "transport_can", target_os = "linux"))]
mod tests {
    use std::{
        any::Any,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use zenoh_buffers::buffer::SplitBuffer;
    use zenoh_core::ztimeout;
    use zenoh_link::Link;
    use zenoh_protocol::{
        core::{
            Channel, CongestionControl, EndPoint, Priority, Reliability, WhatAmI, ZenohIdProto,
        },
        network::{
            push::{ext::QoSType, Push},
            NetworkBodyMut, NetworkMessage, NetworkMessageMut,
        },
        zenoh::PushBody,
    };
    use zenoh_result::ZResult;
    use zenoh_transport::{
        multicast::TransportMulticast, unicast::TransportUnicast, TransportEventHandler,
        TransportManager, TransportMulticastEventHandler, TransportPeer, TransportPeerEventHandler,
    };

    const TIMEOUT: Duration = Duration::from_secs(60);
    const SLEEP: Duration = Duration::from_secs(1);
    const SLEEP_COUNT: Duration = Duration::from_millis(10);

    /// Far fewer than the UDP test's 1 000: every message here is fragmented
    /// across a 63-byte MTU, so this is already thousands of frames.
    const MSG_COUNT: usize = 100;

    /// Well above the link MTU, so zenoh's own fragmentation drives the link.
    /// 189 bytes is the figure phase-377 measured with two zenoh-pico peers, so
    /// the two phases are directly comparable.
    const MSG_SIZE_FRAGMENTED: [usize; 1] = [189];

    /// Past a single batch, so fragmentation is exercised well beyond one frame
    /// worth of bookkeeping.
    const MSG_SIZE_LARGE: [usize; 1] = [4_096];

    const DEVICE: &str = "vcan0";

    /// Whether the virtual bus exists. Creating one needs root, so a developer
    /// without it gets an explanation rather than a failure.
    fn vcan_present() -> bool {
        if std::path::Path::new(&format!("/sys/class/net/{DEVICE}")).exists() {
            return true;
        }
        println!("SKIPPING: no `{DEVICE}` interface. Create one with `ci/vcan-setup.sh`.");
        false
    }

    /// Counts what arrives and checks it byte for byte.
    ///
    /// Counting alone would not catch a reassembly bug: a message split across
    /// 71 CAN frames and put back together wrongly still arrives.
    struct SHPeer {
        count: Arc<AtomicUsize>,
        corrupt: Arc<AtomicUsize>,
        expected: Arc<Vec<u8>>,
    }

    impl SHPeer {
        fn new(expected: Vec<u8>) -> Self {
            Self {
                count: Arc::new(AtomicUsize::new(0)),
                corrupt: Arc::new(AtomicUsize::new(0)),
                expected: Arc::new(expected),
            }
        }

        fn get_count(&self) -> usize {
            self.count.load(Ordering::Relaxed)
        }

        fn get_corrupt(&self) -> usize {
            self.corrupt.load(Ordering::Relaxed)
        }
    }

    impl TransportEventHandler for SHPeer {
        fn new_unicast(
            &self,
            _peer: TransportPeer,
            _transport: TransportUnicast,
        ) -> ZResult<Arc<dyn TransportPeerEventHandler>> {
            panic!("a CAN link is multicast only");
        }

        fn new_multicast(
            &self,
            _transport: TransportMulticast,
        ) -> ZResult<Arc<dyn TransportMulticastEventHandler>> {
            Ok(Arc::new(SCPeer::new(
                self.count.clone(),
                self.corrupt.clone(),
                self.expected.clone(),
            )))
        }
    }

    pub struct SCPeer {
        count: Arc<AtomicUsize>,
        corrupt: Arc<AtomicUsize>,
        expected: Arc<Vec<u8>>,
    }

    impl SCPeer {
        pub fn new(
            count: Arc<AtomicUsize>,
            corrupt: Arc<AtomicUsize>,
            expected: Arc<Vec<u8>>,
        ) -> Self {
            Self {
                count,
                corrupt,
                expected,
            }
        }
    }

    impl TransportMulticastEventHandler for SCPeer {
        fn new_peer(&self, peer: TransportPeer) -> ZResult<Arc<dyn TransportPeerEventHandler>> {
            println!("\tNew peer: {}", peer.zid);
            Ok(Arc::new(SCPeer {
                count: self.count.clone(),
                corrupt: self.corrupt.clone(),
                expected: self.expected.clone(),
            }))
        }
        fn closed(&self) {}

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl TransportPeerEventHandler for SCPeer {
        fn handle_message(&self, msg: NetworkMessageMut) -> ZResult<()> {
            if let NetworkBodyMut::Push(push) = msg.body {
                if let PushBody::Put(put) = &push.payload {
                    if put.payload.contiguous().as_ref() != self.expected.as_slice() {
                        self.corrupt.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            self.count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn new_link(&self, _link: Link) {}
        fn del_link(&self, _link: Link) {}
        fn closed(&self) {}

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    struct TransportMulticastPeer {
        manager: TransportManager,
        handler: Arc<SHPeer>,
        transport: TransportMulticast,
    }

    /// Unlike the UDP multicast test, the two peers open *different* endpoints:
    /// on a CAN bus a peer's identifier is its address, and a peer drops frames
    /// carrying its own identifier. Two peers sharing one `id` would each
    /// discard everything the other sent.
    async fn open_transport(
        endpoint01: &EndPoint,
        endpoint02: &EndPoint,
        payload: &[u8],
    ) -> (TransportMulticastPeer, TransportMulticastPeer) {
        let peer01_id = ZenohIdProto::try_from([1]).unwrap();
        let peer02_id = ZenohIdProto::try_from([2]).unwrap();

        let peer01_handler = Arc::new(SHPeer::new(payload.to_vec()));
        let peer01_manager = TransportManager::builder()
            .zid(peer01_id)
            .whatami(WhatAmI::Peer)
            .build_test(peer01_handler.clone())
            .unwrap();

        let peer02_handler = Arc::new(SHPeer::new(payload.to_vec()));
        let peer02_manager = TransportManager::builder()
            .whatami(WhatAmI::Peer)
            .zid(peer02_id)
            .build_test(peer02_handler.clone())
            .unwrap();

        println!("Opening transport with {endpoint01}");
        let t01 = ztimeout!(peer01_manager.open_transport_multicast(endpoint01.clone())).unwrap();
        assert!(!ztimeout!(peer01_manager.get_transports_multicast()).is_empty());
        println!("\tPeer01 link MTU: {}", t01.get_link().unwrap().mtu);

        println!("Opening transport with {endpoint02}");
        let t02 = ztimeout!(peer02_manager.open_transport_multicast(endpoint02.clone())).unwrap();
        assert!(!ztimeout!(peer02_manager.get_transports_multicast()).is_empty());
        println!("\tPeer02 link MTU: {}", t02.get_link().unwrap().mtu);

        // The two peers find each other through the Join messages the multicast
        // transport emits; there is no handshake on a bus.
        ztimeout!(async {
            while peer01_manager
                .get_transport_multicast(&peer02_id)
                .await
                .is_none()
            {
                tokio::time::sleep(SLEEP_COUNT).await;
            }
        });
        let peer01_transport =
            ztimeout!(peer01_manager.get_transport_multicast(&peer02_id)).unwrap();
        println!("\tPeer01 peers: {:?}", peer01_transport.get_peers().unwrap());

        ztimeout!(async {
            while peer02_manager
                .get_transport_multicast(&peer01_id)
                .await
                .is_none()
            {
                tokio::time::sleep(SLEEP_COUNT).await;
            }
        });
        let peer02_transport =
            ztimeout!(peer02_manager.get_transport_multicast(&peer01_id)).unwrap();
        println!("\tPeer02 peers: {:?}", peer02_transport.get_peers().unwrap());

        (
            TransportMulticastPeer {
                manager: peer01_manager,
                handler: peer01_handler,
                transport: peer01_transport,
            },
            TransportMulticastPeer {
                manager: peer02_manager,
                handler: peer02_handler,
                transport: peer02_transport,
            },
        )
    }

    async fn close_transport(peer01: TransportMulticastPeer, peer02: TransportMulticastPeer) {
        println!("Closing peer01 transport");
        ztimeout!(peer01.transport.close()).unwrap();
        assert!(ztimeout!(peer01.manager.get_transports_multicast()).is_empty());
        ztimeout!(async {
            while !peer02.transport.get_peers().unwrap().is_empty() {
                tokio::time::sleep(SLEEP_COUNT).await;
            }
        });

        println!("Closing peer02 transport");
        ztimeout!(peer02.transport.close()).unwrap();
        assert!(ztimeout!(peer02.manager.get_transports_multicast()).is_empty());

        tokio::time::sleep(SLEEP).await;
    }

    /// A payload with structure, so a reassembly that puts the right number of
    /// bytes back in the wrong order is caught. All-zeros would not be.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    async fn test_transport(
        peer01: &TransportMulticastPeer,
        peer02: &TransportMulticastPeer,
        channel: Channel,
        payload: &[u8],
    ) {
        let message = NetworkMessage::from(Push {
            wire_expr: "test".into(),
            ext_qos: QoSType::new(channel.priority, CongestionControl::Block, false),
            ..Push::from(payload.to_vec())
        });

        println!(
            "Sending {MSG_COUNT} messages of {} bytes... {channel:?}",
            payload.len()
        );
        for _ in 0..MSG_COUNT {
            peer01.transport.schedule(message.clone().as_mut()).unwrap();
        }

        // Wait for delivery to settle rather than stopping at the first
        // message. Sampling until the count holds still gives a figure that
        // means something; stopping at `count != 0` reports how much happened
        // to have arrived by then, which is not a measurement of anything.
        ztimeout!(async {
            let mut last = usize::MAX;
            let mut stable = 0;
            loop {
                let now = peer02.handler.get_count();
                if now == MSG_COUNT {
                    break;
                }
                if now == last {
                    stable += 1;
                    // A full second with no new message.
                    if stable >= 100 {
                        break;
                    }
                } else {
                    stable = 0;
                    last = now;
                }
                tokio::time::sleep(SLEEP_COUNT).await;
            }
        });

        let received = peer02.handler.get_count();
        let corrupt = peer02.handler.get_corrupt();
        println!(
            "\tPeer02 received {received}/{MSG_COUNT} messages of {} bytes, {corrupt} corrupt",
            payload.len()
        );

        // The link is best-effort by nature — CAN is reliable per frame but not
        // end to end — so the contract is that traffic arrives, not that all of
        // it does. This mirrors the UDP multicast test.
        assert!(received > 0, "nothing arrived at all");
        // Whatever does arrive must be intact. A message reassembled from CAN
        // frames is either right or it is a bug; there is no partial credit.
        assert_eq!(corrupt, 0, "{corrupt} of {received} messages were corrupt");

        tokio::time::sleep(SLEEP).await;
    }

    async fn run(endpoints: (&EndPoint, &EndPoint), channel: &[Channel], msg_size: &[usize]) {
        for ch in channel.iter() {
            for ms in msg_size.iter() {
                let payload = pattern(*ms);
                let (peer01, peer02) = open_transport(endpoints.0, endpoints.1, &payload).await;
                test_transport(&peer01, &peer02, *ch, &payload).await;
                close_transport(peer01, peer02).await;
            }
        }
    }

    fn channels() -> [Channel; 2] {
        [
            Channel {
                priority: Priority::DEFAULT,
                reliability: Reliability::BestEffort,
            },
            Channel {
                priority: Priority::RealTime,
                reliability: Reliability::BestEffort,
            },
        ]
    }

    /// phase-378 W4: two zenoh-rs peers exchange a payload that does not fit one
    /// frame, so the transport's own fragmentation is driving the link.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs a vcan0 interface; see the module docs"]
    async fn transport_multicast_can_fragmented() {
        zenoh_util::init_log_from_env_or("error");
        if !vcan_present() {
            return;
        }

        let e01: EndPoint = format!("can/{DEVICE}#id=0x100").parse().unwrap();
        let e02: EndPoint = format!("can/{DEVICE}#id=0x101").parse().unwrap();
        run((&e01, &e02), &channels(), &MSG_SIZE_FRAGMENTED).await;
    }

    /// phase-378 W4: the same, well past a single batch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs a vcan0 interface; see the module docs"]
    async fn transport_multicast_can_large() {
        zenoh_util::init_log_from_env_or("error");
        if !vcan_present() {
            return;
        }

        // 100 messages of 4 KiB is 7 100 frames, and a virtual bus delivers them
        // as fast as memory allows — far faster than any real bus, where
        // 2 Mbit/s of CAN FD is under 2 800 frames per second. Without a bigger
        // receive buffer the kernel drops the overflow before the link sees it,
        // and 31% of messages are lost with no error anywhere. This is the
        // knob for that, and this test is its demonstration.
        let e01: EndPoint = format!("can/{DEVICE}#id=0x110;so_rcvbuf=8388608")
            .parse()
            .unwrap();
        let e02: EndPoint = format!("can/{DEVICE}#id=0x111;so_rcvbuf=8388608")
            .parse()
            .unwrap();
        run((&e01, &e02), &channels()[..1], &MSG_SIZE_LARGE).await;
    }

    /// phase-378 W2: the link reports the MTU of the mode it actually obtained,
    /// and a band that excludes a peer's own identifier is refused at open.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs a vcan0 interface; see the module docs"]
    async fn can_link_open_reports_its_mode() {
        zenoh_util::init_log_from_env_or("error");
        if !vcan_present() {
            return;
        }

        let handler = Arc::new(SHPeer::new(Vec::new()));
        let manager = TransportManager::builder()
            .zid(ZenohIdProto::try_from([3]).unwrap())
            .whatami(WhatAmI::Peer)
            .build_test(handler)
            .unwrap();

        let ep: EndPoint = format!("can/{DEVICE}#id=0x120").parse().unwrap();
        let transport = ztimeout!(manager.open_transport_multicast(ep)).unwrap();
        let mtu = transport.get_link().unwrap().mtu;
        // 63 with CAN FD, 7 if the interface refused it. A virtual bus supports
        // FD, so anything else means the negotiation regressed.
        assert_eq!(mtu, 63, "vcan0 should negotiate CAN FD");
        ztimeout!(transport.close()).unwrap();

        // An identifier outside its own band could never be addressed, so the
        // link refuses it rather than degrading quietly.
        let bad: EndPoint = format!("can/{DEVICE}#id=0x201;match=0x100;mask=0x700")
            .parse()
            .unwrap();
        let err = ztimeout!(manager.open_transport_multicast(bad)).unwrap_err();
        assert!(
            err.to_string().contains("outside its own"),
            "unexpected error: {err}"
        );

        // An over-long interface name is refused before open, with the limit
        // named, rather than truncating into some other interface.
        let long: EndPoint = "can/vcan-nonexistent".parse().unwrap();
        let err = ztimeout!(manager.open_transport_multicast(long)).unwrap_err();
        assert!(
            err.to_string().contains("at most 15"),
            "unexpected error: {err}"
        );

        // No interface of that name, and the error should say so. The name is
        // kept under IFNAMSIZ so this exercises the missing-interface path
        // rather than the name-length guard.
        let missing: EndPoint = "can/vcan9nope".parse().unwrap();
        let err = ztimeout!(manager.open_transport_multicast(missing)).unwrap_err();
        assert!(
            err.to_string().contains("no such interface"),
            "unexpected error: {err}"
        );
    }
}

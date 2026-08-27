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

//! End-to-end tests for the ISO-TP unicast CAN link (phase-393 W4).
//!
//! These need a `vcan0` interface, which creating requires root:
//!
//! ```sh
//! ci/vcan-setup.sh
//! ```
//!
//! The `can-isotp` kernel module does **not** need loading by hand — it carries
//! `alias can-proto-6` and autoloads on the first socket, even unprivileged.
//!
//! ```sh
//! cargo test -p zenoh-transport --features transport_isotp \
//!     --test unicast_isotp -- --ignored --nocapture
//! ```

#[cfg(all(feature = "transport_isotp", target_os = "linux"))]
mod tests {
    use std::{
        any::Any,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use zenoh_core::ztimeout;
    use zenoh_link::Link;
    use zenoh_protocol::{
        core::{EndPoint, WhatAmI, ZenohIdProto},
        network::NetworkMessageMut,
    };
    use zenoh_result::ZResult;
    use zenoh_transport::{
        multicast::TransportMulticast, unicast::TransportUnicast, TransportEventHandler,
        TransportManager, TransportMulticastEventHandler, TransportPeer, TransportPeerEventHandler,
    };

    const TIMEOUT: Duration = Duration::from_secs(60);
    const SLEEP: Duration = Duration::from_secs(1);
    const DEVICE: &str = "vcan0";

    fn vcan_present() -> bool {
        if std::path::Path::new(&format!("/sys/class/net/{DEVICE}")).exists() {
            return true;
        }
        println!("SKIPPING: no `{DEVICE}` interface. Create one with `ci/vcan-setup.sh`.");
        false
    }

    #[derive(Default)]
    struct SHPeer {
        count: Arc<AtomicUsize>,
    }

    impl TransportEventHandler for SHPeer {
        fn new_unicast(
            &self,
            _peer: TransportPeer,
            _transport: TransportUnicast,
        ) -> ZResult<Arc<dyn TransportPeerEventHandler>> {
            Ok(Arc::new(SCPeer {
                count: self.count.clone(),
            }))
        }

        fn new_multicast(
            &self,
            _transport: TransportMulticast,
        ) -> ZResult<Arc<dyn TransportMulticastEventHandler>> {
            panic!("ISO-TP is a unicast link");
        }
    }

    struct SCPeer {
        count: Arc<AtomicUsize>,
    }

    impl TransportPeerEventHandler for SCPeer {
        fn handle_message(&self, _msg: NetworkMessageMut) -> ZResult<()> {
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

    /// The claim of phase-393 W4: a **unicast** zenoh transport comes up over a
    /// CAN bus. Everything ROS needs beyond topics follows from the transport
    /// being unicast, so this is the load-bearing assertion.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs a vcan0 interface; see the module docs"]
    async fn transport_unicast_isotp_opens() {
        zenoh_util::init_log_from_env_or("error");
        if !vcan_present() {
            return;
        }

        // A directed pair: each side's tx is the other's rx.
        let listen: EndPoint = format!("isotp/{DEVICE}#tx_id=0x7E8;rx_id=0x7E0")
            .parse()
            .unwrap();
        let connect: EndPoint = format!("isotp/{DEVICE}#tx_id=0x7E0;rx_id=0x7E8")
            .parse()
            .unwrap();

        let server_handler = Arc::new(SHPeer::default());
        let server = TransportManager::builder()
            .zid(ZenohIdProto::try_from([1]).unwrap())
            .whatami(WhatAmI::Peer)
            .build_test(server_handler.clone())
            .unwrap();

        let client_handler = Arc::new(SHPeer::default());
        let client = TransportManager::builder()
            .zid(ZenohIdProto::try_from([2]).unwrap())
            .whatami(WhatAmI::Peer)
            .build_test(client_handler.clone())
            .unwrap();

        println!("listening on {listen}");
        ztimeout!(server.add_listener(listen.clone())).unwrap();

        println!("connecting to {connect}");
        let transport = ztimeout!(client.open_transport_unicast(connect.clone())).unwrap();

        // The whole point: a UNICAST transport. Asserted rather than inferred,
        // because a link that silently reported multicast would still carry
        // topics and would still lose every query.
        let links = transport.get_links().unwrap();
        assert_eq!(links.len(), 1);
        println!("\tlink MTU {}", links[0].mtu);
        assert_eq!(
            links[0].mtu, 4095,
            "ISO-TP should give a 4095-byte MTU, not a single CAN frame"
        );

        assert!(!ztimeout!(client.get_transports_unicast()).is_empty());
        assert!(!ztimeout!(server.get_transports_unicast()).is_empty());

        ztimeout!(transport.close()).unwrap();
        tokio::time::sleep(SLEEP).await;
        ztimeout!(server.del_listener(&listen)).unwrap();
    }
}

//
// Copyright (c) 2017, 2020 ADLINK Technology Inc.
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ADLINK zenoh team, <zenoh@adlink-labs.tech>
//
use async_std::future;
use async_std::sync::Arc;
use async_std::task;
use async_trait::async_trait;
use rand::RngCore;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zenoh_protocol::core::{whatami, PeerId};
use zenoh_protocol::link::{Link, Locator};
use zenoh_protocol::proto::{Data, ZenohBody, ZenohMessage};
use zenoh_protocol::session::{
    Session, SessionEventHandler, SessionHandler, SessionManager, SessionManagerConfig,
};
use zenoh_util::core::ZResult;

// Session Handler for the peer
struct MySH;

impl MySH {
    fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SessionHandler for MySH {
    async fn new_session(
        &self,
        _session: Session,
    ) -> ZResult<Arc<dyn SessionEventHandler + Send + Sync>> {
        Ok(Arc::new(MyMH::new()))
    }
}

// Message Handler for the peer
struct MyMH;

impl MyMH {
    fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SessionEventHandler for MyMH {
    async fn handle_message(&self, message: ZenohMessage) -> ZResult<()> {
        match message.body {
            ZenohBody::Data(Data { mut payload, .. }) => {
                let mut count_bytes = [0u8; 8];
                payload.read_bytes(&mut count_bytes).unwrap();
                let count = u64::from_le_bytes(count_bytes);

                let mut now_bytes = [0u8; 16];
                payload.read_bytes(&mut now_bytes).unwrap();
                let now_pub = u128::from_le_bytes(now_bytes);

                let now_sub = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let interval = Duration::from_nanos((now_sub - now_pub) as u64);

                println!("{} bytes: seq={} time={:?}", payload.len(), count, interval);
            }
            _ => panic!("Invalid message"),
        }

        Ok(())
    }

    async fn new_link(&self, _link: Link) {}
    async fn del_link(&self, _link: Link) {}
    async fn closing(&self) {}
    async fn closed(&self) {}
}

fn print_usage(bin: String) {
    println!(
        "Usage:
    cargo run --release --bin {} <locator to listen on>
Example:
    cargo run --release --bin {} tcp/127.0.0.1:7447",
        bin, bin
    );
}

fn main() {
    // Enable logging
    env_logger::init();

    // Initialize the Peer Id
    let mut pid = [0u8; PeerId::MAX_SIZE];
    rand::thread_rng().fill_bytes(&mut pid);
    let pid = PeerId::new(1, pid);

    let config = SessionManagerConfig {
        version: 0,
        whatami: whatami::PEER,
        id: pid,
        handler: Arc::new(MySH::new()),
    };
    let manager = SessionManager::new(config, None);

    let mut args = std::env::args();
    // Get exe name
    let bin = args
        .next()
        .unwrap()
        .split(std::path::MAIN_SEPARATOR)
        .last()
        .unwrap()
        .to_string();

    // Get next arg
    let value = if let Some(value) = args.next() {
        value
    } else {
        return print_usage(bin);
    };
    let listen_on: Locator = if let Ok(v) = value.parse() {
        v
    } else {
        return print_usage(bin);
    };

    // Connect to publisher
    task::block_on(async {
        let _ = manager.add_listener(&listen_on).await.unwrap();
        // Stop forever
        future::pending::<()>().await;
    });
}

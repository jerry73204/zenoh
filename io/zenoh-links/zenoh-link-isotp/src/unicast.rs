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

use async_trait::async_trait;
use zenoh_link_commons::{LinkManagerUnicastTrait, LinkUnicast, NewLinkChannelSender};
use zenoh_protocol::core::{EndPoint, Locator};
use zenoh_result::{bail, ZResult};

use crate::IsotpEndpoint;

/// An ISO-TP channel is a directed identifier pair, so peers do not discover
/// one another: each side is configured with the other's identifier. One side
/// listens and the other connects, exactly as the serial link does — there is
/// no `accept()` on a bus, and none is needed.
pub struct LinkManagerUnicastIsotp {
    #[allow(dead_code)]
    manager: NewLinkChannelSender,
}

impl LinkManagerUnicastIsotp {
    pub fn new(manager: NewLinkChannelSender) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl LinkManagerUnicastTrait for LinkManagerUnicastIsotp {
    async fn new_link(&self, endpoint: EndPoint) -> ZResult<LinkUnicast> {
        // Parse and validate on every platform, so a malformed endpoint is
        // reported as malformed rather than as a missing platform.
        let ep = IsotpEndpoint::parse(&endpoint)?;
        new_link_inner(ep).await
    }

    async fn new_listener(&self, endpoint: EndPoint) -> ZResult<Locator> {
        let ep = IsotpEndpoint::parse(&endpoint)?;
        new_listener_inner(ep).await
    }

    async fn del_listener(&self, _endpoint: &EndPoint) -> ZResult<()> {
        bail!("ISO-TP: listeners are not implemented yet")
    }

    async fn get_listeners(&self) -> Vec<EndPoint> {
        Vec::new()
    }

    async fn get_locators(&self) -> Vec<Locator> {
        Vec::new()
    }

    async fn get_locators_noloopback(&self) -> Vec<Locator> {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
async fn new_link_inner(_ep: IsotpEndpoint) -> ZResult<LinkUnicast> {
    bail!("ISO-TP: connecting is not implemented yet (phase-393 W1/W2)")
}

#[cfg(target_os = "linux")]
async fn new_listener_inner(_ep: IsotpEndpoint) -> ZResult<Locator> {
    bail!("ISO-TP: listening is not implemented yet (phase-393 W1/W2)")
}

#[cfg(not(target_os = "linux"))]
async fn new_link_inner(ep: IsotpEndpoint) -> ZResult<LinkUnicast> {
    bail!(
        "ISO-TP is a Linux kernel protocol (PF_CAN/CAN_ISOTP); cannot open {:?} on this platform",
        ep.device
    )
}

#[cfg(not(target_os = "linux"))]
async fn new_listener_inner(ep: IsotpEndpoint) -> ZResult<Locator> {
    bail!(
        "ISO-TP is a Linux kernel protocol (PF_CAN/CAN_ISOTP); cannot listen on {:?} on this platform",
        ep.device
    )
}

//
// Copyright (c) 2022 ZettaScale Technology
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
use super::network::shared_nodes;
pub use super::pubsub::*;
pub use super::queries::*;
pub use super::resource::*;
use super::router::Tables;
use async_std::task::spawn_blocking;
use futures::{join, StreamExt};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use zenoh_protocol::proto::LinkState;
use zenoh_protocol_core::{PeerId, WhatAmI};
use zenoh_transport::TransportUnicast;
// use zenoh_collections::Timer;
use itertools::Itertools as _;

pub(crate) struct UpdateTask {
    pub task: async_std::task::JoinHandle<()>,
    pub req_tx: flume::Sender<UpdateRequest>,
}

impl UpdateTask {
    pub fn new(tables: Arc<RwLock<Tables>>) -> Self {
        let (req_tx, req_rx) = flume::bounded(16);
        let (batch_tx, batch_rx) = flume::bounded(16);

        let batcher = Self::batcher(req_rx, batch_tx);
        let updater = spawn_blocking(move || {
            Self::updater(tables, batch_rx);
        });

        let task = async_std::task::spawn(async move {
            join!(batcher, updater);
        });

        Self { task, req_tx }
    }

    pub fn push(&self, req: impl Into<UpdateRequest>) -> bool {
        let req = req.into();
        self.req_tx.try_send(req).is_ok()
    }

    pub fn join(self) {
        let Self { task, req_tx: _ } = self;
        let _ = async_std::task::block_on(task);
    }

    async fn batcher(
        input_rx: flume::Receiver<UpdateRequest>,
        batch_tx: flume::Sender<UpdateBatch>,
    ) {
        loop {
            let mut link_state_updates = vec![];
            let mut add_links = vec![];
            let mut remove_links = vec![];

            let reqs: Vec<_> = input_rx
                .stream()
                .take_until(async move {
                    async_std::task::sleep(Duration::from_millis(100)).await;
                })
                .collect()
                .await;

            reqs.into_iter().for_each(|req| match req {
                UpdateRequest::LinkStateUpdate(req) => link_state_updates.push(req),
                UpdateRequest::AddLink(req) => add_links.push(req),
                UpdateRequest::RemoveLink(req) => remove_links.push(req),
            });

            let link_state_updates: Vec<_> = link_state_updates
                .into_iter()
                .map(|req| {
                    let LinkStateUpdate { list, pid, whatami } = req;
                    ((pid, whatami), list)
                })
                .into_group_map()
                .into_iter()
                .map(|((pid, whatami), lists)| {
                    let states = lists.into_iter().flatten();
                    let states = merge_link_state_list(states);

                    LinkStateUpdate {
                        list: states,
                        pid,
                        whatami,
                    }
                })
                .collect();

            let batch = UpdateBatch {
                link_state_updates,
                add_links,
                remove_links,
            };

            if batch.is_empty() {
                continue;
            }

            let ok = batch_tx.send_async(batch).await.is_ok();
            if !ok {
                break;
            }
        }
    }

    fn updater(tables: Arc<RwLock<Tables>>, batch_rx: flume::Receiver<UpdateBatch>) {
        batch_rx.into_iter().for_each(|batch| {
            Self::update_batch(&mut *zwrite!(tables), batch);
        });
    }

    fn update_batch(tables: &mut Tables, batch: UpdateBatch) {
        use WhatAmI::*;
        debug_assert!(!batch.is_empty());

        let UpdateBatch {
            link_state_updates,
            add_links,
            remove_links,
        } = batch;

        let mut compute_router_trees = false;
        let mut compute_peer_trees = false;

        add_links.into_iter().for_each(|req| {
            let AddLink {
                transport,
                link_id_tx,
            } = req;
            let whatami = match transport.get_whatami() {
                Ok(whatami) => whatami,
                Err(_) => {
                    log::error!("Closed transport in session closing!");
                    return;
                }
            };

            let link_id = match (tables.whatami, whatami) {
                (Router, Router) => {
                    compute_router_trees = true;
                    tables.routers_net.as_mut().unwrap().add_link(transport)
                }
                (Router, Peer) | (Peer, Router | Peer) => {
                    compute_peer_trees = true;
                    tables.peers_net.as_mut().unwrap().add_link(transport)
                }
                _ => return,
            };

            let _ = link_id_tx.send(link_id);
        });

        remove_links.into_iter().for_each(|req| {
            let RemoveLink { transport } = req;
            let pid = transport.get_pid();
            let whatami = transport.get_whatami();

            let (pid, whatami) = match (pid, whatami) {
                (Ok(pid), Ok(whatami)) => (pid, whatami),
                _ => {
                    log::error!("Closed transport in session closing!");
                    return;
                }
            };

            match (tables.whatami, whatami) {
                (Router, Router) => {
                    let removed_nodes = tables.routers_net.as_mut().unwrap().remove_link(&pid);

                    for (_, removed_node) in removed_nodes {
                        pubsub_remove_node(tables, &removed_node.pid, Router);
                        queries_remove_node(tables, &removed_node.pid, Router);
                    }

                    compute_router_trees = true;
                }
                (Router, Peer) | (Peer, Router | Peer) => {
                    let removed_nodes = tables.peers_net.as_mut().unwrap().remove_link(&pid);

                    for (_, removed_node) in removed_nodes {
                        pubsub_remove_node(tables, &removed_node.pid, Peer);
                        queries_remove_node(tables, &removed_node.pid, Peer);
                    }

                    compute_peer_trees = true;
                }
                _ => (),
            };
        });

        link_state_updates.into_iter().for_each(|req| {
            let LinkStateUpdate { list, pid, whatami } = req;

            match (tables.whatami, whatami) {
                (Router, Router) => {
                    let removed_nodes = tables.routers_net.as_mut().unwrap().link_states(list, pid);

                    for (_, removed_node) in removed_nodes {
                        pubsub_remove_node(tables, &removed_node.pid, Router);
                        queries_remove_node(tables, &removed_node.pid, Router);
                    }

                    compute_router_trees = true;
                }
                (Router, Peer) | (Peer, Router | Peer) => {
                    let removed_nodes = tables.peers_net.as_mut().unwrap().link_states(list, pid);

                    for (_, removed_node) in removed_nodes {
                        pubsub_remove_node(tables, &removed_node.pid, Peer);
                        queries_remove_node(tables, &removed_node.pid, Peer);
                    }

                    compute_peer_trees = true;
                }
                _ => (),
            }
        });

        // update shared nodes
        if tables.whatami == WhatAmI::Router {
            tables.shared_nodes = shared_nodes(
                tables.routers_net.as_ref().unwrap(),
                tables.peers_net.as_ref().unwrap(),
            );
        }

        if compute_router_trees {
            Self::compute_router_trees(tables);
        }

        if compute_peer_trees {
            Self::compute_peer_trees(tables);
        }
    }

    fn compute_router_trees(tables: &mut Tables) {
        log::trace!("Compute trees");
        let new_childs = tables.routers_net.as_mut().unwrap().compute_trees();

        log::trace!("Compute routes");
        pubsub_tree_change(tables, &new_childs, WhatAmI::Router);
        queries_tree_change(tables, &new_childs, WhatAmI::Router);

        log::trace!("Computations completed");
    }

    fn compute_peer_trees(tables: &mut Tables) {
        log::trace!("Compute trees");
        let new_childs = tables.peers_net.as_mut().unwrap().compute_trees();

        log::trace!("Compute routes");
        pubsub_tree_change(tables, &new_childs, WhatAmI::Peer);
        queries_tree_change(tables, &new_childs, WhatAmI::Peer);

        log::trace!("Computations completed");
    }
}

struct UpdateBatch {
    pub link_state_updates: Vec<LinkStateUpdate>,
    pub add_links: Vec<AddLink>,
    pub remove_links: Vec<RemoveLink>,
}

impl UpdateBatch {
    pub fn is_empty(&self) -> bool {
        self.link_state_updates.is_empty()
            && self.add_links.is_empty()
            && self.remove_links.is_empty()
    }
}

pub(crate) enum UpdateRequest {
    LinkStateUpdate(LinkStateUpdate),
    AddLink(AddLink),
    RemoveLink(RemoveLink),
}

impl From<RemoveLink> for UpdateRequest {
    fn from(v: RemoveLink) -> Self {
        Self::RemoveLink(v)
    }
}

impl From<AddLink> for UpdateRequest {
    fn from(v: AddLink) -> Self {
        Self::AddLink(v)
    }
}

impl From<LinkStateUpdate> for UpdateRequest {
    fn from(v: LinkStateUpdate) -> Self {
        Self::LinkStateUpdate(v)
    }
}

pub(crate) struct LinkStateUpdate {
    pub list: Vec<LinkState>,
    pub pid: PeerId,
    pub whatami: WhatAmI,
}

pub(crate) struct AddLink {
    pub transport: TransportUnicast,
    pub link_id_tx: flume::Sender<usize>,
}

pub(crate) struct RemoveLink {
    pub transport: TransportUnicast,
}

fn merge_link_state_list<I>(list: I) -> Vec<LinkState>
where
    I: IntoIterator<Item = LinkState>,
{
    use itertools::MinMaxResult;

    list.into_iter()
        .into_group_map_by(|state| state.psid)
        .into_iter()
        .map(|(psid, group)| {
            let pid = group.iter().find_map(|state| state.pid);
            let whatami = group.iter().find_map(|state| state.whatami);
            let locators = group
                .iter()
                .find_map(|state| state.locators.as_ref())
                .cloned();
            let newest_state = match group.iter().minmax_by_key(|state| state.sn) {
                MinMaxResult::NoElements => unreachable!(),
                MinMaxResult::OneElement(state) => state,
                MinMaxResult::MinMax(_, state) => state,
            };
            let sn = newest_state.sn;
            let links = newest_state.links.clone();

            LinkState {
                psid,
                sn,
                pid,
                whatami,
                locators,
                links,
            }
        })
        .collect()
}

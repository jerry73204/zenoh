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
use super::face::{Face, FaceState};
use super::network::{shared_nodes, Network};
pub use super::pubsub::*;
pub use super::queries::*;
pub use super::resource::*;
use super::runtime::Runtime;
use async_std::task::spawn_blocking;
use futures::{join, StreamExt};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Weak};
use std::sync::{Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uhlc::HLC;
use zenoh_link::Link;
use zenoh_protocol::proto::{LinkState, LinkStateList, ZenohBody, ZenohMessage};
use zenoh_protocol_core::{PeerId, WhatAmI, ZInt};
use zenoh_transport::{DeMux, Mux, Primitives, TransportPeerEventHandler, TransportUnicast};
// use zenoh_collections::Timer;
use itertools::Itertools as _;
use zenoh_core::zconfigurable;
use zenoh_core::Result as ZResult;
use zenoh_sync::get_mut_unchecked;

zconfigurable! {
    static ref TREES_COMPUTATION_DELAY: u64 = 100;
}

pub struct Tables {
    pub(crate) pid: PeerId,
    pub(crate) whatami: WhatAmI,
    face_counter: usize,
    #[allow(dead_code)]
    pub(crate) hlc: Option<Arc<HLC>>,
    // pub(crate) timer: Timer,
    // pub(crate) queries_default_timeout: Duration,
    pub(crate) root_res: Arc<Resource>,
    pub(crate) faces: HashMap<usize, Arc<FaceState>>,
    pub(crate) pull_caches_lock: Mutex<()>,
    pub(crate) router_subs: HashSet<Arc<Resource>>,
    pub(crate) peer_subs: HashSet<Arc<Resource>>,
    pub(crate) router_qabls: HashSet<Arc<Resource>>,
    pub(crate) peer_qabls: HashSet<Arc<Resource>>,
    pub(crate) routers_net: Option<Network>,
    pub(crate) peers_net: Option<Network>,
    pub(crate) shared_nodes: Vec<PeerId>,
}

impl Tables {
    pub fn new(
        pid: PeerId,
        whatami: WhatAmI,
        hlc: Option<Arc<HLC>>,
        _queries_default_timeout: Duration,
    ) -> Self {
        Tables {
            pid,
            whatami,
            face_counter: 0,
            hlc,
            // timer: Timer::new(true),
            // queries_default_timeout,
            root_res: Resource::root(),
            faces: HashMap::new(),
            pull_caches_lock: Mutex::new(()),
            router_subs: HashSet::new(),
            peer_subs: HashSet::new(),
            router_qabls: HashSet::new(),
            peer_qabls: HashSet::new(),
            routers_net: None,
            peers_net: None,
            shared_nodes: vec![],
            // routers_trees_task: None,
            // peers_trees_task: None,
        }
    }

    #[doc(hidden)]
    pub fn _get_root(&self) -> &Arc<Resource> {
        &self.root_res
    }

    pub fn print(&self) -> String {
        Resource::print_tree(&self.root_res)
    }

    #[inline]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(crate) fn get_mapping<'a>(
        &'a self,
        face: &'a FaceState,
        expr_id: &ZInt,
    ) -> Option<&'a Arc<Resource>> {
        match expr_id {
            0 => Some(&self.root_res),
            expr_id => face.get_mapping(expr_id),
        }
    }

    #[inline]
    pub(crate) fn get_net(&self, net_type: WhatAmI) -> Option<&Network> {
        match net_type {
            WhatAmI::Router => self.routers_net.as_ref(),
            WhatAmI::Peer => self.peers_net.as_ref(),
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn get_face(&self, pid: &PeerId) -> Option<&Arc<FaceState>> {
        self.faces.values().find(|face| face.pid == *pid)
    }

    fn open_net_face(
        &mut self,
        pid: PeerId,
        whatami: WhatAmI,
        primitives: Arc<dyn Primitives + Send + Sync>,
        link_id: usize,
    ) -> Weak<FaceState> {
        let fid = self.face_counter;
        self.face_counter += 1;
        let mut newface = self
            .faces
            .entry(fid)
            .or_insert_with(|| FaceState::new(fid, pid, whatami, primitives.clone(), link_id))
            .clone();
        log::debug!("New {}", newface);

        pubsub_new_face(self, &mut newface);
        queries_new_face(self, &mut newface);

        Arc::downgrade(&newface)
    }

    pub fn open_face(
        &mut self,
        pid: PeerId,
        whatami: WhatAmI,
        primitives: Arc<dyn Primitives + Send + Sync>,
    ) -> Weak<FaceState> {
        self.open_net_face(pid, whatami, primitives, 0)
    }

    pub fn close_face(&mut self, face: &Weak<FaceState>) {
        match face.upgrade() {
            Some(mut face) => {
                log::debug!("Close {}", face);
                finalize_pending_queries(self, &mut face);

                let mut face_clone = face.clone();
                let face = get_mut_unchecked(&mut face);
                for res in face.remote_mappings.values_mut() {
                    get_mut_unchecked(res).session_ctxs.remove(&face.id);
                    Resource::clean(res);
                }
                face.remote_mappings.clear();
                for res in face.local_mappings.values_mut() {
                    get_mut_unchecked(res).session_ctxs.remove(&face.id);
                    Resource::clean(res);
                }
                face.local_mappings.clear();
                for mut res in face.remote_subs.drain() {
                    get_mut_unchecked(&mut res).session_ctxs.remove(&face.id);
                    undeclare_client_subscription(self, &mut face_clone, &mut res);
                    Resource::clean(&mut res);
                }
                for (mut res, kind) in face.remote_qabls.drain() {
                    get_mut_unchecked(&mut res).session_ctxs.remove(&face.id);
                    undeclare_client_queryable(self, &mut face_clone, &mut res, kind);
                    Resource::clean(&mut res);
                }
                self.faces.remove(&face.id);
            }
            None => log::error!("Face already closed!"),
        }
    }

    fn compute_routes(&mut self, res: &mut Arc<Resource>) {
        compute_data_routes(self, res);
        compute_query_routes(self, res);
    }

    pub(crate) fn compute_matches_routes(&mut self, res: &mut Arc<Resource>) {
        if res.context.is_some() {
            self.compute_routes(res);

            let resclone = res.clone();
            for match_ in &mut get_mut_unchecked(res).context_mut().matches {
                let match_ = &mut match_.upgrade().unwrap();
                if !Arc::ptr_eq(match_, &resclone) && match_.context.is_some() {
                    self.compute_routes(match_);
                }
            }
        }
    }
}

pub struct Router {
    whatami: WhatAmI,
    pub tables: Arc<RwLock<Tables>>,
    tree_computation: Option<TreeComputation>,
}

impl Router {
    pub fn new(
        pid: PeerId,
        whatami: WhatAmI,
        hlc: Option<Arc<HLC>>,
        queries_default_timeout: Duration,
    ) -> Self {
        let tables = Arc::new(RwLock::new(Tables::new(
            pid,
            whatami,
            hlc,
            queries_default_timeout,
        )));
        let tree_computation = TreeComputation::new(tables.clone());

        Router {
            whatami,
            tables,
            tree_computation: Some(tree_computation),
        }
    }

    pub fn init_link_state(
        &mut self,
        runtime: Runtime,
        peers_autoconnect: bool,
        routers_autoconnect_gossip: bool,
    ) {
        let mut tables = zwrite!(self.tables);
        tables.peers_net = Some(Network::new(
            "[Peers network]".to_string(),
            tables.pid,
            runtime.clone(),
            peers_autoconnect,
            routers_autoconnect_gossip,
        ));
        if runtime.whatami == WhatAmI::Router {
            tables.routers_net = Some(Network::new(
                "[Routers network]".to_string(),
                tables.pid,
                runtime,
                peers_autoconnect,
                routers_autoconnect_gossip,
            ));
            tables.shared_nodes = shared_nodes(
                tables.routers_net.as_ref().unwrap(),
                tables.peers_net.as_ref().unwrap(),
            );
        }
    }

    pub fn new_primitives(&self, primitives: Arc<dyn Primitives + Send + Sync>) -> Arc<Face> {
        Arc::new(Face {
            tables: self.tables.clone(),
            state: {
                let mut tables = zwrite!(self.tables);
                let pid = tables.pid;
                tables
                    .open_face(pid, WhatAmI::Client, primitives)
                    .upgrade()
                    .unwrap()
            },
        })
    }

    pub fn new_transport_unicast(
        self: Arc<Self>,
        transport: TransportUnicast,
    ) -> ZResult<Arc<LinkStateInterceptor>> {
        let mut tables = zwrite!(self.tables);
        let whatami = transport.get_whatami()?;

        let link_id = match (self.whatami, whatami) {
            (WhatAmI::Router, WhatAmI::Router) => tables
                .routers_net
                .as_mut()
                .unwrap()
                .add_link(transport.clone()),
            (WhatAmI::Router, WhatAmI::Peer)
            | (WhatAmI::Peer, WhatAmI::Router)
            | (WhatAmI::Peer, WhatAmI::Peer) => tables
                .peers_net
                .as_mut()
                .unwrap()
                .add_link(transport.clone()),
            _ => 0,
        };

        if tables.whatami == WhatAmI::Router {
            tables.shared_nodes = shared_nodes(
                tables.routers_net.as_ref().unwrap(),
                tables.peers_net.as_ref().unwrap(),
            );
        }

        let handler = Arc::new(LinkStateInterceptor::new(
            transport.clone(),
            self.clone(),
            Face {
                tables: self.tables.clone(),
                state: tables
                    .open_net_face(
                        transport.get_pid().unwrap(),
                        whatami,
                        Arc::new(Mux::new(transport)),
                        link_id,
                    )
                    .upgrade()
                    .unwrap(),
            },
        ));

        match (self.whatami, whatami) {
            (WhatAmI::Router, WhatAmI::Router) => {
                self.schedule_compute_trees(WhatAmI::Router);
            }
            (WhatAmI::Router, WhatAmI::Peer)
            | (WhatAmI::Peer, WhatAmI::Router)
            | (WhatAmI::Peer, WhatAmI::Peer) => {
                self.schedule_compute_trees(WhatAmI::Peer);
            }
            _ => (),
        }
        Ok(handler)
    }

    pub fn schedule_compute_trees(&self, net_type: WhatAmI) {
        self.tree_computation
            .as_ref()
            .unwrap()
            .schedule_compute_trees(net_type);
    }

    pub fn schedule_link_state_update(&self, list: LinkStateList, pid: PeerId, whatami: WhatAmI) {
        self.tree_computation
            .as_ref()
            .unwrap()
            .schedule_link_state_update(list, pid, whatami);
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.tree_computation.take().unwrap().close();
    }
}

struct TreeComputation {
    pub link_state_task: async_std::task::JoinHandle<()>,
    pub router_task: JoinHandle<()>,
    pub peer_task: JoinHandle<()>,
    pub link_state_msg_tx: flume::Sender<LinkStateUpdateRequest>,
    pub router_req_tx: SyncSender<()>,
    pub peer_req_tx: SyncSender<()>,
}

impl TreeComputation {
    fn new(tables: Arc<RwLock<Tables>>) -> Self {
        let (link_state_msg_tx, link_state_msg_rx) = flume::bounded(16);
        let (router_req_tx, router_req_rx) = sync_channel(1);
        let (peer_req_tx, peer_req_rx) = sync_channel(1);

        let link_state_task = {
            let tables = tables.clone();
            let router_req_tx = router_req_tx.clone();
            let peer_req_tx = peer_req_tx.clone();

            async_std::task::spawn(async move {
                Self::update_link_state(tables, link_state_msg_rx, router_req_tx, peer_req_tx)
                    .await;
            })
        };
        let router_task = {
            let tables = tables.clone();

            thread::spawn(move || {
                Self::compute_router_trees(tables, router_req_rx);
            })
        };
        let peer_task = thread::spawn(move || {
            Self::compute_peer_trees(tables, peer_req_rx);
        });

        Self {
            router_task,
            peer_task,
            router_req_tx,
            peer_req_tx,
            link_state_task,
            link_state_msg_tx,
        }
    }

    fn schedule_compute_trees(&self, net_type: WhatAmI) {
        match net_type {
            WhatAmI::Router => {
                let _ = self.router_req_tx.try_send(());
            }
            WhatAmI::Peer => {
                let _ = self.peer_req_tx.try_send(());
            }
            WhatAmI::Client => unreachable!(),
        }
    }

    fn schedule_link_state_update(&self, list: LinkStateList, pid: PeerId, whatami: WhatAmI) {
        let req = LinkStateUpdateRequest {
            list: list.link_states,
            pid,
            whatami,
        };
        let _ = self.link_state_msg_tx.send(req);
    }

    fn close(self) {
        let Self {
            router_task,
            peer_task,
            link_state_task,
            router_req_tx: _,
            peer_req_tx: _,
            link_state_msg_tx: _,
        } = self;

        let _ = async_std::task::block_on(link_state_task);
        let _ = router_task.join();
        let _ = peer_task.join();
    }

    async fn update_link_state(
        tables_ref: Arc<RwLock<Tables>>,
        input_rx: flume::Receiver<LinkStateUpdateRequest>,
        router_req_tx: SyncSender<()>,
        peer_req_tx: SyncSender<()>,
    ) {
        use WhatAmI::*;

        let (batch_tx, batch_rx) = flume::bounded(16);

        let batcher = async move {
            'outer: loop {
                let orig_reqs: Vec<_> = input_rx
                    .stream()
                    .take_until(async move {
                        async_std::task::sleep(Duration::from_millis(100)).await;
                    })
                    .collect()
                    .await;

                let merged_reqs = orig_reqs
                    .into_iter()
                    .map(|req| {
                        let LinkStateUpdateRequest { list, pid, whatami } = req;
                        ((pid, whatami), list)
                    })
                    .into_group_map()
                    .into_iter()
                    .map(|((pid, whatami), lists)| {
                        let states = lists.into_iter().flatten();
                        let states = merge_link_state_list(states);

                        LinkStateUpdateRequest {
                            list: states,
                            pid,
                            whatami,
                        }
                    });

                for req in merged_reqs {
                    let ok = batch_tx.send_async(req).await.is_ok();
                    if !ok {
                        break 'outer;
                    }
                }
            }
        };

        let updater = spawn_blocking(move || {
            for req in batch_rx.into_iter() {
                let LinkStateUpdateRequest { list, pid, whatami } = req;

                let mut tables = zwrite!(tables_ref);
                match (tables.whatami, whatami) {
                    (Router, Router) => {
                        let removed_nodes =
                            tables.routers_net.as_mut().unwrap().link_states(list, pid);

                        for (_, removed_node) in removed_nodes {
                            pubsub_remove_node(&mut tables, &removed_node.pid, Router);
                            queries_remove_node(&mut tables, &removed_node.pid, Router);
                        }

                        tables.shared_nodes = shared_nodes(
                            tables.routers_net.as_ref().unwrap(),
                            tables.peers_net.as_ref().unwrap(),
                        );

                        if router_req_tx.try_send(()).is_err() {
                            break;
                        }
                    }
                    (Router, Peer) | (Peer, Router | Peer) => {
                        let removed_nodes =
                            tables.peers_net.as_mut().unwrap().link_states(list, pid);

                        for (_, removed_node) in removed_nodes {
                            pubsub_remove_node(&mut tables, &removed_node.pid, Peer);
                            queries_remove_node(&mut tables, &removed_node.pid, Peer);
                        }

                        if tables.whatami == Router {
                            tables.shared_nodes = shared_nodes(
                                tables.routers_net.as_ref().unwrap(),
                                tables.peers_net.as_ref().unwrap(),
                            );
                        }

                        if peer_req_tx.try_send(()).is_err() {
                            break;
                        }
                    }
                    _ => (),
                };
            }
        });

        join!(batcher, updater);
    }

    fn compute_router_trees(tables_ref: Arc<RwLock<Tables>>, rx: Receiver<()>) {
        while let Ok(()) = rx.recv() {
            let mut tables = zwrite!(tables_ref);

            log::trace!("Compute trees");
            let new_childs = tables.routers_net.as_mut().unwrap().compute_trees();

            log::trace!("Compute routes");
            pubsub_tree_change(&mut tables, &new_childs, WhatAmI::Router);
            queries_tree_change(&mut tables, &new_childs, WhatAmI::Router);

            log::trace!("Computations completed");
            thread::sleep(Duration::from_millis(*TREES_COMPUTATION_DELAY));
        }
    }

    fn compute_peer_trees(tables_ref: Arc<RwLock<Tables>>, rx: Receiver<()>) {
        while let Ok(()) = rx.recv() {
            let mut tables = zwrite!(tables_ref);

            log::trace!("Compute trees");
            let new_childs = tables.peers_net.as_mut().unwrap().compute_trees();

            log::trace!("Compute routes");
            pubsub_tree_change(&mut tables, &new_childs, WhatAmI::Peer);
            queries_tree_change(&mut tables, &new_childs, WhatAmI::Peer);

            log::trace!("Computations completed");
            thread::sleep(Duration::from_millis(*TREES_COMPUTATION_DELAY));
        }
    }
}

struct LinkStateUpdateRequest {
    pub list: Vec<LinkState>,
    pub pid: PeerId,
    pub whatami: WhatAmI,
}

pub struct LinkStateInterceptor {
    pub(crate) transport: TransportUnicast,
    pub(crate) router: Arc<Router>,
    pub(crate) face: Face,
    pub(crate) demux: DeMux<Face>,
}

impl LinkStateInterceptor {
    fn new(transport: TransportUnicast, router: Arc<Router>, face: Face) -> Self {
        LinkStateInterceptor {
            transport,
            router,
            face: face.clone(),
            demux: DeMux::new(face),
        }
    }
}

impl TransportPeerEventHandler for LinkStateInterceptor {
    fn handle_message(&self, msg: ZenohMessage) -> ZResult<()> {
        log::trace!("Recv {:?}", msg);
        match msg.body {
            ZenohBody::LinkStateList(list) => {
                let pid = self.transport.get_pid()?;
                let whatami = self.transport.get_whatami()?;
                self.router.schedule_link_state_update(list, pid, whatami);
                Ok(())
            }
            _ => self.demux.handle_message(msg),
        }
    }

    fn new_link(&self, _link: Link) {}

    fn del_link(&self, _link: Link) {}

    fn closing(&self) {
        self.demux.closing();
        let tables_ref = self.router.tables.clone();
        match (self.transport.get_pid(), self.transport.get_whatami()) {
            (Ok(pid), Ok(whatami)) => {
                let mut tables = zwrite!(tables_ref);
                match (tables.whatami, whatami) {
                    (WhatAmI::Router, WhatAmI::Router) => {
                        for (_, removed_node) in
                            tables.routers_net.as_mut().unwrap().remove_link(&pid)
                        {
                            pubsub_remove_node(&mut tables, &removed_node.pid, WhatAmI::Router);
                            queries_remove_node(&mut tables, &removed_node.pid, WhatAmI::Router);
                        }

                        tables.shared_nodes = shared_nodes(
                            tables.routers_net.as_ref().unwrap(),
                            tables.peers_net.as_ref().unwrap(),
                        );

                        self.router.schedule_compute_trees(WhatAmI::Router);
                    }
                    (WhatAmI::Router, WhatAmI::Peer)
                    | (WhatAmI::Peer, WhatAmI::Router)
                    | (WhatAmI::Peer, WhatAmI::Peer) => {
                        for (_, removed_node) in
                            tables.peers_net.as_mut().unwrap().remove_link(&pid)
                        {
                            pubsub_remove_node(&mut tables, &removed_node.pid, WhatAmI::Peer);
                            queries_remove_node(&mut tables, &removed_node.pid, WhatAmI::Peer);
                        }

                        if tables.whatami == WhatAmI::Router {
                            tables.shared_nodes = shared_nodes(
                                tables.routers_net.as_ref().unwrap(),
                                tables.peers_net.as_ref().unwrap(),
                            );
                        }

                        self.router.schedule_compute_trees(WhatAmI::Peer);
                    }
                    _ => (),
                };
            }
            (_, _) => log::error!("Closed transport in session closing!"),
        }
    }

    fn closed(&self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
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

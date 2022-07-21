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
use super::worker::{AddLink, LinkStateUpdate, RemoveLink, UpdateRequest, UpdateTask};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::sync::{Mutex, RwLock};
use std::time::Duration;
use uhlc::HLC;
use zenoh_core::zconfigurable;
use zenoh_core::Result as ZResult;
use zenoh_link::Link;
use zenoh_protocol::proto::{LinkStateList, ZenohBody, ZenohMessage};
use zenoh_protocol_core::{PeerId, WhatAmI, ZInt};
use zenoh_sync::get_mut_unchecked;
use zenoh_transport::{DeMux, Mux, Primitives, TransportPeerEventHandler, TransportUnicast};

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
    pub tables: Arc<RwLock<Tables>>,
    update_task: Option<UpdateTask>,
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
        let update_task = UpdateTask::new(tables.clone());

        Router {
            tables,
            update_task: Some(update_task),
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
        let whatami = transport.get_whatami()?;
        let link_id = self.request_add_link(transport.clone());

        let state = {
            let mut tables = zwrite!(self.tables);
            tables
                .open_net_face(
                    transport.get_pid().unwrap(),
                    whatami,
                    Arc::new(Mux::new(transport.clone())),
                    link_id,
                )
                .upgrade()
                .unwrap()
        };
        let handler = Arc::new(LinkStateInterceptor::new(
            transport,
            self.clone(),
            Face {
                tables: self.tables.clone(),
                state,
            },
        ));

        Ok(handler)
    }

    fn request_update<I>(&self, req: I) -> bool
    where
        I: Into<UpdateRequest>,
    {
        self.update_task.as_ref().unwrap().push(req)
    }

    fn request_link_state_update(&self, list: LinkStateList, pid: PeerId, whatami: WhatAmI) {
        self.request_update(LinkStateUpdate {
            list: list.link_states,
            pid,
            whatami,
        });
    }

    fn request_remove_link(&self, transport: TransportUnicast) {
        self.request_update(RemoveLink { transport });
    }

    fn request_add_link(&self, transport: TransportUnicast) -> usize {
        let (link_id_tx, link_id_rx) = flume::bounded(1);
        self.request_update(AddLink {
            transport,
            link_id_tx,
        });

        match link_id_rx.recv() {
            Ok(link_id) => link_id,
            Err(_) => unreachable!("internal error"),
        }
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.update_task.take().unwrap().join();
    }
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
                self.router.request_link_state_update(list, pid, whatami);
                Ok(())
            }
            _ => self.demux.handle_message(msg),
        }
    }

    fn new_link(&self, _link: Link) {}

    fn del_link(&self, _link: Link) {}

    fn closing(&self) {
        self.demux.closing();
        self.router.request_remove_link(self.transport.clone());
    }

    fn closed(&self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
}

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
use super::runtime::Runtime;
use petgraph::graph::NodeIndex;
use petgraph::visit::{IntoNodeReferences, VisitMap, Visitable};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fmt;
use std::hash::Hasher;
use vec_map::VecMap;
use zenoh_link::Locator;
use zenoh_protocol::core::{PeerId, WhatAmI, ZInt};
use zenoh_protocol::proto::{LinkState, ZenohMessage};
use zenoh_transport::TransportUnicast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub(crate) struct LinkId(usize);

impl LinkId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }
}

impl fmt::Display for LinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub(crate) struct Node {
    pub(crate) pid: PeerId,
    pub(crate) whatami: Option<WhatAmI>,
    pub(crate) locators: Option<Vec<Locator>>,
    pub(crate) sn: ZInt,
    pub(crate) links: HashSet<PeerId>,
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.pid)
    }
}

pub(crate) struct Link {
    pub(crate) transport: TransportUnicast,
    pid: PeerId,
    mappings: VecMap<PeerId>,
    local_mappings: VecMap<ZInt>,
}

impl Link {
    fn new(transport: TransportUnicast) -> Self {
        let pid = transport.get_pid().unwrap();
        Link {
            transport,
            pid,
            mappings: VecMap::new(),
            local_mappings: VecMap::new(),
        }
    }

    #[inline]
    pub(crate) fn set_pid_mapping(&mut self, psid: ZInt, pid: PeerId) {
        self.mappings.insert(psid as usize, pid);
    }

    #[inline]
    pub(crate) fn get_pid(&self, psid: &ZInt) -> Option<&PeerId> {
        self.mappings.get((*psid) as usize)
    }

    #[inline]
    pub(crate) fn set_local_psid_mapping(&mut self, psid: ZInt, local_psid: ZInt) {
        self.local_mappings.insert(psid as usize, local_psid);
    }

    #[inline]
    pub(crate) fn get_local_psid(&self, psid: &ZInt) -> Option<&ZInt> {
        self.local_mappings.get((*psid) as usize)
    }
}

pub(crate) struct Tree {
    pub(crate) parent: Option<NodeIndex>,
    pub(crate) childs: Vec<NodeIndex>,
    pub(crate) directions: Vec<Option<NodeIndex>>,
}

pub(crate) struct Network {
    pub(crate) name: String,
    pub(crate) peers_autoconnect: bool,
    pub(crate) routers_autoconnect_gossip: bool,
    pub(crate) idx: NodeIndex,
    pub(crate) links: VecMap<Link>,
    pub(crate) trees: Vec<Tree>,
    pub(crate) distances: Vec<f64>,
    pub(crate) graph: petgraph::stable_graph::StableUnGraph<Node, f64>,
    pub(crate) runtime: Runtime,
}

impl Network {
    pub(crate) fn new(
        name: String,
        pid: PeerId,
        runtime: Runtime,
        peers_autoconnect: bool,
        routers_autoconnect_gossip: bool,
    ) -> Self {
        let mut graph = petgraph::stable_graph::StableGraph::default();
        log::debug!("{} Add node (self) {}", name, pid);
        let idx = graph.add_node(Node {
            pid,
            whatami: Some(runtime.whatami),
            locators: None,
            sn: 1,
            links: HashSet::new(),
        });
        Network {
            name,
            peers_autoconnect,
            routers_autoconnect_gossip,
            idx,
            links: VecMap::new(),
            trees: vec![Tree {
                parent: None,
                childs: vec![],
                directions: vec![None],
            }],
            distances: vec![0.0],
            graph,
            runtime,
        }
    }

    pub(crate) fn dot(&self) -> String {
        format!(
            "{:?}",
            petgraph::dot::Dot::with_config(&self.graph, &[petgraph::dot::Config::EdgeNoLabel])
        )
    }

    #[inline]
    pub(crate) fn get_idx(&self, pid: &PeerId) -> Option<NodeIndex> {
        self.graph
            .node_references()
            .find_map(|(idx, node)| (node.pid == *pid).then(|| idx))
    }

    #[inline]
    pub(crate) fn get_link(&self, id: LinkId) -> Option<&Link> {
        self.links.get(id.0)
    }

    #[inline]
    pub(crate) fn get_link_from_pid(&self, pid: &PeerId) -> Option<&Link> {
        self.links.values().find(|link| link.pid == *pid)
    }

    #[inline]
    pub(crate) fn get_local_context(&self, context: Option<ZInt>, link_id: LinkId) -> NodeIndex {
        let context = context.unwrap_or(0);
        match self.get_link(link_id) {
            Some(link) => match link.get_local_psid(&context) {
                Some(&psid) => NodeIndex::new(psid as usize),
                None => {
                    log::error!(
                        "Cannot find local psid for context {} on link {}",
                        context,
                        link_id
                    );
                    NodeIndex::new(0)
                }
            },
            None => {
                log::error!("Cannot find link {}", link_id);
                NodeIndex::new(0)
            }
        }
    }

    #[inline]
    fn get_locators(&self) -> Vec<Locator> {
        self.runtime.manager().get_locators()
    }

    fn add_node(&mut self, node: Node) -> NodeIndex {
        let pid = node.pid;
        let idx = self.graph.add_node(node);
        self.links
            .values_mut()
            .filter_map(|link| {
                let (psid, _) = link.mappings.iter().find(|&(_, &p)| p == pid)?;
                Some((link, psid))
            })
            .for_each(|(link, psid)| {
                link.local_mappings.insert(psid, idx.index() as ZInt);
            });
        idx
    }

    fn make_link_state(&self, idx: NodeIndex, details: bool) -> LinkState {
        let links = self.graph[idx]
            .links
            .iter()
            .filter_map(|pid| {
                if let Some(idx2) = self.get_idx(pid) {
                    Some(idx2.index() as ZInt)
                } else {
                    log::error!(
                        "{} Internal error building link state: cannot get index of {}",
                        self.name,
                        pid
                    );
                    None
                }
            })
            .collect();
        LinkState {
            psid: idx.index() as ZInt,
            sn: self.graph[idx].sn,
            pid: if details {
                Some(self.graph[idx].pid)
            } else {
                None
            },
            whatami: self.graph[idx].whatami,
            locators: if idx == self.idx {
                Some(self.get_locators())
            } else {
                self.graph[idx].locators.clone()
            },
            links,
        }
    }

    fn make_msg(&self, idxs: Vec<(NodeIndex, bool)>) -> ZenohMessage {
        let mut list = vec![];
        for (idx, details) in idxs {
            list.push(self.make_link_state(idx, details));
        }
        ZenohMessage::make_link_state_list(list, None)
    }

    fn send_on_link(&self, idxs: Vec<(NodeIndex, bool)>, transport: &TransportUnicast) {
        let msg = self.make_msg(idxs);
        log::trace!("{} Send to {:?} {:?}", self.name, transport.get_pid(), msg);
        if let Err(e) = transport.handle_message(msg) {
            log::debug!("{} Error sending LinkStateList: {}", self.name, e);
        }
    }

    fn send_on_links<P>(&self, idxs: Vec<(NodeIndex, bool)>, mut value_selector: P)
    where
        P: FnMut(&Link) -> bool,
    {
        let msg = self.make_msg(idxs);
        for link in self.links.values() {
            if value_selector(link) {
                log::trace!("{} Send to {} {:?}", self.name, link.pid, msg);
                if let Err(e) = link.transport.handle_message(msg.clone()) {
                    log::debug!("{} Error sending LinkStateList: {}", self.name, e);
                }
            }
        }
    }

    fn update_edge(&mut self, idx1: NodeIndex, idx2: NodeIndex) {
        let mut hasher = DefaultHasher::default();
        let slice1 = self.graph[idx1].pid.as_slice();
        let slice2 = self.graph[idx2].pid.as_slice();

        let (min_slice, max_slice) = if slice1 <= slice2 {
            (slice1, slice2)
        } else {
            (slice2, slice1)
        };

        hasher.write(min_slice);
        hasher.write(max_slice);

        let weight = 100.0 + ((hasher.finish() as u32) as f64) / u32::MAX as f64;
        self.graph.update_edge(idx1, idx2, weight);
    }

    pub(crate) fn link_states(
        &mut self,
        link_states: Vec<LinkState>,
        src: PeerId,
    ) -> Vec<(NodeIndex, Node)> {
        log::trace!("{} Received from {} raw: {:?}", self.name, src, link_states);

        let graph = &self.graph;
        let links = &mut self.links;

        let src_link = match links.values_mut().find(|link| link.pid == src) {
            Some(link) => link,
            None => {
                log::error!(
                    "{} Received LinkStateList from unknown link {}",
                    self.name,
                    src
                );
                return vec![];
            }
        };

        // register psid<->pid mappings & apply mapping to nodes
        #[allow(clippy::needless_collect)] // need to release borrow on self
        let link_states: Vec<_> = link_states
            .into_iter()
            .filter_map(|link_state| {
                if let Some(pid) = link_state.pid {
                    src_link.set_pid_mapping(link_state.psid, pid);
                    if let Some(idx) = graph.node_indices().find(|idx| graph[*idx].pid == pid) {
                        src_link.set_local_psid_mapping(link_state.psid, idx.index() as u64);
                    }
                    Some((
                        pid,
                        link_state.whatami.unwrap_or(WhatAmI::Router),
                        link_state.locators,
                        link_state.sn,
                        link_state.links,
                    ))
                } else {
                    match src_link.get_pid(&link_state.psid) {
                        Some(pid) => Some((
                            *pid,
                            link_state.whatami.unwrap_or(WhatAmI::Router),
                            link_state.locators,
                            link_state.sn,
                            link_state.links,
                        )),
                        None => {
                            log::error!(
                                "Received LinkState from {} with unknown node mapping {}",
                                src,
                                link_state.psid
                            );
                            None
                        }
                    }
                }
            })
            .collect();

        // apply psid<->pid mapping to links
        let src_link = self.get_link_from_pid(&src).unwrap();
        let link_states: Vec<_> = link_states
            .into_iter()
            .map(|(pid, wai, locs, sn, links)| {
                let links: Vec<PeerId> = links
                    .iter()
                    .filter_map(|l| {
                        if let Some(pid) = src_link.get_pid(l) {
                            Some(*pid)
                        } else {
                            log::error!(
                                "{} Received LinkState from {} with unknown link mapping {}",
                                self.name,
                                src,
                                l
                            );
                            None
                        }
                    })
                    .collect();
                (pid, wai, locs, sn, links)
            })
            .collect();

        // log::trace!(
        //     "{} Received from {} mapped: {:?}",
        //     self.name,
        //     src,
        //     link_states
        // );
        for link_state in &link_states {
            log::trace!(
                "{} Received from {} mapped: {:?}",
                self.name,
                src,
                link_state
            );
        }

        // Add nodes to graph & filter out up to date states
        let mut link_states: Vec<(Vec<PeerId>, NodeIndex, bool)> = link_states
            .into_iter()
            .filter_map(
                |(pid, whatami, locators, sn, links)| match self.get_idx(&pid) {
                    Some(idx) => {
                        let node = &mut self.graph[idx];
                        let oldsn = node.sn;
                        if oldsn < sn {
                            node.sn = sn;
                            node.links = links.iter().cloned().collect();
                            if locators.is_some() {
                                node.locators = locators;
                            }
                            if oldsn == 0 {
                                Some((links, idx, true))
                            } else {
                                Some((links, idx, false))
                            }
                        } else {
                            None
                        }
                    }
                    None => {
                        let node = Node {
                            pid,
                            whatami: Some(whatami),
                            locators,
                            sn,
                            links: links.iter().cloned().collect(),
                        };
                        log::debug!("{} Add node (state) {}", self.name, pid);
                        let idx = self.add_node(node);
                        Some((links, idx, true))
                    }
                },
            )
            .collect();

        // Add/remove edges from graph
        let mut reintroduced_nodes = vec![];
        for (links, idx1, _) in &link_states {
            for link in links {
                if let Some(idx2) = self.get_idx(link) {
                    if self.graph[idx2].links.contains(&self.graph[*idx1].pid) {
                        log::trace!(
                            "{} Update edge (state) {} {}",
                            self.name,
                            self.graph[*idx1].pid,
                            self.graph[idx2].pid
                        );
                        self.update_edge(*idx1, idx2);
                    }
                } else {
                    let node = Node {
                        pid: *link,
                        whatami: None,
                        locators: None,
                        sn: 0,
                        links: HashSet::new(),
                    };
                    log::debug!("{} Add node (reintroduced) {}", self.name, link.clone());
                    let idx = self.add_node(node);
                    reintroduced_nodes.push((vec![], idx, true));
                }
            }
            let mut edges = vec![];
            let mut neighbors = self.graph.neighbors_undirected(*idx1).detach();
            while let Some(edge) = neighbors.next(&self.graph) {
                edges.push(edge);
            }
            for (eidx, idx2) in edges {
                if !links.contains(&self.graph[idx2].pid) {
                    log::trace!(
                        "{} Remove edge (state) {} {}",
                        self.name,
                        self.graph[*idx1].pid,
                        self.graph[idx2].pid
                    );
                    self.graph.remove_edge(eidx);
                }
            }
        }
        link_states.extend(reintroduced_nodes);

        let removed = self.remove_detached_nodes();
        let link_states: Vec<(Vec<PeerId>, NodeIndex, bool)> = link_states
            .into_iter()
            .filter(|ls| !removed.iter().any(|(idx, _)| idx == &ls.1))
            .collect();

        if (self.peers_autoconnect && self.runtime.whatami == WhatAmI::Peer)
            || (self.routers_autoconnect_gossip && self.runtime.whatami == WhatAmI::Router)
        {
            // Connect discovered peers
            for (_, idx, _) in &link_states {
                let node = &self.graph[*idx];
                if (self.runtime.whatami == WhatAmI::Peer
                    && (node.whatami == Some(WhatAmI::Peer)
                        || node.whatami == Some(WhatAmI::Router)))
                    || (self.runtime.whatami == WhatAmI::Router
                        && node.whatami == Some(WhatAmI::Router))
                {
                    if let Some(locators) = &node.locators {
                        let runtime = self.runtime.clone();
                        let pid = node.pid;
                        let locators = locators.clone();
                        self.runtime.spawn(async move {
                            // random backoff
                            async_std::task::sleep(std::time::Duration::from_millis(
                                rand::random::<u64>() % 100,
                            ))
                            .await;
                            runtime.connect_peer(&pid, &locators).await;
                        });
                    }
                }
            }
        }

        // Propagate link states
        // Note: we need to send all states at once for each face
        // to avoid premature node deletion on the other side
        #[allow(clippy::type_complexity)]
        if !link_states.is_empty() {
            let (new_idxs, updated_idxs): (
                Vec<(Vec<PeerId>, NodeIndex, bool)>,
                Vec<(Vec<PeerId>, NodeIndex, bool)>,
            ) = link_states.into_iter().partition(|(_, _, new)| *new);
            let new_idxs: Vec<(NodeIndex, bool)> = new_idxs
                .into_iter()
                .map(|(_, idx1, _new_node)| (idx1, true))
                .collect();
            for link in self.links.values() {
                if link.pid != src {
                    let updated_idxs: Vec<(NodeIndex, bool)> = updated_idxs
                        .clone()
                        .into_iter()
                        .filter_map(|(_, idx1, _)| {
                            if link.pid != self.graph[idx1].pid {
                                Some((idx1, false))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !new_idxs.is_empty() || !updated_idxs.is_empty() {
                        self.send_on_link(
                            [&new_idxs[..], &updated_idxs[..]].concat(),
                            &link.transport,
                        );
                    }
                } else if !new_idxs.is_empty() {
                    self.send_on_link(new_idxs.clone(), &link.transport);
                }
            }
        }
        removed
    }

    pub(crate) fn add_link(&mut self, transport: TransportUnicast) -> LinkId {
        let free_index = (0..).find(|&idx| !self.links.contains_key(idx)).unwrap();
        self.links.insert(free_index, Link::new(transport.clone()));

        let pid = transport.get_pid().unwrap();
        let whatami = transport.get_whatami().unwrap();
        let (idx, new) = match self.get_idx(&pid) {
            Some(idx) => (idx, false),
            None => {
                log::debug!("{} Add node (link) {}", self.name, pid);
                (
                    self.add_node(Node {
                        pid,
                        whatami: Some(whatami),
                        locators: None,
                        sn: 0,
                        links: HashSet::new(),
                    }),
                    true,
                )
            }
        };

        let my_node = &self.graph[self.idx];
        let remote_node = &self.graph[idx];

        if remote_node.links.contains(&my_node.pid) {
            log::trace!("Update edge (link) {} {}", my_node.pid, pid);
            self.update_edge(self.idx, idx);
        }
        self.graph[self.idx].links.insert(pid);
        self.graph[self.idx].sn += 1;

        if new {
            self.send_on_links(vec![(idx, true), (self.idx, false)], |link| link.pid != pid);
        } else {
            self.send_on_links(vec![(self.idx, false)], |link| link.pid != pid);
        }

        let idxs: Vec<_> = self.graph.node_indices().map(|i| (i, true)).collect();
        self.send_on_link(idxs, &transport);
        LinkId::new(free_index)
    }

    pub(crate) fn remove_link(&mut self, pid: &PeerId) -> Vec<(NodeIndex, Node)> {
        log::trace!("{} remove_link {}", self.name, pid);
        self.links.retain(|_, link| link.pid != *pid);
        self.graph[self.idx].links.retain(|link| *link != *pid);

        if let Some((edge, _)) = self
            .get_idx(pid)
            .and_then(|idx| self.graph.find_edge_undirected(self.idx, idx))
        {
            self.graph.remove_edge(edge);
        }
        let removed = self.remove_detached_nodes();

        self.graph[self.idx].sn += 1;

        let links: Vec<ZInt> = self
            .links
            .values()
            .map(|link| self.get_idx(&link.pid).unwrap().index() as ZInt)
            .collect();

        let msg = ZenohMessage::make_link_state_list(
            vec![LinkState {
                psid: self.idx.index() as ZInt,
                sn: self.graph[self.idx].sn,
                pid: None,
                whatami: self.graph[self.idx].whatami,
                locators: Some(self.get_locators()),
                links,
            }],
            None,
        );

        for link in self.links.values() {
            if let Err(e) = link.transport.handle_message(msg.clone()) {
                log::debug!("{} Error sending LinkStateList: {}", self.name, e);
            }
        }

        removed
    }

    fn remove_detached_nodes(&mut self) -> Vec<(NodeIndex, Node)> {
        let mut dfs_stack = vec![self.idx];
        let mut visit_map = self.graph.visit_map();
        while let Some(node) = dfs_stack.pop() {
            if visit_map.visit(node) {
                for succpid in &self.graph[node].links {
                    if let Some(succ) = self.get_idx(succpid) {
                        if !visit_map.is_visited(&succ) {
                            dfs_stack.push(succ);
                        }
                    }
                }
            }
        }

        let mut removed = vec![];
        let indices: Vec<NodeIndex> = self.graph.node_indices().collect();

        for idx in indices {
            if !visit_map.is_visited(&idx) {
                log::debug!("Remove node {}", &self.graph[idx].pid);
                removed.push((idx, self.graph.remove_node(idx).unwrap()));
            }
        }
        removed
    }

    pub(crate) fn compute_trees(&mut self) -> Vec<Vec<NodeIndex>> {
        /* This algorithm reconstructs self.trees. The self.trees is a
         * (sid -> Tree) map, where sid is exactly an index of a node
         * in the graph. */

        // Save the node indices from the graph
        let indexes: Vec<NodeIndex> = self.graph.node_indices().collect();
        let num_trees = indexes.iter().max().unwrap().index() + 1;

        // Clean up self.trees and save the "childs" of each tree from self.trees
        let old_childs: Vec<Vec<NodeIndex>> = self.trees.drain(..).map(|t| t.childs).collect();

        /* Populate self.trees with empty Tree contexts for each sid, A Tree
         * context stores the following fields:
         *
         * - parent: the ingress neightbor node to myself, if coming from the sid.
         * - childs: the exgress neightbor nodes from myself, if coming from the sid.
         * - directions: a (dest_node -> neightbor_node) map to look up the outbound
         *               neightbor to go for a destination node.
         */
        self.trees.resize_with(num_trees, || Tree {
            parent: None,
            childs: Vec::with_capacity(num_trees),
            directions: Vec::with_capacity(num_trees),
        });

        // Loop through available indices to constructhe the trees one
        // by one.
        for &tree_root_idx in &indexes {
            // Run Bellman-Ford, starting at the interested node,
            // which is called tree_root_index here.
            let paths = petgraph::algo::bellman_ford(&self.graph, tree_root_idx).unwrap();

            // If the current node index is zero, it's a special case the
            // starting node is myself. Store the distance table by the way.
            if tree_root_idx.index() == 0 {
                self.distances = paths.distances;
            }

            // Debug message
            if log::log_enabled!(log::Level::Debug) {
                let ps: Vec<Option<String>> = paths
                    .predecessors
                    .iter()
                    .enumerate()
                    .map(|(is, o)| {
                        o.map(|ip| {
                            format!(
                                "{} <- {}",
                                self.graph[ip].pid,
                                self.graph[NodeIndex::new(is)].pid
                            )
                        })
                    })
                    .collect();
                log::debug!("Tree {} {:?}", self.graph[tree_root_idx].pid, ps);
            }

            // Save the ingress neighbor to myself according to Bellman-Ford.
            let root_tree = &mut self.trees[tree_root_idx.index()];
            root_tree.parent = paths.predecessors[self.idx.index()];

            // Save the exgress neighbors from myself according to Bellman-Ford.
            //
            // It uses an inefficient algorithm to scan all nodes in the graph,
            // and check if each node is an outbound node from myself.
            for &idx in &indexes {
                if let Some(parent_idx) = paths.predecessors[idx.index()] {
                    if parent_idx == self.idx {
                        root_tree.childs.push(idx);
                    }
                }
            }

            // Populate initial values to the "directions" field.
            root_tree.directions.resize_with(num_trees, || None);

            // The DFS space is used to check if two nodes are connected.
            let mut dfs = petgraph::algo::DfsSpace::new(&self.graph);

            // Loop over available node indices, each treated as the
            // destination node from myself in each loop.
            for &destination in &indexes {
                // Skip the case when the destination is myself.
                if self.idx == destination {
                    continue;
                }

                // Check if there is a path from myself to the destination.
                // If not, skip this loop.
                let is_connected = petgraph::algo::has_path_connecting(
                    &self.graph,
                    self.idx,
                    destination,
                    Some(&mut dfs),
                );

                if !is_connected {
                    continue;
                }

                // Traverse from the destination to myself using the
                // "predecessors" table from Bellman-Ford. Eventually,
                // it will find the outbound neighbor of myself to go
                // to the destination.
                let mut direction = None;
                let mut current = destination;
                while let Some(parent) = paths.predecessors[current.index()] {
                    if parent == self.idx {
                        direction = Some(current);
                        break;
                    } else {
                        current = parent;
                    }
                }

                // If the outbound neighbor is found, save it. If not,
                // set the outbound neighbor to the ingress neighbor, working as a "rejection".
                root_tree.directions[destination.index()] = match direction {
                    Some(direction) => Some(direction),
                    None => root_tree.parent,
                };
            }
        }

        let new_childs = {
            let old_part = old_childs.iter().enumerate().map(|(i, old_child)| {
                self.trees[i]
                    .childs
                    .iter()
                    .filter(|idx| !old_child.contains(idx))
                    .cloned()
                    .collect()
            });
            let new_part = (old_childs.len()..num_trees).map(|i| self.trees[i].childs.clone());

            old_part.chain(new_part).collect()
        };
        new_childs
    }

    #[inline]
    pub(super) fn shared_nodes(&self, other: &Network) -> Vec<PeerId> {
        let pid_set1: HashSet<_> = self.graph.node_weights().map(|node| node.pid).collect();
        let pid_set2: HashSet<_> = other.graph.node_weights().map(|node| node.pid).collect();
        let common_pids: Vec<_> = pid_set1.intersection(&pid_set2).cloned().collect();
        common_pids
    }
}

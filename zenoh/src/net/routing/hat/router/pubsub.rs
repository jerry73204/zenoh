//
// Copyright (c) 2023 ZettaScale Technology
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
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs,
    str::FromStr,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use petgraph::graph::NodeIndex;
use zenoh_protocol::{
    core::{WhatAmI, ZenohIdProto, key_expr::OwnedKeyExpr},
    network::{
        declare::{
            Declare, DeclareBody, DeclarePreSubscriber, DeclareRouteUpdate, DeclareSubscriber, SubscriberId, SyncInfo, UndeclareSubscriber, common::ext::WireExprType, ext
        },
        interest::{InterestId, InterestMode},
    },
};
use zenoh_sync::get_mut_unchecked;

use super::{
    face_hat, face_hat_mut, get_peer, get_router, get_router_id, get_routes_entries, hat, hat_mut,
    interests::push_declaration_profile,
    network::{Network, Tree},
    res_hat, res_hat_mut, HatCode, HatContext, HatFace, HatTables,
};
#[cfg(feature = "unstable")]
use crate::key_expr::KeyExpr;
use crate::net::routing::{
    RoutingContext, dispatcher::{
        face::FaceState,
        interests::RemoteInterest,
        pubsub::{SubscriberInfo, update_data_routes_from, update_matches_data_routes},
        resource::{NodeId, Resource, SessionContext},
        tables::{Route, RoutingExpr, Tables},
    }, hat::{CurrentFutureTrait, HatPubSubTrait, SendDeclare, Sources}, router::{RoutesIndexes}
};

#[inline]
fn send_sourced_subscription_to_net_children(
    tables: &Tables,
    net: &Network,
    children: &[NodeIndex],
    res: &Arc<Resource>,
    src_face: Option<&Arc<FaceState>>,
    _sub_info: &SubscriberInfo,
    routing_context: NodeId,
) {
    for child in children {
        if net.graph.contains_node(*child) {
            match tables.get_face(&net.graph[*child].zid).cloned() {
                Some(mut someface) => {
                    if src_face
                        .map(|src_face| someface.id != src_face.id)
                        .unwrap_or(true)
                    {
                        let push_declaration = push_declaration_profile(tables, &someface);
                        let key_expr = Resource::decl_key(res, &mut someface, push_declaration);

                        someface.primitives.send_declare(RoutingContext::with_expr(
                            Declare {
                                interest_id: None,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType {
                                    node_id: routing_context,
                                },
                                body: DeclareBody::DeclareSubscriber(DeclareSubscriber {
                                    id: 0, // Sourced subscriptions do not use ids
                                    wire_expr: key_expr,
                                }),
                            },
                            res.expr(),
                        ));
                    }
                }
                None => tracing::trace!("Unable to find face for zid {}", net.graph[*child].zid),
            }
        }
    }
}

#[inline]
fn send_presubscription_to_target_direction(
    tables: &Tables,
    net: &Network,
    tree: &Tree,
    target_router_id: NodeId,
    sync_info: SyncInfo,
    estimated_time: Duration,
    id: SubscriberId,
    res: &Arc<Resource>,
    src_face: Option<&Arc<FaceState>>,
    _sub_info: &SubscriberInfo,
    routing_context: NodeId,
) {
    if let Some(direction) = tree.directions[target_router_id as usize] {
        if net.graph.contains_node(direction) {
            match tables.get_face(&net.graph[direction].zid).cloned() {
                Some(mut someface) => {
                    if src_face
                        .map(|src_face| someface.id != src_face.id)
                        .unwrap_or(true)
                    {
                        let push_declaration = push_declaration_profile(tables, &someface);
                        let key_expr = Resource::decl_key(res, &mut someface, push_declaration);
                        tracing::trace!("send_presubscription_to_target_direction {}",someface.zid);
                        someface.primitives.send_declare(RoutingContext::with_expr(
                            Declare {
                                interest_id: None,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType {
                                    node_id: routing_context,
                                },
                                body: DeclareBody::DeclarePreSubscriber(DeclarePreSubscriber {
                                    target_router_id: Some(target_router_id),
                                    sync_info: Some(sync_info),
                                    id, // Propagate the client subscription id
                                    wire_expr: key_expr,
                                    estimated_time,
                                }),
                            },
                            res.expr(),
                        ));
                    }
                }
                None => tracing::trace!("Unable to find face for zid {}", net.graph[direction].zid),
            }
        }
    }
}

#[inline]
fn send_routeupdate_to_convergence(
    tables: &Tables,
    net: &Network,
    presub_tree_sid: usize,
    originsub_tree_sid: usize,
    pub_router_id: NodeId,
    estimated_time: Duration,
    res: &Arc<Resource>,
    src_face: Option<&Arc<FaceState>>,
    _sub_info: &SubscriberInfo,
    routing_context: NodeId,
) {
    let presub_tree = &net.trees[presub_tree_sid];
    let originsub_tree = &net.trees[originsub_tree_sid];
    if let Some((new_direction, old_direction)) = presub_tree.directions[pub_router_id as usize].zip(originsub_tree.directions[pub_router_id as usize]) {
        // if new_direction == old_direction{
        //     tracing::trace!("convergence point");
        //     // converge_point!
        // }
        // else{
            let direction = new_direction;
            if net.graph.contains_node(direction) {
                match tables.get_face(&net.graph[direction].zid).cloned() {
                    Some(mut someface) => {
                        if src_face
                            .map(|src_face| someface.id != src_face.id)
                            .unwrap_or(true)
                        {
                            let push_declaration = push_declaration_profile(tables, &someface);
                            let key_expr = Resource::decl_key(res, &mut someface, push_declaration);
                            tracing::trace!("send_routeupdate_to_convergence {}",someface.zid);
                            someface.primitives.send_declare(RoutingContext::with_expr(
                                Declare {
                                    interest_id: None,
                                    ext_qos: ext::QoSType::DECLARE,
                                    ext_tstamp: None,
                                    ext_nodeid: ext::NodeIdType {
                                        node_id: routing_context,
                                    },
                                    body: DeclareBody::DeclareRouteUpdate(DeclareRouteUpdate {
                                        pub_router_id,
                                        prev_router_id: originsub_tree_sid as u16,
                                        wire_expr: key_expr,
                                        estimated_time,
                                    }),
                                },
                                res.expr(),
                            ));
                        }
                    }
                    None => tracing::trace!("Unable to find face for zid {}", net.graph[direction].zid),
                }
            }
        // }
    }
}

#[inline]
fn propagate_simple_subscription_to(
    tables: &mut Tables,
    dst_face: &mut Arc<FaceState>,
    res: &Arc<Resource>,
    _sub_info: &SubscriberInfo,
    src_face: &mut Arc<FaceState>,
    full_peer_net: bool,
    send_declare: &mut SendDeclare,
) {
    if src_face.id != dst_face.id
        && !face_hat!(dst_face).local_subs.contains_key(res)
        && if full_peer_net {
            dst_face.whatami == WhatAmI::Client
        } else {
            dst_face.whatami != WhatAmI::Router
                && (src_face.whatami != WhatAmI::Peer
                    || dst_face.whatami != WhatAmI::Peer
                    || hat!(tables).failover_brokering(src_face.zid, dst_face.zid))
        }
    {
        let matching_interests = face_hat!(dst_face)
            .remote_interests
            .values()
            .filter(|i| i.options.subscribers() && i.matches(res))
            .cloned()
            .collect::<Vec<_>>();

        for RemoteInterest {
            res: int_res,
            options,
            ..
        } in matching_interests
        {
            let res = if options.aggregate() {
                int_res.as_ref().unwrap_or(res)
            } else {
                res
            };
            if !face_hat!(dst_face).local_subs.contains_key(res) {
                let id = face_hat!(dst_face).next_id.fetch_add(1, Ordering::SeqCst);
                face_hat_mut!(dst_face).local_subs.insert(res.clone(), id);
                let key_expr =
                    Resource::decl_key(res, dst_face, push_declaration_profile(tables, dst_face));
                send_declare(
                    &dst_face.primitives,
                    RoutingContext::with_expr(
                        Declare {
                            interest_id: None,
                            ext_qos: ext::QoSType::DECLARE,
                            ext_tstamp: None,
                            ext_nodeid: ext::NodeIdType::DEFAULT,
                            body: DeclareBody::DeclareSubscriber(DeclareSubscriber {
                                id,
                                wire_expr: key_expr,
                            }),
                        },
                        res.expr(),
                    ),
                );
            }
        }
    }
}

fn propagate_simple_subscription(
    tables: &mut Tables,
    res: &Arc<Resource>,
    sub_info: &SubscriberInfo,
    src_face: &mut Arc<FaceState>,
    send_declare: &mut SendDeclare,
) {
    let full_peer_net = hat!(tables).full_net(WhatAmI::Peer);
    for mut dst_face in tables
        .faces
        .values()
        .cloned()
        .collect::<Vec<Arc<FaceState>>>()
    {
        propagate_simple_subscription_to(
            tables,
            &mut dst_face,
            res,
            sub_info,
            src_face,
            full_peer_net,
            send_declare,
        );
    }
}

fn propagate_sourced_subscription(
    tables: &Tables,
    res: &Arc<Resource>,
    sub_info: &SubscriberInfo,
    src_face: Option<&Arc<FaceState>>,
    source: &ZenohIdProto,
    net_type: WhatAmI,
) {
    let net = hat!(tables).get_net(net_type).unwrap();
    match net.get_idx(source) {
        Some(tree_sid) => {
            print!("propogate_source_subscription");
            print!("This is net: {:#?}", &net);
            print!("This is net trees: {:#?}", &net.trees[tree_sid.index()]);
            if net.trees.len() > tree_sid.index() {
                send_sourced_subscription_to_net_children(
                    tables,
                    net,
                    &net.trees[tree_sid.index()].children,
                    res,
                    src_face,
                    sub_info,
                    tree_sid.index() as NodeId,
                );
            } else {
                tracing::trace!(
                    "Propagating sub {}: tree for node {} sid:{} not yet ready",
                    res.expr(),
                    tree_sid.index(),
                    source
                );
            }
        }
        None => tracing::error!(
            "Error propagating sub {}: cannot get index of {}!",
            res.expr(),
            source
        ),
    }
}

fn propagate_sourced_presubscription(
    tables: &Tables,
    id: SubscriberId,
    res: &Arc<Resource>,
    sub_info: &SubscriberInfo,
    src_face: Option<&Arc<FaceState>>,
    source: &ZenohIdProto,
    net_type: WhatAmI,
    target_router: &ZenohIdProto,
    sync_info: SyncInfo,
    estimated_time: Duration,
) {
    let net = hat!(tables).get_net(net_type).unwrap();
    match (net.get_idx(source), net.get_idx(target_router)) {
        (Some(tree_sid), Some(target_router_id)) => {
            print!("propagate_source_presubscription");
            print!("This is net: {:#?}", &net);
            print!("This is net trees: {:#?}", &net.trees[tree_sid.index()]);
            if net.trees.len() > tree_sid.index() {
                send_presubscription_to_target_direction(
                    tables,
                    net,
                    &net.trees[tree_sid.index()],
                    target_router_id.index() as NodeId,
                    sync_info,
                    estimated_time,
                    id,
                    res,
                    src_face,
                    sub_info,
                    tree_sid.index() as NodeId,
                );
            } else {
                tracing::trace!(
                    "Propagating sub {}: tree for node {} sid:{} not yet ready",
                    res.expr(),
                    tree_sid.index(),
                    source
                );
            }
        }
        (_, _) => tracing::error!(
            "Error propagating sub {}: cannot get index of {} and {}!",
            res.expr(),
            source,
            target_router
        ),
    }
}

fn propagate_routeupdate(
    tables: &Tables,
    res: &Arc<Resource>,
    sub_info: &SubscriberInfo,
    src_face: Option<&Arc<FaceState>>,
    source: &ZenohIdProto,
    pub_router: &ZenohIdProto,
    prev_router: &ZenohIdProto,
    estimated_time: Duration,
    net_type: WhatAmI,
) {
    tracing::trace!("propagate_routeupdate");
    let net = hat!(tables).get_net(net_type).unwrap();
    match (net.get_idx(source), net.get_idx(prev_router), net.get_idx(pub_router)) {
        (Some(presub_tree_sid), Some(originsub_tree_sid), Some(pub_router_id)) => {
            tracing::trace!("propagate_routeupdate");
            tracing::trace!("This is presub trees: {:#?}", &net.trees[presub_tree_sid.index()]);
            tracing::trace!("This is originsub trees: {:#?}", &net.trees[originsub_tree_sid.index()]);
            tracing::trace!("The pub_router_id: {}", pub_router_id.index());
            if net.trees.len() > presub_tree_sid.index() && net.trees.len() > originsub_tree_sid.index(){
                send_routeupdate_to_convergence(
                    tables,
                    net,
                    presub_tree_sid.index(),
                    originsub_tree_sid.index(),
                    pub_router_id.index() as NodeId,
                    estimated_time,
                    res,
                    src_face,
                    sub_info,
                    presub_tree_sid.index() as NodeId,
                );
            } else {
                tracing::trace!(
                    "Propagating sub {} and sub {}: tree for node {} sid:{} not yet ready",
                    res.expr(),
                    presub_tree_sid.index(), originsub_tree_sid.index(),
                    pub_router_id.index()
                );
            }
        }
        (_, _, _) => tracing::error!(
            "Error propagating sub {}: cannot get index of {} and {}!",
            res.expr(),
            source,
            prev_router
        ),
    }
}

fn register_router_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    router: ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    println!(
        "Register_router_subscription: the modity resource context {}",
        Resource::format_for_no_recursive(&res)
    );
    if !res_hat!(res).router_subs.contains(&router) {
        // Register router subscription
        {
            res_hat_mut!(res).router_subs.insert(router);
            hat_mut!(tables).router_subs.insert(res.clone());
        }

        // Propagate subscription to routers
        propagate_sourced_subscription(tables, res, sub_info, Some(face), &router, WhatAmI::Router);
    }
    // Propagate subscription to peers
    if hat!(tables).full_net(WhatAmI::Peer) && face.whatami != WhatAmI::Peer {
        register_linkstatepeer_subscription(tables, face, res, sub_info, tables.zid)
    }

    // Propagate subscription to clients
    propagate_simple_subscription(tables, res, sub_info, face, send_declare);
}

// fn initiate_declare_routeupdate(
//     tables: &mut Tables,
//     res:
//     face: &mut Arc<FaceState>,
//     sync_info: SyncInfo,
//     estimated_time: Duration,
//     res: &mut Arc<Resource>,
//     sub_info: &SubscriberInfo,
//     router: ZenohIdProto,
//     net_type: WhatAmI,
//     send_declare: &mut SendDeclare,
// ) {

fn presubscription_preparation(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    sync_info: SyncInfo,
    estimated_time: Duration,
    id: SubscriberId,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    router: ZenohIdProto,
    net_type: WhatAmI,
    send_declare: &mut SendDeclare,
) {
    tracing::trace!("presubscription_preparation");
    // 1107: don't need to deconstruct here
    let SyncInfo {
        subscriber_identity,
        pub_router_id,
        sync_seq,
    } = sync_info;
    //
    res_hat_mut!(res).presubscriptions.insert(subscriber_identity, (id, sync_seq));
    hat_mut!(tables).pre_subs.entry(subscriber_identity).or_default().push(res.clone());
    // First trigger the route update sending
    // let prev_router_id = get_
    // tables.get_face(&tables.zid).cloned().unwrap().primitives.send_declare(RoutingContext::with_expr(
    //                             Declare {
    //                                 interest_id: None,
    //                                 ext_qos: ext::QoSType::DECLARE,
    //                                 ext_tstamp: None,
    //                                 ext_nodeid: ext::NodeIdType {
    //                                     node_id: 0,
    //                                 },
    //                                 body: DeclareBody::DeclareRouteUpdate(DeclareRouteUpdate {
    //                                     pub_router_id,
    //                                     prev_router_id: originsub_tree_sid as u16,
    //                                     wire_expr: key_expr,
    //                                     estimated_time,
    //                                 }),
    //                             },
    //                             res.expr(),
    //                         ));
    // 1107: No need to change it here, remove get_router, pass it inside
    if let Some(pub_router) = get_router(tables, face, pub_router_id) {
        propagate_routeupdate(
            tables,
            res,
            sub_info,
            None,
            &tables.zid,
            &pub_router,
            &router,
            estimated_time,
            WhatAmI::Router
        );
    }
    // face prebuilt for the comming client

}

fn register_router_presubscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    target_router: ZenohIdProto,
    sync_info: SyncInfo,
    estimated_time: Duration,
    id: SubscriberId,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    router: ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    tracing::trace!("register_router_presubscription");
    tracing::trace!("{:?}", res);
    tracing::trace!("{:?}", res_hat!(res).router_subs);
    if !res_hat!(res).router_subs.contains(&router) {
        // Register the presubscription
        {
            tracing::trace!("router: {:?}", router);
            res_hat_mut!(res).router_subs.insert(router);
            tracing::trace!("target_router: {:?}", target_router);
            res_hat_mut!(res).router_subs.insert(target_router);
            tracing::trace!("register_router_presubscription for {} and {}",router, target_router);
            hat_mut!(tables).router_subs.insert(res.clone());
        }
        if target_router == tables.zid {
            // Change the key expression
            if let Some(key_expr) = res.expr().strip_prefix("%/"){
                if let Some(mut res) = Resource::get_resource(&tables.root_res, &key_expr){
                    tracing::trace!("After stripping the pre-subscribe prefix, res: {}", res.expr());

                    // Trigger the DataRouteUpdate
                    presubscription_preparation(
                        tables,
                        face,
                        sync_info,
                        estimated_time,
                        id,
                        &mut res,
                        sub_info,
                        router,
                        WhatAmI::Router,
                        send_declare
                    );
                }
            }
        }
        else{
            // Propagate subscription to routers
            // 1107: The sync_info can be built here, or before the send
            if let Some(pub_router_id) = get_router_id(tables, face, sync_info.pub_router_id){
                let sync_info = SyncInfo {
                    pub_router_id:pub_router_id,
                    ..sync_info
                };
                propagate_sourced_presubscription(
                    tables,
                    id,
                    res,
                    sub_info,
                    Some(face),
                    &router,
                    WhatAmI::Router,
                    &target_router,
                    sync_info,
                    estimated_time,
                );
            }
        }
    }

    // // Propagate subscription to clients
    // propagate_simple_subscription(tables, res, sub_info, face, send_declare);
}

fn register_router_prerouteupdate(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    pub_router: ZenohIdProto,
    prev_router: ZenohIdProto,
    estimated_time: Duration,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    router: ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    tracing::trace!("register_router_prerouteupdate");
    // first insert the router into the subscriber
    tracing::trace!("res: {}, res.router_subs: {:?}", res.expr(), res_hat!(res).router_subs);
    if !res_hat!(res).router_subs.contains(&router) {
        // Register router subscription
        {
            res_hat_mut!(res).router_subs.insert(router);
            hat_mut!(tables).router_subs.insert(res.clone());
        }
        // calculate if it is the convergence point
        // if not, propagate the routeupdate packet to the publisher
        // Propagate subscription to routers
        // Move this out(todo)
    }
    propagate_routeupdate(tables, res, sub_info, Some(face), &router, &pub_router, &prev_router, estimated_time, WhatAmI::Router);
}

fn declare_router_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    router: ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    register_router_subscription(tables, face, res, sub_info, router, send_declare);
}

fn declare_router_presubscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    target_router: ZenohIdProto,
    sync_info: SyncInfo,
    estimated_time: Duration,
    id: SubscriberId,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    router: ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    register_router_presubscription(
        tables,
        face,
        target_router,
        sync_info,
        estimated_time,
        id,
        res,
        sub_info,
        router,
        send_declare,
    );
}

fn register_linkstatepeer_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    peer: ZenohIdProto,
) {
    if !res_hat!(res).linkstatepeer_subs.contains(&peer) {
        // Register peer subscription
        {
            res_hat_mut!(res).linkstatepeer_subs.insert(peer);
            hat_mut!(tables).linkstatepeer_subs.insert(res.clone());
        }

        // Propagate subscription to peers
        propagate_sourced_subscription(tables, res, sub_info, Some(face), &peer, WhatAmI::Peer);
    }
}

fn declare_linkstatepeer_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    peer: ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    register_linkstatepeer_subscription(tables, face, res, sub_info, peer);
    let propa_sub_info = *sub_info;
    let zid = tables.zid;
    register_router_subscription(tables, face, res, &propa_sub_info, zid, send_declare);
}

fn register_simple_subscription(
    _tables: &mut Tables,
    face: &mut Arc<FaceState>,
    id: SubscriberId,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
) {
    // Register subscription
    {
        let res = get_mut_unchecked(res);
        match res.session_ctxs.get_mut(&face.id) {
            Some(ctx) => {
                if ctx.subs.is_none() {
                    get_mut_unchecked(ctx).subs = Some(*sub_info);
                }
            }
            None => {
                let ctx = res
                    .session_ctxs
                    .entry(face.id)
                    .or_insert_with(|| Arc::new(SessionContext::new(face.clone())));
                get_mut_unchecked(ctx).subs = Some(*sub_info);
            }
        }
    }
    face_hat_mut!(face).remote_subs.insert(id, res.clone());
}

fn declare_simple_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    id: SubscriberId,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    send_declare: &mut SendDeclare,
) {
    register_simple_subscription(tables, face, id, res, sub_info);
    let zid = tables.zid;
    register_router_subscription(tables, face, res, sub_info, zid, send_declare);
}

/// Read target router ZID from handover prediction file
/// File format: {"imsi":21,"current_cell":13,"predicted_target":15,"target_edge_ip":"10.6.0.2",...}
fn get_target_router_from_prediction() -> Option<ZenohIdProto> {
    // Mapping: target_edge_ip -> zid
    let edge_ip_to_zid: HashMap<&str, &str> = [
        ("10.1.0.2", "c10274ef26525ccdb47947a9dfdd7f01"),
        ("10.2.0.2", "bef96187ff749ce9379ef4257d963e18"),
        ("10.3.0.2", "7c3d8f2a1b4e6590a8d2f7c3e9b1405d"),
        ("10.4.0.2", "2e9a4c6f8d1b3570c9e2a5d8f0b74631"),
        ("10.5.0.2", "5f1c9e3a7d0b8264f6c1a9e3d7b05842"),
        ("10.6.0.2", "8a2d6f0c4e9b1753a8d2f6c0e4b91765"),
        ("10.7.0.2", "3b7e1a5d9f0c4826b3e7a1d5f9c04837"),
    ]
    .iter()
    .cloned()
    .collect();

    // Read the prediction file
    let content = match fs::read_to_string("/mnt/ns3_handover.json") {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to read handover prediction file: {}", e);
            return None;
        }
    };

    // Parse JSON to extract target_edge_ip
    // Format: {"imsi":21,...,"target_edge_ip":"10.6.0.2",...}
    let target_edge_ip = content
        .split("\"target_edge_ip\":\"")
        .nth(1)?
        .split('"')
        .next()?;

    tracing::trace!("Parsed target_edge_ip from prediction file: {}", target_edge_ip);

    // Look up the ZID
    let zid_str = match edge_ip_to_zid.get(target_edge_ip) {
        Some(zid) => *zid,
        None => {
            tracing::error!("Unknown target_edge_ip: {}", target_edge_ip);
            return None;
        }
    };

    match ZenohIdProto::from_str(zid_str) {
        Ok(zid) => {
            tracing::trace!("Resolved target router ZID: {}", zid);
            Some(zid)
        }
        Err(e) => {
            tracing::error!("Failed to parse ZID {}: {}", zid_str, e);
            None
        }
    }
}

fn declare_simple_presubscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    estimated_time: Duration,
    id: SubscriberId,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    send_declare: &mut SendDeclare,
) {
    tracing::trace!("declare_simple_presubscription");
    // register_simple_subscription(tables, face, id, res, sub_info);

    // Store the presubscription info
    let client_id = face.zid;
    hat_mut!(tables).pre_subs.entry(client_id).or_default().push(res.clone());

    let zid = tables.zid;
    let subscriber_identity = face.zid;
    // Read target router from handover prediction file
    let target_router = match get_target_router_from_prediction() {
        Some(zid) => zid,
        None => {
            tracing::error!("Failed to get target router from prediction file");
            return;
        }
    };
    let pub_router_id = 2;  // Filled in by the subscription (todo!)
    let sync_seq = 123;
    let sync_info = SyncInfo { subscriber_identity, pub_router_id, sync_seq };
    let estimated_time = Duration::from_millis(3);
    // 1107: Do not build the SyncInfo here, pass the component subscriber_identity, pub_router, sync_seq
    register_router_presubscription(
        tables,
        face,
        target_router,
        sync_info,
        estimated_time,
        id,
        res,
        sub_info,
        zid,
        send_declare,
    );
}

#[inline]
fn remote_router_subs(tables: &Tables, res: &Arc<Resource>) -> bool {
    res.context.is_some()
        && res_hat!(res)
            .router_subs
            .iter()
            .any(|peer| peer != &tables.zid)
}

#[inline]
fn remote_linkstatepeer_subs(tables: &Tables, res: &Arc<Resource>) -> bool {
    res.context.is_some()
        && res_hat!(res)
            .linkstatepeer_subs
            .iter()
            .any(|peer| peer != &tables.zid)
}

#[inline]
fn simple_subs(res: &Arc<Resource>) -> Vec<Arc<FaceState>> {
    res.session_ctxs
        .values()
        .filter_map(|ctx| {
            if ctx.subs.is_some() {
                Some(ctx.face.clone())
            } else {
                None
            }
        })
        .collect()
}

#[inline]
fn remote_simple_subs(res: &Arc<Resource>, face: &Arc<FaceState>) -> bool {
    res.session_ctxs
        .values()
        .any(|ctx| ctx.face.id != face.id && ctx.subs.is_some())
}

#[inline]
fn send_forget_sourced_subscription_to_net_children(
    tables: &Tables,
    net: &Network,
    children: &[NodeIndex],
    res: &Arc<Resource>,
    src_face: Option<&Arc<FaceState>>,
    routing_context: Option<NodeId>,
) {
    for child in children {
        if net.graph.contains_node(*child) {
            match tables.get_face(&net.graph[*child].zid).cloned() {
                Some(mut someface) => {
                    if src_face
                        .map(|src_face| someface.id != src_face.id)
                        .unwrap_or(true)
                    {
                        let push_declaration = push_declaration_profile(tables, &someface);
                        let wire_expr = Resource::decl_key(res, &mut someface, push_declaration);

                        someface.primitives.send_declare(RoutingContext::with_expr(
                            Declare {
                                interest_id: None,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType {
                                    node_id: routing_context.unwrap_or(0),
                                },
                                body: DeclareBody::UndeclareSubscriber(UndeclareSubscriber {
                                    id: 0, // Sourced subscriptions do not use ids
                                    ext_wire_expr: WireExprType { wire_expr },
                                }),
                            },
                            res.expr(),
                        ));
                    }
                }
                None => tracing::trace!("Unable to find face for zid {}", net.graph[*child].zid),
            }
        }
    }
}

fn propagate_forget_simple_subscription(
    tables: &mut Tables,
    res: &Arc<Resource>,
    send_declare: &mut SendDeclare,
) {
    for mut face in tables.faces.values().cloned() {
        if let Some(id) = face_hat_mut!(&mut face).local_subs.remove(res) {
            send_declare(
                &face.primitives,
                RoutingContext::with_expr(
                    Declare {
                        interest_id: None,
                        ext_qos: ext::QoSType::DECLARE,
                        ext_tstamp: None,
                        ext_nodeid: ext::NodeIdType::DEFAULT,
                        body: DeclareBody::UndeclareSubscriber(UndeclareSubscriber {
                            id,
                            ext_wire_expr: WireExprType::null(),
                        }),
                    },
                    res.expr(),
                ),
            );
        }
        for res in face_hat!(&mut face)
            .local_subs
            .keys()
            .cloned()
            .collect::<Vec<Arc<Resource>>>()
        {
            if !res.context().matches.iter().any(|m| {
                m.upgrade().is_some_and(|m| {
                    m.context.is_some()
                        && (remote_simple_subs(&m, &face)
                            || remote_linkstatepeer_subs(tables, &m)
                            || remote_router_subs(tables, &m))
                })
            }) {
                if let Some(id) = face_hat_mut!(&mut face).local_subs.remove(&res) {
                    send_declare(
                        &face.primitives,
                        RoutingContext::with_expr(
                            Declare {
                                interest_id: None,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType::DEFAULT,
                                body: DeclareBody::UndeclareSubscriber(UndeclareSubscriber {
                                    id,
                                    ext_wire_expr: WireExprType::null(),
                                }),
                            },
                            res.expr(),
                        ),
                    );
                }
            }
        }
    }
}

fn propagate_forget_simple_subscription_to_peers(
    tables: &mut Tables,
    res: &Arc<Resource>,
    send_declare: &mut SendDeclare,
) {
    if !hat!(tables).full_net(WhatAmI::Peer)
        && res_hat!(res).router_subs.len() == 1
        && res_hat!(res).router_subs.contains(&tables.zid)
    {
        for mut face in tables
            .faces
            .values()
            .cloned()
            .collect::<Vec<Arc<FaceState>>>()
        {
            if face.whatami == WhatAmI::Peer
                && face_hat!(face).local_subs.contains_key(res)
                && !res.session_ctxs.values().any(|s| {
                    face.zid != s.face.zid
                        && s.subs.is_some()
                        && (s.face.whatami == WhatAmI::Client
                            || (s.face.whatami == WhatAmI::Peer
                                && hat!(tables).failover_brokering(s.face.zid, face.zid)))
                })
            {
                if let Some(id) = face_hat_mut!(&mut face).local_subs.remove(res) {
                    send_declare(
                        &face.primitives,
                        RoutingContext::with_expr(
                            Declare {
                                interest_id: None,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType::DEFAULT,
                                body: DeclareBody::UndeclareSubscriber(UndeclareSubscriber {
                                    id,
                                    ext_wire_expr: WireExprType::null(),
                                }),
                            },
                            res.expr(),
                        ),
                    );
                }
            }
        }
    }
}

fn propagate_forget_sourced_subscription(
    tables: &Tables,
    res: &Arc<Resource>,
    src_face: Option<&Arc<FaceState>>,
    source: &ZenohIdProto,
    net_type: WhatAmI,
) {
    let net = hat!(tables).get_net(net_type).unwrap();
    match net.get_idx(source) {
        Some(tree_sid) => {
            if net.trees.len() > tree_sid.index() {
                send_forget_sourced_subscription_to_net_children(
                    tables,
                    net,
                    &net.trees[tree_sid.index()].children,
                    res,
                    src_face,
                    Some(tree_sid.index() as NodeId),
                );
            } else {
                tracing::trace!(
                    "Propagating forget sub {}: tree for node {} sid:{} not yet ready",
                    res.expr(),
                    tree_sid.index(),
                    source
                );
            }
        }
        None => tracing::error!(
            "Error propagating forget sub {}: cannot get index of {}!",
            res.expr(),
            source
        ),
    }
}

fn unregister_router_subscription(
    tables: &mut Tables,
    res: &mut Arc<Resource>,
    router: &ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    res_hat_mut!(res).router_subs.retain(|sub| sub != router);

    if res_hat!(res).router_subs.is_empty() {
        hat_mut!(tables)
            .router_subs
            .retain(|sub| !Arc::ptr_eq(sub, res));

        if hat_mut!(tables).full_net(WhatAmI::Peer) {
            undeclare_linkstatepeer_subscription(tables, None, res, &tables.zid.clone());
        }
        propagate_forget_simple_subscription(tables, res, send_declare);
    }

    propagate_forget_simple_subscription_to_peers(tables, res, send_declare);
}

fn undeclare_router_subscription(
    tables: &mut Tables,
    face: Option<&Arc<FaceState>>,
    res: &mut Arc<Resource>,
    router: &ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    if res_hat!(res).router_subs.contains(router) {
        unregister_router_subscription(tables, res, router, send_declare);
        propagate_forget_sourced_subscription(tables, res, face, router, WhatAmI::Router);
    }
}

fn forget_router_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    res: &mut Arc<Resource>,
    router: &ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    undeclare_router_subscription(tables, Some(face), res, router, send_declare);
}

fn unregister_peer_subscription(tables: &mut Tables, res: &mut Arc<Resource>, peer: &ZenohIdProto) {
    res_hat_mut!(res)
        .linkstatepeer_subs
        .retain(|sub| sub != peer);

    if res_hat!(res).linkstatepeer_subs.is_empty() {
        hat_mut!(tables)
            .linkstatepeer_subs
            .retain(|sub| !Arc::ptr_eq(sub, res));
    }
}

fn undeclare_linkstatepeer_subscription(
    tables: &mut Tables,
    face: Option<&Arc<FaceState>>,
    res: &mut Arc<Resource>,
    peer: &ZenohIdProto,
) {
    if res_hat!(res).linkstatepeer_subs.contains(peer) {
        unregister_peer_subscription(tables, res, peer);
        propagate_forget_sourced_subscription(tables, res, face, peer, WhatAmI::Peer);
    }
}

fn forget_linkstatepeer_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    res: &mut Arc<Resource>,
    peer: &ZenohIdProto,
    send_declare: &mut SendDeclare,
) {
    undeclare_linkstatepeer_subscription(tables, Some(face), res, peer);
    let simple_subs = res.session_ctxs.values().any(|ctx| ctx.subs.is_some());
    let linkstatepeer_subs = remote_linkstatepeer_subs(tables, res);
    let zid = tables.zid;
    if !simple_subs && !linkstatepeer_subs {
        undeclare_router_subscription(tables, None, res, &zid, send_declare);
    }
}

pub(super) fn undeclare_simple_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    res: &mut Arc<Resource>,
    send_declare: &mut SendDeclare,
) {
    if !face_hat_mut!(face).remote_subs.values().any(|s| *s == *res) {
        if let Some(ctx) = get_mut_unchecked(res).session_ctxs.get_mut(&face.id) {
            get_mut_unchecked(ctx).subs = None;
        }

        let mut simple_subs = simple_subs(res);
        let router_subs = remote_router_subs(tables, res);
        let linkstatepeer_subs = remote_linkstatepeer_subs(tables, res);
        if simple_subs.is_empty() && !linkstatepeer_subs {
            undeclare_router_subscription(tables, None, res, &tables.zid.clone(), send_declare);
        } else {
            propagate_forget_simple_subscription_to_peers(tables, res, send_declare);
        }

        if simple_subs.len() == 1 && !router_subs && !linkstatepeer_subs {
            let mut face = &mut simple_subs[0];
            if let Some(id) = face_hat_mut!(face).local_subs.remove(res) {
                send_declare(
                    &face.primitives,
                    RoutingContext::with_expr(
                        Declare {
                            interest_id: None,
                            ext_qos: ext::QoSType::DECLARE,
                            ext_tstamp: None,
                            ext_nodeid: ext::NodeIdType::DEFAULT,
                            body: DeclareBody::UndeclareSubscriber(UndeclareSubscriber {
                                id,
                                ext_wire_expr: WireExprType::null(),
                            }),
                        },
                        res.expr(),
                    ),
                );
            }
            for res in face_hat!(face)
                .local_subs
                .keys()
                .cloned()
                .collect::<Vec<Arc<Resource>>>()
            {
                if !res.context().matches.iter().any(|m| {
                    m.upgrade().is_some_and(|m| {
                        m.context.is_some()
                            && (remote_simple_subs(&m, face)
                                || remote_linkstatepeer_subs(tables, &m)
                                || remote_router_subs(tables, &m))
                    })
                }) {
                    if let Some(id) = face_hat_mut!(&mut face).local_subs.remove(&res) {
                        send_declare(
                            &face.primitives,
                            RoutingContext::with_expr(
                                Declare {
                                    interest_id: None,
                                    ext_qos: ext::QoSType::DECLARE,
                                    ext_tstamp: None,
                                    ext_nodeid: ext::NodeIdType::DEFAULT,
                                    body: DeclareBody::UndeclareSubscriber(UndeclareSubscriber {
                                        id,
                                        ext_wire_expr: WireExprType::null(),
                                    }),
                                },
                                res.expr(),
                            ),
                        );
                    }
                }
            }
        }
    }
}

fn forget_simple_subscription(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    id: SubscriberId,
    send_declare: &mut SendDeclare,
) -> Option<Arc<Resource>> {
    if let Some(mut res) = face_hat_mut!(face).remote_subs.remove(&id) {
        undeclare_simple_subscription(tables, face, &mut res, send_declare);
        Some(res)
    } else {
        None
    }
}

pub(super) fn pubsub_remove_node(
    tables: &mut Tables,
    node: &ZenohIdProto,
    net_type: WhatAmI,
    send_declare: &mut SendDeclare,
) {
    match net_type {
        WhatAmI::Router => {
            for mut res in hat!(tables)
                .router_subs
                .iter()
                .filter(|res| res_hat!(res).router_subs.contains(node))
                .cloned()
                .collect::<Vec<Arc<Resource>>>()
            {
                unregister_router_subscription(tables, &mut res, node, send_declare);

                update_matches_data_routes(tables, &mut res);
                Resource::clean(&mut res)
            }
        }
        WhatAmI::Peer => {
            for mut res in hat!(tables)
                .linkstatepeer_subs
                .iter()
                .filter(|res| res_hat!(res).linkstatepeer_subs.contains(node))
                .cloned()
                .collect::<Vec<Arc<Resource>>>()
            {
                unregister_peer_subscription(tables, &mut res, node);
                let simple_subs = res.session_ctxs.values().any(|ctx| ctx.subs.is_some());
                let linkstatepeer_subs = remote_linkstatepeer_subs(tables, &res);
                if !simple_subs && !linkstatepeer_subs {
                    undeclare_router_subscription(
                        tables,
                        None,
                        &mut res,
                        &tables.zid.clone(),
                        send_declare,
                    );
                }

                update_matches_data_routes(tables, &mut res);
                Resource::clean(&mut res)
            }
        }
        _ => (),
    }
}

pub(super) fn pubsub_tree_change(
    tables: &mut Tables,
    new_children: &[Vec<NodeIndex>],
    net_type: WhatAmI,
) {
    let net = match hat!(tables).get_net(net_type) {
        Some(net) => net,
        None => {
            tracing::error!("Error accessing net in pubsub_tree_change!");
            return;
        }
    };
    // propagate subs to new children
    for (tree_sid, tree_children) in new_children.iter().enumerate() {
        if !tree_children.is_empty() {
            let tree_idx = NodeIndex::new(tree_sid);
            if net.graph.contains_node(tree_idx) {
                let tree_id = net.graph[tree_idx].zid;

                let subs_res = match net_type {
                    WhatAmI::Router => &hat!(tables).router_subs,
                    _ => &hat!(tables).linkstatepeer_subs,
                };

                for res in subs_res {
                    let subs = match net_type {
                        WhatAmI::Router => &res_hat!(res).router_subs,
                        _ => &res_hat!(res).linkstatepeer_subs,
                    };
                    for sub in subs {
                        if *sub == tree_id {
                            let sub_info = SubscriberInfo;
                            send_sourced_subscription_to_net_children(
                                tables,
                                net,
                                tree_children,
                                res,
                                None,
                                &sub_info,
                                tree_sid as NodeId,
                            );
                        }
                    }
                }
            }
        }
    }

    // recompute routes
    update_data_routes_from(tables, &mut tables.root_res.clone());

    // println!("[router] Now the resource tree after 'pubsub tree change' will print");
    // println!("[router] root_res tree {:#?}", tables.root_res);
}

pub(super) fn pubsub_linkstate_change(
    tables: &mut Tables,
    zid: &ZenohIdProto,
    links: &[ZenohIdProto],
    send_declare: &mut SendDeclare,
) {
    if let Some(mut src_face) = tables.get_face(zid).cloned() {
        if hat!(tables).router_peers_failover_brokering && src_face.whatami == WhatAmI::Peer {
            let to_forget = face_hat!(src_face)
                .local_subs
                .keys()
                .filter(|res| {
                    let client_subs = res
                        .session_ctxs
                        .values()
                        .any(|ctx| ctx.face.whatami == WhatAmI::Client && ctx.subs.is_some());
                    !remote_router_subs(tables, res)
                        && !client_subs
                        && !res.session_ctxs.values().any(|ctx| {
                            ctx.face.whatami == WhatAmI::Peer
                                && src_face.id != ctx.face.id
                                && HatTables::failover_brokering_to(links, ctx.face.zid)
                        })
                })
                .cloned()
                .collect::<Vec<Arc<Resource>>>();
            for res in to_forget {
                if let Some(id) = face_hat_mut!(&mut src_face).local_subs.remove(&res) {
                    let wire_expr = Resource::get_best_key(&res, "", src_face.id);
                    send_declare(
                        &src_face.primitives,
                        RoutingContext::with_expr(
                            Declare {
                                interest_id: None,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType::default(),
                                body: DeclareBody::UndeclareSubscriber(UndeclareSubscriber {
                                    id,
                                    ext_wire_expr: WireExprType { wire_expr },
                                }),
                            },
                            res.expr(),
                        ),
                    );
                }
            }

            for mut dst_face in tables.faces.values().cloned() {
                if src_face.id != dst_face.id
                    && HatTables::failover_brokering_to(links, dst_face.zid)
                {
                    for res in face_hat!(src_face).remote_subs.values() {
                        if !face_hat!(dst_face).local_subs.contains_key(res) {
                            let id = face_hat!(dst_face).next_id.fetch_add(1, Ordering::SeqCst);
                            face_hat_mut!(&mut dst_face)
                                .local_subs
                                .insert(res.clone(), id);
                            let push_declaration = push_declaration_profile(tables, &dst_face);
                            let key_expr = Resource::decl_key(res, &mut dst_face, push_declaration);
                            send_declare(
                                &dst_face.primitives,
                                RoutingContext::with_expr(
                                    Declare {
                                        interest_id: None,
                                        ext_qos: ext::QoSType::DECLARE,
                                        ext_tstamp: None,
                                        ext_nodeid: ext::NodeIdType::default(),
                                        body: DeclareBody::DeclareSubscriber(DeclareSubscriber {
                                            id,
                                            wire_expr: key_expr,
                                        }),
                                    },
                                    res.expr(),
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn make_sub_id(res: &Arc<Resource>, face: &mut Arc<FaceState>, mode: InterestMode) -> u32 {
    if mode.future() {
        if let Some(id) = face_hat!(face).local_subs.get(res) {
            *id
        } else {
            let id = face_hat!(face).next_id.fetch_add(1, Ordering::SeqCst);
            face_hat_mut!(face).local_subs.insert(res.clone(), id);
            id
        }
    } else {
        0
    }
}

pub(crate) fn declare_sub_interest(
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    id: InterestId,
    res: Option<&mut Arc<Resource>>,
    mode: InterestMode,
    aggregate: bool,
    send_declare: &mut SendDeclare,
) {
    if mode.current() {
        let interest_id = Some(id);
        if let Some(res) = res.as_ref() {
            if aggregate {
                if hat!(tables).router_subs.iter().any(|sub| {
                    sub.context.is_some()
                        && sub.matches(res)
                        && (remote_simple_subs(sub, face)
                            || remote_linkstatepeer_subs(tables, sub)
                            || remote_router_subs(tables, sub))
                }) {
                    let id = make_sub_id(res, face, mode);
                    let wire_expr =
                        Resource::decl_key(res, face, push_declaration_profile(tables, face));
                    send_declare(
                        &face.primitives,
                        RoutingContext::with_expr(
                            Declare {
                                interest_id,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType::DEFAULT,
                                body: DeclareBody::DeclareSubscriber(DeclareSubscriber {
                                    id,
                                    wire_expr,
                                }),
                            },
                            res.expr(),
                        ),
                    );
                }
            } else {
                for sub in &hat!(tables).router_subs {
                    if sub.context.is_some()
                        && sub.matches(res)
                        && (res_hat!(sub).router_subs.iter().any(|r| *r != tables.zid)
                            || res_hat!(sub)
                                .linkstatepeer_subs
                                .iter()
                                .any(|r| *r != tables.zid)
                            || sub.session_ctxs.values().any(|s| {
                                s.face.id != face.id
                                    && s.subs.is_some()
                                    && (s.face.whatami == WhatAmI::Client
                                        || face.whatami == WhatAmI::Client
                                        || (s.face.whatami == WhatAmI::Peer
                                            && hat!(tables)
                                                .failover_brokering(s.face.zid, face.zid)))
                            }))
                    {
                        let id = make_sub_id(sub, face, mode);
                        let wire_expr =
                            Resource::decl_key(sub, face, push_declaration_profile(tables, face));
                        send_declare(
                            &face.primitives,
                            RoutingContext::with_expr(
                                Declare {
                                    interest_id,
                                    ext_qos: ext::QoSType::DECLARE,
                                    ext_tstamp: None,
                                    ext_nodeid: ext::NodeIdType::DEFAULT,
                                    body: DeclareBody::DeclareSubscriber(DeclareSubscriber {
                                        id,
                                        wire_expr,
                                    }),
                                },
                                sub.expr(),
                            ),
                        );
                    }
                }
            }
        } else {
            for sub in &hat!(tables).router_subs {
                if sub.context.is_some()
                    && (res_hat!(sub).router_subs.iter().any(|r| *r != tables.zid)
                        || res_hat!(sub)
                            .linkstatepeer_subs
                            .iter()
                            .any(|r| *r != tables.zid)
                        || sub.session_ctxs.values().any(|s| {
                            s.subs.is_some()
                                && (s.face.whatami != WhatAmI::Peer
                                    || face.whatami != WhatAmI::Peer
                                    || hat!(tables).failover_brokering(s.face.zid, face.zid))
                        }))
                {
                    let id = make_sub_id(sub, face, mode);
                    let wire_expr =
                        Resource::decl_key(sub, face, push_declaration_profile(tables, face));
                    send_declare(
                        &face.primitives,
                        RoutingContext::with_expr(
                            Declare {
                                interest_id,
                                ext_qos: ext::QoSType::DECLARE,
                                ext_tstamp: None,
                                ext_nodeid: ext::NodeIdType::DEFAULT,
                                body: DeclareBody::DeclareSubscriber(DeclareSubscriber {
                                    id,
                                    wire_expr,
                                }),
                            },
                            sub.expr(),
                        ),
                    );
                }
            }
        }
    }
}

pub(crate) fn activate_presubscription_to_subscription(
    hat_code: &(dyn crate::net::routing::hat::HatTrait + Send + Sync),
    tables: &mut Tables,
    face: &mut Arc<FaceState>,
    id: SubscriberId,
    res: &mut Arc<Resource>,
    sub_info: &SubscriberInfo,
    ) {
        // face_hat_mut!(face).remote_subs.insert(id, res.clone());
        register_simple_subscription(tables, face, id, res, sub_info);
        // register_simple_subscription(for insert the session_ctx in resource, compute data route will use it)
        // Do I need to recompute the data route, or compute it at first, just make the resource valid and not valid at first,
    }
impl HatPubSubTrait for HatCode {
    fn declare_subscription(
        &self,
        tables: &mut Tables,
        face: &mut Arc<FaceState>,
        id: SubscriberId,
        res: &mut Arc<Resource>,
        sub_info: &SubscriberInfo,
        node_id: NodeId,
        send_declare: &mut SendDeclare,
    ) {
        // backtrace::trace(|frame|{
        //     // let ip = frame.ip();
        //     // let symbol_address = frame.symbol_address();
        //     // dbg!(ip,symbol_address);
        //     backtrace::resolve_frame(frame, |symbol|{
        //         if let Some(name) = symbol.name(){
        //             dbg!(name);
        //         }
        //         if let Some(filename) = symbol.filename(){
        //             dbg!(filename);
        //         }
        //         dbg!();
        //     });
        //     true
        // });

        match face.whatami {
            WhatAmI::Router => {
                if let Some(router) = get_router(tables, face, node_id) {
                    declare_router_subscription(tables, face, res, sub_info, router, send_declare)
                }
            }
            WhatAmI::Peer => {
                if hat!(tables).full_net(WhatAmI::Peer) {
                    if let Some(peer) = get_peer(tables, face, node_id) {
                        declare_linkstatepeer_subscription(
                            tables,
                            face,
                            res,
                            sub_info,
                            peer,
                            send_declare,
                        )
                    }
                } else {
                    declare_simple_subscription(tables, face, id, res, sub_info, send_declare)
                }
            }
            _ => declare_simple_subscription(tables, face, id, res, sub_info, send_declare),
        }
    }

    fn declare_presubscription(
        &self,
        tables: &mut Tables,
        face: &mut Arc<FaceState>,
        id: SubscriberId,
        target_router_id: Option<NodeId>,
        sync_info: Option<SyncInfo>,
        estimated_time: Duration,
        res: &mut Arc<Resource>,
        sub_info: &SubscriberInfo,
        node_id: NodeId,
        send_declare: &mut SendDeclare,
    ) {
        match face.whatami {
            WhatAmI::Router => {
                tracing::trace!("The presubscription is from router!");
                let Some((target_router_id, sync_info)) = target_router_id.zip(sync_info) else { return };
                let router_opt = get_router(tables, face, node_id);
                let target_opt = get_router(tables, face, target_router_id);
                // 1107: Change the get_router_id --> get_router, first get the ZenohIdProto 'pub_opt'
                let pub_opt = get_router(tables, face, sync_info.pub_router_id);
                if let (Some(router), Some(target_router), Some(_)) = (router_opt, target_opt, pub_opt) {
                    // 1107: Do not change the sync_info SyncInfo here
                    // let sync_info = SyncInfo {
                    //     pub_router_id:pub_router_id,
                    //     ..sync_info
                    // };
                    declare_router_presubscription(
                        tables,
                        face,
                        target_router,
                        sync_info,
                        estimated_time,
                        id,
                        res,
                        sub_info,
                        router,
                        send_declare,
                    )
                }

            }
            WhatAmI::Client => {
                tracing::trace!("Receive Presubscription!!!");
                declare_simple_presubscription(tables, face, estimated_time, id, res, sub_info, send_declare);
            }
            _ => {}
        }
    }

    fn declare_routeupdate(
        &self,
        tables: &mut Tables,
        face: &mut Arc<FaceState>,
        pub_router_id: NodeId,
        prev_router_id: NodeId,
        estimated_time: Duration,
        res: Option<Arc<Resource>>,
        sub_info: &SubscriberInfo,
        node_id: NodeId,
        send_declare: &mut SendDeclare,
    ) -> Option<Arc<Resource>>
    {
        tracing::trace!("declare_routeupdate");
        match face.whatami {
            WhatAmI::Router => {
                tracing::trace!("declare_routeupdate sent from router");
                if let Some(mut res) = res {
                    if let Some((pub_router, prev_router)) = get_router(tables, face, pub_router_id).zip(get_router(tables, face, prev_router_id))  {
                        if let Some(router) = get_router(tables, face, node_id) {
                            register_router_prerouteupdate(tables, face, pub_router, prev_router, estimated_time, &mut res, sub_info, router, send_declare);
                            Some(res)
                        } else {
                            None
                        }
                    } else{
                        None
                    }
                } else {
                    None
                }
            }
            _ => None
        }
    }

    fn undeclare_subscription(
        &self,
        tables: &mut Tables,
        face: &mut Arc<FaceState>,
        id: SubscriberId,
        res: Option<Arc<Resource>>,
        node_id: NodeId,
        send_declare: &mut SendDeclare,
    ) -> Option<Arc<Resource>> {
        dbg!();
        match face.whatami {
            WhatAmI::Router => {
                dbg!();
                if let Some(mut res) = res {
                    if let Some(router) = get_router(tables, face, node_id) {
                        forget_router_subscription(tables, face, &mut res, &router, send_declare);
                        Some(res)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            WhatAmI::Peer => {
                if hat!(tables).full_net(WhatAmI::Peer) {
                    if let Some(mut res) = res {
                        if let Some(peer) = get_peer(tables, face, node_id) {
                            forget_linkstatepeer_subscription(
                                tables,
                                face,
                                &mut res,
                                &peer,
                                send_declare,
                            );
                            Some(res)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    forget_simple_subscription(tables, face, id, send_declare)
                }
            }
            _ => forget_simple_subscription(tables, face, id, send_declare),
        }
    }

    fn get_subscriptions(&self, tables: &Tables) -> Vec<(Arc<Resource>, Sources)> {
        // Compute the list of known suscriptions (keys)
        hat!(tables)
            .router_subs
            .iter()
            .map(|s| {
                (
                    s.clone(),
                    // Compute the list of routers, peers and clients that are known
                    // sources of those subscriptions
                    Sources {
                        routers: Vec::from_iter(res_hat!(s).router_subs.iter().cloned()),
                        peers: if hat!(tables).full_net(WhatAmI::Peer) {
                            Vec::from_iter(res_hat!(s).linkstatepeer_subs.iter().cloned())
                        } else {
                            s.session_ctxs
                                .values()
                                .filter_map(|f| {
                                    (f.face.whatami == WhatAmI::Peer && f.subs.is_some())
                                        .then_some(f.face.zid)
                                })
                                .collect()
                        },
                        clients: s
                            .session_ctxs
                            .values()
                            .filter_map(|f| {
                                (f.face.whatami == WhatAmI::Client && f.subs.is_some())
                                    .then_some(f.face.zid)
                            })
                            .collect(),
                    },
                )
            })
            .collect()
    }

    fn compute_data_route(
        &self,
        tables: &Tables,
        expr: &mut RoutingExpr,
        source: NodeId,
        source_type: WhatAmI,
    ) -> Arc<Route> {
        #[inline]
        fn insert_faces_for_subs(
            route: &mut Route,
            expr: &RoutingExpr,
            tables: &Tables,
            net: &Network,
            source: NodeId,
            subs: &HashSet<ZenohIdProto>,
        ) {
            if net.trees.len() > source as usize {
                for sub in subs {
                    if let Some(sub_idx) = net.get_idx(sub) {
                        if net.trees[source as usize].directions.len() > sub_idx.index() {
                            if let Some(direction) =
                                net.trees[source as usize].directions[sub_idx.index()]
                            {
                                tracing::trace!(
                                    "The sub_idx: {:?}, direction: {:?}",
                                    sub_idx,
                                    direction
                                );
                                if net.graph.contains_node(direction) {
                                    if let Some(face) = tables.get_face(&net.graph[direction].zid) {
                                        route.entry(face.id).or_insert_with(|| {
                                            let key_expr = Resource::get_best_key(
                                                expr.prefix,
                                                expr.suffix,
                                                face.id,
                                            );
                                            (face.clone(), key_expr.to_owned(), source)
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                tracing::trace!("Tree for node sid:{} not yet ready", source);
            }
        }

        let mut route = HashMap::new();
        let key_expr = expr.full_expr();
        if key_expr.ends_with('/') {
            return Arc::new(route);
        }
        tracing::trace!(
            "compute_data_route({}, {:?}, {:?})",
            key_expr,
            source,
            source_type
        );
        let key_expr = match OwnedKeyExpr::try_from(key_expr) {
            Ok(ke) => ke,
            Err(e) => {
                tracing::warn!("Invalid KE reached the system: {}", e);
                return Arc::new(route);
            }
        };
        let res = Resource::get_resource(expr.prefix, expr.suffix);
        let matches = res
            .as_ref()
            .and_then(|res| res.context.as_ref())
            .map(|ctx| Cow::from(&ctx.matches))
            .unwrap_or_else(|| Cow::from(Resource::get_matches(tables, &key_expr)));

        let master = !hat!(tables).full_net(WhatAmI::Peer)
            || *hat!(tables).elect_router(&tables.zid, &key_expr, hat!(tables).shared_nodes.iter())
                == tables.zid;

        for mres in matches.iter() {
            let mres = mres.upgrade().unwrap();

            if master || source_type == WhatAmI::Router {
                let net = hat!(tables).routers_net.as_ref().unwrap();
                let router_source = match source_type {
                    WhatAmI::Router => source,
                    _ => net.idx.index() as NodeId,
                };
                tracing::trace!(
                    "insert_faces_for_subs(source: {}, subs: {:?} )",
                    router_source,
                    &res_hat!(mres).router_subs
                );
                insert_faces_for_subs(
                    &mut route,
                    expr,
                    tables,
                    net,
                    router_source,
                    &res_hat!(mres).router_subs,
                );
            }

            if (master || source_type != WhatAmI::Router) && hat!(tables).full_net(WhatAmI::Peer) {
                let net = hat!(tables).linkstatepeers_net.as_ref().unwrap();
                let peer_source = match source_type {
                    WhatAmI::Peer => source,
                    _ => net.idx.index() as NodeId,
                };
                tracing::trace!(
                    "insert_faces_for_subs(source: {}, subs: {:?} )",
                    peer_source,
                    &res_hat!(mres).linkstatepeer_subs,
                );
                insert_faces_for_subs(
                    &mut route,
                    expr,
                    tables,
                    net,
                    peer_source,
                    &res_hat!(mres).linkstatepeer_subs,
                );
            }

            if master || source_type == WhatAmI::Router {
                for (sid, context) in &mres.session_ctxs {
                    if context.subs.is_some() && context.face.whatami != WhatAmI::Router {
                        route.entry(*sid).or_insert_with(|| {
                            let key_expr = Resource::get_best_key(expr.prefix, expr.suffix, *sid);
                            (context.face.clone(), key_expr.to_owned(), NodeId::default())
                        });
                    }
                }
            }
        }
        for mcast_group in &tables.mcast_groups {
            route.insert(
                mcast_group.id,
                (
                    mcast_group.clone(),
                    expr.full_expr().to_string().into(),
                    NodeId::default(),
                ),
            );
        }
        Arc::new(route)
    }

    fn get_data_routes_entries(&self, tables: &Tables) -> RoutesIndexes {
        get_routes_entries(tables)
    }

    #[zenoh_macros::unstable]
    fn get_matching_subscriptions(
        &self,
        tables: &Tables,
        key_expr: &KeyExpr<'_>,
    ) -> HashMap<usize, Arc<FaceState>> {
        #[inline]
        fn insert_faces_for_subs(
            route: &mut HashMap<usize, Arc<FaceState>>,
            tables: &Tables,
            net: &Network,
            source: usize,
            subs: &HashSet<ZenohIdProto>,
        ) {
            if net.trees.len() > source {
                for sub in subs {
                    if let Some(sub_idx) = net.get_idx(sub) {
                        if net.trees[source].directions.len() > sub_idx.index() {
                            if let Some(direction) = net.trees[source].directions[sub_idx.index()] {
                                if net.graph.contains_node(direction) {
                                    if let Some(face) = tables.get_face(&net.graph[direction].zid) {
                                        route.entry(face.id).or_insert_with(|| face.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                tracing::trace!("Tree for node sid:{} not yet ready", source);
            }
        }

        let mut matching_subscriptions = HashMap::new();
        if key_expr.ends_with('/') {
            return matching_subscriptions;
        }
        tracing::trace!("get_matching_subscriptions({})", key_expr,);

        let res = Resource::get_resource(&tables.root_res, key_expr);
        let matches = res
            .as_ref()
            .and_then(|res| res.context.as_ref())
            .map(|ctx| Cow::from(&ctx.matches))
            .unwrap_or_else(|| Cow::from(Resource::get_matches(tables, key_expr)));

        let master = !hat!(tables).full_net(WhatAmI::Peer)
            || *hat!(tables).elect_router(&tables.zid, key_expr, hat!(tables).shared_nodes.iter())
                == tables.zid;

        for mres in matches.iter() {
            let mres = mres.upgrade().unwrap();

            if master {
                let net = hat!(tables).routers_net.as_ref().unwrap();
                insert_faces_for_subs(
                    &mut matching_subscriptions,
                    tables,
                    net,
                    net.idx.index(),
                    &res_hat!(mres).router_subs,
                );
            }

            if hat!(tables).full_net(WhatAmI::Peer) {
                let net = hat!(tables).linkstatepeers_net.as_ref().unwrap();
                insert_faces_for_subs(
                    &mut matching_subscriptions,
                    tables,
                    net,
                    net.idx.index(),
                    &res_hat!(mres).linkstatepeer_subs,
                );
            }

            if master {
                for (sid, context) in &mres.session_ctxs {
                    if context.subs.is_some() && context.face.whatami != WhatAmI::Router {
                        matching_subscriptions
                            .entry(*sid)
                            .or_insert_with(|| context.face.clone());
                    }
                }
            }
        }
        matching_subscriptions
    }
}

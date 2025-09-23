// examples/examples/check_id_type.rs

use clap::Parser;
use zenoh::{key_expr::KeyExpr, Config};
use zenoh_examples::CommonArgs;
// use zenoh::prelude::r#async::*;
use zenoh::Session;
use zenoh_protocol::core::ZenohIdProto; // Import ZenohIdProto

#[tokio::main]
async fn main() {
    zenoh::init_log_from_env_or("error"); // Initialize logging

    println!("Opening session...");
    let config = zenoh::config::Config::default();
    let session = zenoh::open(config).await.unwrap();
    println!("Session opened successfully.");

    let zenoh_id = session.zid();
    println!("--- ZenohId Information ---");
    println!("Type of session.zid(): {}", std::any::type_name::<ZenohIdProto>());
    println!("Value of session.zid(): {}", zenoh_id);
    // println!("Length of ZenohId (bytes): {}", zenoh_id.as_bytes().len());

    // Convert ZenohId to ZenohIdProto
    let zenoh_id_proto: ZenohIdProto = zenoh_id.into();
    println!("\n--- ZenohIdProto Information ---");
    println!("Type of ZenohIdProto: {}", std::any::type_name::<ZenohIdProto>());
    println!("Value of ZenohIdProto: {:?}", zenoh_id_proto);

    // Close the session
    session.close().await.unwrap();
    println!("\nSession closed.");
}


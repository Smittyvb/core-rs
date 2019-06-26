#![feature(type_alias_enum_variants)]

#[macro_use]
extern crate json;
#[macro_use]
extern crate log;
extern crate nimiq_block as block;
extern crate nimiq_block_albatross as block_albatross;
extern crate nimiq_block_base as block_base;
extern crate nimiq_block_production as block_production;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_blockchain_albatross as blockchain_albatross;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_consensus as consensus;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_mempool as mempool;
extern crate nimiq_network as network;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;
extern crate nimiq_lib as lib;

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::future::Future;
use hyper::Server;
use json::JsonValue;
use parking_lot::RwLock;

use consensus::{Consensus, ConsensusEvent, ConsensusProtocol, AlbatrossConsensusProtocol};
use lib::block_producer::BlockProducer;

use crate::error::Error;

pub mod jsonrpc;
pub mod error;
pub mod common;
pub mod nimiq;
pub mod albatross;

pub use common::RpcHandler;

fn rpc_not_implemented<T>() -> Result<T, JsonValue> {
    Err(object!{"message" => "Not implemented"})
}


#[derive(Debug, Clone)]
pub struct JsonRpcConfig {
    pub credentials: Option<Credentials>,
    pub methods: HashSet<String>,
    pub allowip: (),
    pub corsdomain: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    pub fn new(username: &str, password: &str) -> Credentials {
        Credentials { username: String::from(username), password: String::from(password) }
    }

    pub fn check(&self, username: &str, password: &str) -> bool {
        self.username == username && self.password == password
    }
}

pub struct JsonRpcServerState {
    consensus_state: &'static str,
}

pub fn rpc_server<P, PH, BP>(consensus: Arc<Consensus<P>>, block_producer: Arc<BP>, ip: IpAddr, port: u16, config: JsonRpcConfig) -> Result<Box<dyn Future<Item=(), Error=()> + Send + Sync>, Error>
    where P: ConsensusProtocol + 'static,
    PH: AbstractRpcHandler<P, BP> + 'static,
    BP: BlockProducer<P> + 'static,
{
    let state = Arc::new(RwLock::new(JsonRpcServerState {
        consensus_state: "syncing",
    }));

    // Register for consensus events.
    {
        trace!("Register listener for consensus");
        let state = Arc::downgrade(&state);
        consensus.notifier.write().register(move |e: &ConsensusEvent| {
            trace!("Consensus Event: {:?}", e);
            if let Some(state) = state.upgrade() {
                match e {
                    ConsensusEvent::Established => { state.write().consensus_state = "established" },
                    ConsensusEvent::Lost => { state.write().consensus_state = "lost" },
                    ConsensusEvent::Syncing => { state.write().consensus_state = "syncing" },
                    _ => ()
                }
            }
        });
    }

    let config = Arc::new(config);
    Ok(Box::new(Server::try_bind(&SocketAddr::new(ip, port))?
        .serve(move || {
            jsonrpc::Service::new(PH::new(Arc::clone(&consensus), Arc::clone(&block_producer), Arc::clone(&state), Arc::clone(&config)))
        })
        .map_err(|e| error!("RPC server failed: {}", e)))) // as Box<dyn Future<Item=(), Error=()> + Send + Sync>
}

pub trait AbstractRpcHandler<P, BP>: jsonrpc::Handler
    where P: ConsensusProtocol + 'static,
    BP: BlockProducer<P> + 'static
{
    fn new(consensus: Arc<Consensus<P>>, block_producer: Arc<BP>, state: Arc<RwLock<JsonRpcServerState>>, config: Arc<JsonRpcConfig>) -> Self;
}

pub struct DummyRpcHandler();

impl<BP> AbstractRpcHandler<AlbatrossConsensusProtocol, BP> for DummyRpcHandler
    where BP: BlockProducer<AlbatrossConsensusProtocol> + 'static
{
    fn new(_consensus: Arc<Consensus<AlbatrossConsensusProtocol>>, _block_producer: Arc<BP>, _state: Arc<RwLock<JsonRpcServerState>>, _config: Arc<JsonRpcConfig>) -> Self {
        Self()
    }
}

impl jsonrpc::Handler for DummyRpcHandler {
    fn get_method(&self, _name: &str) -> Option<fn(&Self, Vec<JsonValue>) -> Result<JsonValue, JsonValue>> {
        None
    }
}

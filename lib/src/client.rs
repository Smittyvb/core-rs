use std::sync::Arc;
use std::env;

use futures::{Async, Future, Poll};

use consensus::consensus::Consensus;
use database::Environment;
use network::network::Network;
use network::network_config::{NetworkConfig, ReverseProxyConfig, Seed, PeerKeyStore};
use network_primitives::address::NetAddress;
use primitives::networks::NetworkId;
use network_primitives::protocol::Protocol;
use mempool::MempoolConfig;

use crate::error::ClientError;



lazy_static! {
    pub static ref DEFAULT_USER_AGENT: String = format!("core-rs/{} (native; {} {})", env!("CARGO_PKG_VERSION"), env::consts::OS, env::consts::ARCH);
}


/// Builder for consensus and client
pub struct ClientBuilder {
    protocol: Protocol,
    environment: &'static Environment,
    peer_key_store: PeerKeyStore,
    hostname: Option<String>,
    port: Option<u16>,
    user_agent: String,
    network_id: NetworkId,
    reverse_proxy_config: Option<ReverseProxyConfig>,
    additional_seeds: Vec<Seed>,
    identity_file: Option<String>,
    identity_password: Option<String>,
    mempool_config: Option<MempoolConfig>
}

impl ClientBuilder {
    pub fn new(protocol: Protocol, environment: &'static Environment, peer_key_store: PeerKeyStore) -> Self {
        ClientBuilder {
            protocol,
            environment,
            peer_key_store,
            hostname: None,
            port: None,
            user_agent: DEFAULT_USER_AGENT.clone(),
            network_id: NetworkId::Main,
            reverse_proxy_config: None,
            additional_seeds: Vec::new(),
            identity_file: None,
            identity_password: None,
            mempool_config: None
        }
    }

    pub fn with_network_id(&mut self, network_id: NetworkId) -> &mut Self {
        self.network_id = network_id;
        self
    }

    pub fn with_hostname(&mut self, hostname: &str) -> &mut Self {
        self.hostname = Some(String::from(hostname));
        self
    }

    pub fn with_port(&mut self, port: u16) -> &mut Self {
        self.port = Some(port);
        self
    }

    pub fn with_user_agent(&mut self, user_agent: &str) {
        self.user_agent = String::from(user_agent);
    }

    pub fn with_seeds(&mut self, seeds: Vec<Seed>) -> &mut Self {
        self.additional_seeds.extend(seeds);
        self
    }

    pub fn with_reverse_proxy(&mut self, port: u16, address: NetAddress, header: String, with_tls_termination: bool) -> &mut Self {
        self.reverse_proxy_config = Some(ReverseProxyConfig{port, address, header, with_tls_termination});
        self
    }

    pub fn with_tls_identity(&mut self, identity_file: &str, identity_password: &str) -> &mut Self {
        self.identity_file = Some(String::from(identity_file));
        self.identity_password = Some(String::from(identity_password));
        self
    }

    pub fn with_mempool_config(&mut self, mempool_config: MempoolConfig) -> &mut Self {
        self.mempool_config = Some(mempool_config);
        self
    }

    pub fn build_client(self) -> Result<ClientInitializeFuture, ClientError> {
        let consensus = self.build_consensus()?;
        Ok(ClientInitializeFuture {
            consensus: consensus.clone(),
            initialized: false
        })
    }

    pub fn build_consensus(self) -> Result<Arc<Consensus>, ClientError> {
        // deconstruct builder
        let Self {
            environment,
            peer_key_store,
            network_id,
            mempool_config,
            protocol,
            hostname,
            port,
            reverse_proxy_config,
            identity_file,
            identity_password,
            user_agent,
            additional_seeds,
        } = self;

        // build network config
        let mut network_config = match protocol {
            Protocol::Dumb => {
                // NOTE: Just ignore hostname and port
                //if self.hostname.is_some() { return Err(ClientError::UnexpectedHostname) }
                //if self.port.is_some() { return Err(ClientError::UnexpectedPort) }
                NetworkConfig::new_dumb_network_config()
            },
            Protocol::Ws => {
                let hostname = hostname.ok_or(ClientError::MissingHostname)?;
                let port = port.unwrap_or(self.protocol.default_port()
                    .ok_or(ClientError::MissingPort)?);
                NetworkConfig::new_ws_network_config(hostname, port, reverse_proxy_config)
            },
            Protocol::Wss => {
                let hostname = hostname.ok_or(ClientError::MissingHostname)?;
                let port = port.unwrap_or(protocol.default_port()
                    .ok_or(ClientError::MissingPort)?);
                let identity_file = identity_file.ok_or(ClientError::MissingIdentityFile)?;
                let identity_password = identity_password.ok_or(ClientError::MissingIdentityFile)?;
                NetworkConfig::new_wss_network_config(hostname, port, identity_file, identity_password, reverse_proxy_config)
            },
            Protocol::Rtc => {
                return Err(ClientError::RtcNotImplemented)
            },
        };
        network_config.set_user_agent(user_agent);
        network_config.set_additional_seeds(additional_seeds);
        network_config.init_persistent(&peer_key_store)?;

        let mempool_config = mempool_config.unwrap_or_else(MempoolConfig::default);
        Ok(Consensus::new(environment, network_id, network_config, mempool_config)?)
    }
}

/// A trait representing a Client that may be uninitialized, initialized or connected
/// NOTE: The futures and imtermediate states implement this
pub trait Client {
    fn initialized(&self) -> bool;
    fn connected(&self) -> bool;
    fn consensus(&self) -> Arc<Consensus>;

    fn network(&self) -> Arc<Network> {
        Arc::clone(&self.consensus().network)
    }
}


/// Future that eventually returns a InitializedClient
pub struct ClientInitializeFuture {
    consensus: Arc<Consensus>,
    initialized: bool
}

impl Future for ClientInitializeFuture {
    type Item = InitializedClient;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<InitializedClient, ClientError> {
        // NOTE: This is practically Future::fuse, but this way the types are cleaner
        if !self.initialized {
            self.network().initialize().map_err(ClientError::NetworkError)?;
            self.initialized = true;
            Ok(Async::Ready(InitializedClient {
                consensus: Arc::clone(&self.consensus),
            }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}

impl Client for ClientInitializeFuture {
    fn initialized(&self) -> bool {
        self.initialized
    }

    fn connected(&self) -> bool {
        false
    }

    fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.consensus)
    }
}


/// The initialized client
pub struct InitializedClient {
    consensus: Arc<Consensus>,
}

impl InitializedClient {
    pub fn connect(&self) -> ClientConnectFuture {
        ClientConnectFuture { consensus: Arc::clone(&self.consensus), connected: false }
    }
}

impl Client for InitializedClient {
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        false
    }

    fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.consensus)
    }
}


/// Future that eventually returns a ConnectedClient
pub struct ClientConnectFuture {
    consensus: Arc<Consensus>,
    connected: bool
}

impl Future for ClientConnectFuture {
    type Item = ConnectedClient;
    type Error = ClientError;

    fn poll(&mut self) -> Poll<ConnectedClient, ClientError> {
        if !self.connected {
            self.network().connect().map_err(ClientError::NetworkError)?;
            self.connected = true;
            Ok(Async::Ready(ConnectedClient { consensus: Arc::clone(&self.consensus) }))
        }
        else {
            Ok(Async::NotReady)
        }
    }
}

impl Client for ClientConnectFuture {
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        self.connected
    }

    fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.consensus)
    }
}


/// The connected client
pub struct ConnectedClient {
    consensus: Arc<Consensus>
}

impl ConnectedClient {
    pub fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.consensus)
    }
}

impl Client for ConnectedClient {
    fn initialized(&self) -> bool {
        true
    }

    fn connected(&self) -> bool {
        true
    }

    fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.consensus)
    }
}

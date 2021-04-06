#![deny(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications
)]

//! A simple networking plugin for Bevy

mod client;
mod error;
mod network_message;
mod server;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bevy::{prelude::*, utils::Uuid};
pub use client::{AppNetworkClientMessage, NetworkClient};
use crossbeam_channel::{unbounded, Receiver, Sender};
use derive_more::{Deref, Display};
use error::NetworkError;
pub use network_message::{ClientMessage, NetworkMessage, ServerMessage};
use serde::{Deserialize, Serialize};
pub use server::{AppNetworkServerMessage, NetworkServer};

struct SyncChannel<T> {
    pub(crate) sender: Sender<T>,
    pub(crate) receiver: Receiver<T>,
}

impl<T> SyncChannel<T> {
    fn new() -> Self {
        let (sender, receiver) = unbounded();

        SyncChannel { sender, receiver }
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy, Display, Debug)]
#[display(fmt = "Connection from {} with ID={}", addr, uuid)]
/// A [`ConnectionId`] denotes a single connection
///
/// Use [`ConnectionId::is_server`] whether it is a connection to a server
/// or another. In most client/server applications this is not required as there
/// is no ambiguity.
pub struct ConnectionId {
    uuid: Uuid,
    addr: SocketAddr,
}

impl ConnectionId {
    /// Get the address associated to this connection id
    /// 
    /// This contains the IP/Port information
    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    pub(crate) fn server(addr: Option<SocketAddr>) -> ConnectionId {
        ConnectionId {
            uuid: Uuid::nil(),
            addr: addr.unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)),
        }
    }

    /// Check whether this [`ConnectionId`] is a server
    pub fn is_server(&self) -> bool {
        self.uuid == Uuid::nil()
    }
}

#[derive(Serialize, Deserialize)]
/// [`NetworkPacket`]s are untyped packets to be sent over the wire
struct NetworkPacket {
    kind: String,
    data: Box<dyn NetworkMessage>,
}

impl std::fmt::Debug for NetworkPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkPacket")
            .field("kind", &self.kind)
            .finish()
    }
}

/// A network event originating from a [`NetworkServer`]
#[derive(Debug)]
pub enum ServerNetworkEvent {
    /// A new client has connected
    Connected(ConnectionId),
    /// A client has disconnected
    Disconnected(ConnectionId),
    /// An error occured while trying to do a network operation
    Error(NetworkError),
}

#[derive(Debug)]
/// A network event originating from a [`NetworkClient`]
pub enum ClientNetworkEvent {
    /// Connected to a server
    Connected,
    /// Disconnected from a server
    Disconnected,
    /// An error occured while trying to do a network operation
    Error(NetworkError),
}

#[derive(Debug, Deref)]
/// [`NetworkData`] is what is sent over the bevy event system
/// 
/// Please check the root documentation how to up everything
pub struct NetworkData<T> {
    source: ConnectionId,
    #[deref]
    inner: T,
}

impl<T> NetworkData<T> {
    pub(crate) fn new(source: ConnectionId, inner: T) -> Self {
        Self { source, inner }
    }

    /// The source of this network data
    pub fn source(&self) -> ConnectionId {
        self.source
    }

    /// Get the inner data out of it
    pub fn into_inner(self) -> T {
        self.inner
    }
}

#[derive(Clone, Debug)]
#[allow(missing_copy_implementations)]
/// Settings to configure the network, both client and server
pub struct NetworkSettings {
    
    /// Maximum packet size in bytes. If a client ever exceeds this size, they will be disconnected
    ///
    /// ## Default
    /// The default is set to 10MiB
    pub max_packet_length: usize,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        NetworkSettings {
            max_packet_length: 10 * 1024 * 1024,
        }
    }
}

#[derive(Default, Copy, Clone, Debug)]
/// The plugin to add to your bevy [`AppBuilder`](bevy::prelude::AppBuilder) when you want
/// to instantiate a server
pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.insert_resource(server::NetworkServer::new());
        app.add_event::<ServerNetworkEvent>();
        app.init_resource::<NetworkSettings>();
        app.add_system_to_stage(
            CoreStage::PreUpdate,
            server::handle_new_incoming_connections.system(),
        );
    }
}

#[derive(Default, Copy, Clone, Debug)]
/// The plugin to add to your bevy [`AppBuilder`](bevy::prelude::AppBuilder) when you want
/// to instantiate a client
pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.insert_resource(client::NetworkClient::new());
        app.add_event::<ClientNetworkEvent>();
        app.init_resource::<NetworkSettings>();
        app.add_system_to_stage(
            CoreStage::PreUpdate,
            client::send_client_network_events.system(),
        );
    }
}

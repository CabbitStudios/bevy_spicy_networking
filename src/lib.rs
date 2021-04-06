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
pub struct ConnectionId {
    uuid: Uuid,
    addr: SocketAddr,
}

impl ConnectionId {
    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    pub(crate) fn server() -> ConnectionId {
        ConnectionId {
            uuid: Uuid::nil(),
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NetworkPacket {
    kind: String,
    data: Box<dyn NetworkMessage>,
}

impl std::fmt::Debug for NetworkPacket {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        f.debug_struct("NetworkPacket")
            .field("kind", &self.kind)
            .finish()
    }
}

pub enum ServerNetworkEvent {
    Connected(ConnectionId),
    Disconnected(ConnectionId),
    Error(NetworkError),
}

pub enum ClientNetworkEvent {
    Connected,
    Disconnected,
    Error(NetworkError),
}

#[derive(Debug, Deref)]
pub struct NetworkData<T> {
    source: ConnectionId,
    #[deref]
    inner: T,
}

impl<T> NetworkData<T> {
    pub fn new(
        source: ConnectionId,
        inner: T,
    ) -> Self {
        Self { source, inner }
    }

    pub fn source(&self) -> ConnectionId {
        self.source
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

#[derive(Clone)]
pub struct NetworkSettings {
    pub max_packet_length: usize,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        NetworkSettings {
            max_packet_length: 10 * 1024 * 1024,
        }
    }
}

#[derive(Default)]
pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(
        &self,
        app: &mut AppBuilder,
    ) {
        app.insert_resource(server::NetworkServer::new());
        app.add_event::<ServerNetworkEvent>();
        app.init_resource::<NetworkSettings>();
        app.add_system_to_stage(
            CoreStage::PreUpdate,
            server::handle_new_incoming_connections.system(),
        );
    }
}

#[derive(Default)]
pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(
        &self,
        app: &mut AppBuilder,
    ) {
        app.insert_resource(client::NetworkClient::new());
        app.add_event::<ClientNetworkEvent>();
        app.init_resource::<NetworkSettings>();
        app.add_system_to_stage(
            CoreStage::PreUpdate,
            client::send_client_network_events.system(),
        );
    }
}

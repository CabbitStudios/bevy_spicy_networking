use std::{net::SocketAddr, sync::Arc, marker::PhantomData};

use bevy::{prelude::*, utils::Uuid};
use dashmap::DashMap;
use derive_more::Display;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    runtime::Runtime,
    sync::mpsc::{unbounded_channel, UnboundedSender, UnboundedReceiver},
    task::JoinHandle,
};
use async_trait::async_trait;

use crate::{
    error::NetworkError,
    network_message::{ClientMessage, ServerMessage},
    ConnectionId, NetworkData, NetworkPacket, ServerNetworkEvent, SyncChannel, AsyncChannel,
};

/// The servers view of a client.
pub struct ClientConnection {
    id: ConnectionId,
    receive_task: JoinHandle<()>,
    map_receive_task: JoinHandle<()>,
    send_task: JoinHandle<()>,
    send_message: UnboundedSender<NetworkPacket>,
    //addr: SocketAddr,
}

impl ClientConnection {

    /// Close the given connection to a client.
    pub fn stop(self) {
        self.receive_task.abort();
        self.send_task.abort();
    }
}

impl std::fmt::Debug for ClientConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConnection")
            .field("id", &self.id)
            //.field("addr", &self.addr)
            .finish()
    }
}

/// A trait used by [`NetworkServer`] to drive a server, this is responsible
/// for generating the futures that carryout the underlying server logic.
#[async_trait]
pub trait NetworkServerProvider: 'static + Send + Sync{
    /// This is to configure particular protocols
    type NetworkSettings: Send + Sync + Clone;

    /// The type that acts as a combined sender and reciever for a client.
    /// This type needs to be able to be split.
    type Socket: Send;

    /// The read half of the given socket type.
    type ReadHalf: Send;
    
    /// The write half of the given socket type.
    type WriteHalf: Send;
    
    /// This will be spawned as a background operation to continuously add new connections.
    async fn accept_loop(network_settings: Self::NetworkSettings, new_connections: UnboundedSender<Self::Socket>, errors: UnboundedSender<NetworkError>);

    /// Recieves messages from the client, forwards them to Spicy via a sender.
    async fn recv_loop(read_half: Self::ReadHalf, messages: UnboundedSender<NetworkPacket>, settings: Self::NetworkSettings);
    
    /// Sends messages to the client, receives packages from Spicy via receiver.
    async fn send_loop(write_half: Self::WriteHalf, messages: UnboundedReceiver<NetworkPacket>, settings: Self::NetworkSettings);

    /// Split the socket into a read and write half, so that the two actions
    /// can be handled concurrently.
    fn split(combined: Self::Socket) -> (Self::ReadHalf, Self::WriteHalf);
}

/// An instance of a [`NetworkServer`] is used to listen for new client connections
/// using [`NetworkServer::listen`]
pub struct NetworkServer<NSP: NetworkServerProvider> {
    runtime: Runtime,
    recv_message_map: Arc<DashMap<&'static str, Vec<(ConnectionId, Vec<u8>)>>>,
    established_connections: Arc<DashMap<ConnectionId, ClientConnection>>,
    new_connections: AsyncChannel<NSP::Socket>,
    disconnected_connections: AsyncChannel<ConnectionId>,
    error_channel: AsyncChannel<NetworkError>,
    server_handle: Option<JoinHandle<()>>,
    provider: PhantomData<NSP>,
}

impl<NSP: NetworkServerProvider> std::fmt::Debug for NetworkServer<NSP> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NetworkServer [{} Connected Clients]",
            self.established_connections.len()
        )
    }
}

impl<NSP: NetworkServerProvider> NetworkServer<NSP> {
    pub(crate) fn new(provider: NSP) -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Could not build tokio runtime"),
            recv_message_map: Arc::new(DashMap::new()),
            established_connections: Arc::new(DashMap::new()),
            new_connections: AsyncChannel::new(),
            disconnected_connections: AsyncChannel::new(),
            error_channel: AsyncChannel::new(),
            server_handle: None,
            provider: PhantomData,
        }
    }

    /// Start listening for new clients
    ///
    /// ## Note
    /// If you are already listening for new connections, then this will disconnect existing connections first
    pub fn listen(
        &mut self,
        network_settings: &NSP::NetworkSettings,
    ) -> Result<(), NetworkError> {
        self.stop();

        let new_connections = self.new_connections.sender.clone();
        let error_sender = self.error_channel.sender.clone();

        
        let listen_loop = NSP::accept_loop(network_settings.clone(), new_connections, error_sender);

        trace!("Started listening");

        self.server_handle = Some(self.runtime.spawn(listen_loop));

        Ok(())
    }

    /// Send a message to a specific client
    pub fn send_message<T: ClientMessage>(
        &self,
        client_id: ConnectionId,
        message: T,
    ) -> Result<(), NetworkError> {
        let connection = match self.established_connections.get(&client_id) {
            Some(conn) => conn,
            None => return Err(NetworkError::ConnectionNotFound(client_id)),
        };

        let packet = NetworkPacket {
            kind: String::from(T::NAME),
            data: bincode::serialize(&message).unwrap(),
        };

        match connection.send_message.send(packet) {
            Ok(_) => (),
            Err(err) => {
                error!("There was an error sending a packet: {}", err);
                return Err(NetworkError::ChannelClosed(client_id));
            }
        }

        Ok(())
    }

    /// Broadcast a message to all connected clients
    pub fn broadcast<T: ClientMessage + Clone>(&self, message: T) {
        for connection in self.established_connections.iter() {
            let serialized_message = bincode::serialize(&message).unwrap();
            let packet = NetworkPacket {
                kind: String::from(T::NAME),
                data: serialized_message,
            };

            match connection.send_message.send(packet) {
                Ok(_) => (),
                Err(err) => {
                    warn!("Could not send to client because: {}", err);
                }
            }
        }
    }

    /// Disconnect all clients and stop listening for new ones
    ///
    /// ## Notes
    /// This operation is idempotent and will do nothing if you are not actively listening
    pub fn stop(&mut self) {
        if let Some(conn) = self.server_handle.take() {
            conn.abort();
            for conn in self.established_connections.iter() {
                let _ = self.disconnected_connections.sender.send(*conn.key());
            }
            self.established_connections.clear();
            self.recv_message_map.clear();

            while let Ok(_) = self.new_connections.receiver.try_recv(){}
        }
    }

    /// Disconnect a specific client
    pub fn disconnect(&self, conn_id: ConnectionId) -> Result<(), NetworkError> {
        let connection = if let Some(conn) = self.established_connections.remove(&conn_id) {
            conn
        } else {
            return Err(NetworkError::ConnectionNotFound(conn_id));
        };

        connection.1.stop();

        Ok(())
    }
}

pub(crate) fn handle_new_incoming_connections<NSP: NetworkServerProvider>(
    mut server: ResMut<NetworkServer<NSP>>,
    network_settings: Res<NSP::NetworkSettings>,
    mut network_events: EventWriter<ServerNetworkEvent>,
) {
    while let Ok(new_conn) = server.new_connections.receiver.try_recv() {

            let conn_id = ConnectionId {
                uuid: Uuid::new_v4(),
            };

            let (read_half, write_half) = NSP::split(new_conn);
            let recv_message_map = server.recv_message_map.clone();
            let read_network_settings = network_settings.clone();
            let write_network_settings = network_settings.clone();
            let disconnected_connections = server.disconnected_connections.sender.clone();

            let (outgoing_tx, outgoing_rx) = unbounded_channel();
            let (incoming_tx, mut incoming_rx) = unbounded_channel();

            server.established_connections.insert(
                conn_id,
                ClientConnection {
                    id: conn_id,
                    receive_task: server.runtime.spawn(async move {
                        trace!("Starting listen task for {}", conn_id);
                        NSP::recv_loop(read_half, incoming_tx, read_network_settings).await;

                        match disconnected_connections.send(conn_id) {
                            Ok(_) => (),
                            Err(_) => {
                                error!("Could not send disconnected event, because channel is disconnected");
                            }
                        }
                    }),
                    map_receive_task: server.runtime.spawn(async move{
                        while let Some(packet) = incoming_rx.recv().await{
                            match recv_message_map.get_mut(&packet.kind[..]) {
                                Some(mut packets) => packets.push((conn_id, packet.data)),
                                None => {
                                    error!("Could not find existing entries for message kinds: {:?}", packet);
                                }
                            }
                        }
                    }),
                    send_task: server.runtime.spawn(async move {
                        trace!("Starting send task for {}", conn_id);
                        NSP::send_loop(write_half, outgoing_rx, write_network_settings).await;
                    }),
                    send_message: outgoing_tx,
                    //addr: new_conn.addr,
                },
            );

            network_events.send(ServerNetworkEvent::Connected(conn_id));
        
    }

    while let Ok(disconnected_connection) = server.disconnected_connections.receiver.try_recv() {
        server
            .established_connections
            .remove(&disconnected_connection);
        network_events.send(ServerNetworkEvent::Disconnected(disconnected_connection));
    }
}

/// A utility trait on [`App`] to easily register [`ServerMessage`]s
pub trait AppNetworkServerMessage {
    /// Register a server message type
    ///
    /// ## Details
    /// This will:
    /// - Add a new event type of [`NetworkData<T>`]
    /// - Register the type for transformation over the wire
    /// - Internal bookkeeping
    fn listen_for_server_message<T: ServerMessage, NSP: NetworkServerProvider>(&mut self) -> &mut Self;
}

impl AppNetworkServerMessage for App {
    fn listen_for_server_message<T: ServerMessage, NSP: NetworkServerProvider>(&mut self) -> &mut Self {
        let server = self.world.get_resource::<NetworkServer<NSP>>().expect("Could not find `NetworkServer`. Be sure to include the `ServerPlugin` before listening for server messages.");

        debug!("Registered a new ServerMessage: {}", T::NAME);

        assert!(
            !server.recv_message_map.contains_key(T::NAME),
            "Duplicate registration of ServerMessage: {}",
            T::NAME
        );
        server.recv_message_map.insert(T::NAME, Vec::new());
        self.add_event::<NetworkData<T>>();
        self.add_system_to_stage(CoreStage::PreUpdate, register_server_message::<T, NSP>)
    }
}

fn register_server_message<T, NSP: NetworkServerProvider>(
    net_res: ResMut<NetworkServer<NSP>>,
    mut events: EventWriter<NetworkData<T>>,
) where
    T: ServerMessage,
{
    let mut messages = match net_res.recv_message_map.get_mut(T::NAME) {
        Some(messages) => messages,
        None => return,
    };

    events.send_batch(
        messages
            .drain(..)
            .filter_map(|(source, msg)| bincode::deserialize::<T>(&msg).ok().map(|inner| NetworkData{source, inner})),
    );
}

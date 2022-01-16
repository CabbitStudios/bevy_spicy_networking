use std::{net::SocketAddr, sync::Arc};

use bevy::{prelude::*, utils::Uuid};
use dashmap::DashMap;
use derive_more::Display;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    runtime::Runtime,
    sync::mpsc::{unbounded_channel, UnboundedSender},
    task::JoinHandle,
};
use async_trait::async_trait;

use crate::{
    error::NetworkError,
    network_message::{ClientMessage, NetworkMessage, ServerMessage},
    ConnectionId, NetworkData, NetworkPacket, NetworkSettings, ServerNetworkEvent, SyncChannel,
};

#[derive(Display)]
#[display(fmt = "Incoming Connection")]
struct NewIncomingConnection<NSP: NetworkServerProvider> {
    socket: NSP::Socket,
}

/// The servers view of a client.
pub struct ClientConnection {
    id: ConnectionId,
    receive_task: JoinHandle<()>,
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

    /// This is the type given to the listener to start listening.
    type ListenInfo: Send;

    /// The type that accepts new connections to the server.
    type Listener: Send;

    /// The type that acts as a combined sender and reciever for a client.
    /// This type needs to be able to be split.
    type Socket: Send;

    /// The read half of the given socket type.
    type ReadHalf: Send;
    
    /// The write half of the given socket type.
    type WriteHalf: Send;

    /// The parameters used by the read function that need to be created.
    type ReadParams: Send;

    /// The parameters used by the write function that need to be created.
    type WriteParams: Send;

    /// The error type associated with this provider.
    type ProtocolErrors: Sync + Send + std::fmt::Debug + std::fmt::Display;

    /// Creates a new listener, this listener will be passed to 
    /// [`NetworkServerProvider::listener`] to get new connections.
    async fn listen(listen_info: Self::ListenInfo) -> Result<Self::Listener, Self::ProtocolErrors>;

    /// Recieve a connection.
    async fn accept<'l>(listener: &'l mut Self::Listener) -> Result<Self::Socket, Self::ProtocolErrors>;

    /// Recieve a message from the client.
    async fn read_message<'a>(read_half: &'a mut Self::ReadHalf, settings: &'a Self::NetworkSettings, read_params: &'a mut Self::ReadParams) -> Result<NetworkPacket, Self::ProtocolErrors>;
    
    /// Send a message to the client.
    async fn send_message<'a>(message: &'a NetworkPacket, write_half: &'a mut Self::WriteHalf, settings: &'a Self::NetworkSettings, write_params: &'a mut Self::WriteParams) -> Result<(), Self::ProtocolErrors>;

    /// Create the [`NetworkServerProvider::ReadParams`] that will be used by the read function.
    fn init_read<'a> (settings: &'a Self::NetworkSettings) -> Self::ReadParams; 

    /// Create the [`NetworkServerProvider::WriteParams`] that will be used by the write function.
    fn init_write<'a> (settings: &'a Self::NetworkSettings) -> Self::WriteParams; 

    /// Split the socket into a read and write half, so that the two actions
    /// can be handled concurrently.
    fn split(combined: Self::Socket) -> (Self::ReadHalf, Self::WriteHalf);
}

/// An instance of a [`NetworkServer`] is used to listen for new client connections
/// using [`NetworkServer::listen`]
pub struct NetworkServer<NSP: NetworkServerProvider> {
    runtime: Runtime,
    recv_message_map: Arc<DashMap<&'static str, Vec<(ConnectionId, Box<dyn NetworkMessage>)>>>,
    established_connections: Arc<DashMap<ConnectionId, ClientConnection>>,
    new_connections: SyncChannel<NewIncomingConnection<NSP>>,
    disconnected_connections: SyncChannel<ConnectionId>,
    error_channel: SyncChannel<NetworkError<NSP::ProtocolErrors>>,
    server_handle: Option<JoinHandle<()>>,
    provider: NSP,
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
            new_connections: SyncChannel::new(),
            disconnected_connections: SyncChannel::new(),
            error_channel: SyncChannel::new(),
            server_handle: None,
            provider
        }
    }

    /// Start listening for new clients
    ///
    /// ## Note
    /// If you are already listening for new connections, then this will disconnect existing connections first
    pub fn listen(
        &mut self,
        listen_info: impl Into<NSP::ListenInfo>,
    ) -> Result<(), NetworkError<NSP::ProtocolErrors>> {
        self.stop();

        let new_connections = self.new_connections.sender.clone();
        let error_sender = self.error_channel.sender.clone();
        let listen_info = listen_info.into();

        let listen_loop = async move {
            let mut listener = match NSP::listen(listen_info).await {
                Ok(listener) => listener,
                Err(err) => {
                    match error_sender.send(NetworkError::<NSP::ProtocolErrors>::Provider(err)) {
                        Ok(_) => (),
                        Err(err) => {
                            error!("Could not send listen error: {}", err);
                        }
                    }

                    return;
                }
            };

            let new_connections = new_connections;
            loop {
                let resp = match NSP::accept(&mut listener).await {
                    Ok(socket) => NewIncomingConnection { socket },
                    Err(error) =>  {
                        if let Err(err)  = error_sender.send(NetworkError::Provider(error)){
                            error!("Cannot accept more errors, channel closed: {}", err);
                            break;
                        };
                        continue;
                    },
                };

                if let Err(err) = new_connections.send(resp) {
                    error!("Cannot accept new connections, channel closed: {}", err);
                    break;
                }
            }
        };

        trace!("Started listening");

        self.server_handle = Some(self.runtime.spawn(listen_loop));

        Ok(())
    }

    /// Send a message to a specific client
    pub fn send_message<T: ClientMessage>(
        &self,
        client_id: ConnectionId,
        message: T,
    ) -> Result<(), NetworkError<NSP::ProtocolErrors>> {
        let connection = match self.established_connections.get(&client_id) {
            Some(conn) => conn,
            None => return Err(NetworkError::ConnectionNotFound(client_id)),
        };

        let packet = NetworkPacket {
            kind: String::from(T::NAME),
            data: Box::new(message),
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
            let packet = NetworkPacket {
                kind: String::from(T::NAME),
                data: Box::new(message.clone()),
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

            self.new_connections.receiver.try_iter().for_each(|_| ());
        }
    }

    /// Disconnect a specific client
    pub fn disconnect(&self, conn_id: ConnectionId) -> Result<(), NetworkError<NSP::ProtocolErrors>> {
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
    server: Res<NetworkServer<NSP>>,
    network_settings: Res<NSP::NetworkSettings>,
    mut network_events: EventWriter<ServerNetworkEvent<NSP>>,
) {
    for new_conn in server.new_connections.receiver.try_iter() {

            let conn_id = ConnectionId {
                uuid: Uuid::new_v4(),
                //addr: new_conn.addr,
            };

            let (mut read_half, mut write_half) = NSP::split(new_conn.socket);
            let recv_message_map = server.recv_message_map.clone();
            let read_network_settings = network_settings.clone();
            let write_network_settings = network_settings.clone();
            let disconnected_connections = server.disconnected_connections.sender.clone();

            let (send_message, recv_message) = unbounded_channel();

            server.established_connections.insert(
                conn_id,
                ClientConnection {
                    id: conn_id,
                    receive_task: server.runtime.spawn(async move {
                        let recv_message_map = recv_message_map;
                        let mut read_params = NSP::init_read(&read_network_settings);

                        trace!("Starting listen task for {}", conn_id);
                        loop {

                            let packet = match NSP::read_message(&mut read_half, &read_network_settings, &mut read_params).await {
                                Ok(packet) => packet,
                                Err(err) => {
                                    error!("Failed to decode network packet from [{}]: {}", conn_id, err);
                                    break;
                                }
                            };

                            match recv_message_map.get_mut(&packet.kind[..]) {
                                Some(mut packets) => packets.push((conn_id, packet.data)),
                                None => {
                                    error!("Could not find existing entries for message kinds: {:?}", packet);
                                }
                            }
                        }

                        match disconnected_connections.send(conn_id) {
                            Ok(_) => (),
                            Err(_) => {
                                error!("Could not send disconnected event, because channel is disconnected");
                            }
                        }
                    }),
                    send_task: server.runtime.spawn(async move {
                        let mut recv_message = recv_message;

                        let mut write_half = write_half;

                        let mut write_params = NSP::init_write(&write_network_settings);

                        while let Some(message) = recv_message.recv().await {
                            match NSP::send_message(&message, &mut write_half, &write_network_settings, &mut write_params).await {
                                Ok(_) => (),
                                Err(err) => {
                                    error!("Could not send packet: {:?}: {}", message, err);
                                    return;
                                }
                            }
                        }
                    }),
                    send_message,
                    //addr: new_conn.addr,
                },
            );

            network_events.send(ServerNetworkEvent::Connected(conn_id));
        
    }

    let disconnected_connections = &server.disconnected_connections.receiver;

    for disconnected_connection in disconnected_connections.try_iter() {
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
            .flat_map(|(conn, msg)| msg.downcast().map(|msg| NetworkData::new(conn, *msg))),
    );
}

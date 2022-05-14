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

use crate::{
    error::NetworkError,
    network_message::{ClientMessage, NetworkMessage, ServerMessage},
    ConnectionId, NetworkData, NetworkPacket, NetworkSettings, ServerNetworkEvent, SyncChannel,
};

#[derive(Display)]
#[display(fmt = "Incoming Connection from {}", addr)]
struct NewIncomingConnection {
    socket: TcpStream,
    addr: SocketAddr,
}

pub struct ClientConnection {
    id: ConnectionId,
    receive_task: JoinHandle<()>,
    send_task: JoinHandle<()>,
    send_message: UnboundedSender<NetworkPacket>,
    addr: SocketAddr,
}

impl ClientConnection {
    pub fn stop(self) {
        self.receive_task.abort();
        self.send_task.abort();
    }
}

impl std::fmt::Debug for ClientConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConnection")
            .field("id", &self.id)
            .field("addr", &self.addr)
            .finish()
    }
}

/// An instance of a [`NetworkServer`] is used to listen for new client connections
/// using [`NetworkServer::listen`]
pub struct NetworkServer {
    runtime: Runtime,
    recv_message_map: Arc<DashMap<&'static str, Vec<(ConnectionId, Box<dyn NetworkMessage>)>>>,
    established_connections: Arc<DashMap<ConnectionId, ClientConnection>>,
    new_connections: SyncChannel<Result<NewIncomingConnection, NetworkError>>,
    disconnected_connections: SyncChannel<ConnectionId>,
    error_channel: SyncChannel<NetworkError>,
    server_handle: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for NetworkServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NetworkServer [{} Connected Clients]",
            self.established_connections.len()
        )
    }
}

impl NetworkServer {
    pub(crate) fn new() -> NetworkServer {
        NetworkServer {
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
        }
    }

    /// Start listening for new clients
    ///
    /// ## Note
    /// If you are already listening for new connections, then this will disconnect existing connections first
    pub fn listen(
        &mut self,
        addr: impl ToSocketAddrs + Send + 'static,
    ) -> Result<(), NetworkError> {
        self.stop();

        let new_connections = self.new_connections.sender.clone();
        let error_sender = self.error_channel.sender.clone();

        let listen_loop = async move {
            let listener = match TcpListener::bind(addr).await {
                Ok(listener) => listener,
                Err(err) => {
                    match error_sender.send(NetworkError::Listen(err)) {
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
                let resp = match listener.accept().await {
                    Ok((socket, addr)) => Ok(NewIncomingConnection { socket, addr }),
                    Err(error) => Err(NetworkError::Accept(error)),
                };

                match new_connections.send(resp) {
                    Ok(_) => (),
                    Err(err) => {
                        error!("Cannot accept new connections, channel closed: {}", err);
                        break;
                    }
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
    ) -> Result<(), NetworkError> {
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

pub(crate) fn handle_new_incoming_connections(
    server: Res<NetworkServer>,
    network_settings: Res<NetworkSettings>,
    mut network_events: EventWriter<ServerNetworkEvent>,
) {
    for inc_conn in server.new_connections.receiver.try_iter() {
        match inc_conn {
            Ok(new_conn) => {
                match new_conn.socket.set_nodelay(true) {
                    Ok(_) => (),
                    Err(e) => error!("Could not set nodelay for [{}]: {}", new_conn, e),
                }

                let conn_id = ConnectionId {
                    uuid: Uuid::new_v4(),
                    addr: new_conn.addr,
                };

                let (read_socket, send_socket) = new_conn.socket.into_split();
                let recv_message_map = server.recv_message_map.clone();
                let network_settings = network_settings.clone();
                let disconnected_connections = server.disconnected_connections.sender.clone();

                let (send_message, recv_message) = unbounded_channel();

                server.established_connections.insert(
                    conn_id,
                    ClientConnection {
                        id: conn_id,
                        receive_task: server.runtime.spawn(async move {
                            let recv_message_map = recv_message_map;
                            let network_settings = network_settings;

                            let mut read_socket = read_socket;

                            let mut buffer: Vec<u8> = (0..network_settings.max_packet_length).map(|_| 0).collect();

                            trace!("Starting listen task for {}", conn_id);
                            loop {
                                trace!("Listening for length!");

                                let length = match read_socket.read_u32().await {
                                    Ok(len) => len as usize,
                                    Err(err) => {
                                        // If we get an EOF here, the connection was broken and we simply report a 'disconnected' signal
                                        if err.kind() == std::io::ErrorKind::UnexpectedEof { break }

                                        error!("Encountered error while reading length [{}]: {}", conn_id, err);
                                        break;
                                    }
                                };

                                trace!("Received packet with length: {}", length);

                                if length > network_settings.max_packet_length {
                                    error!("Received too large packet from [{}]: {} > {}", conn_id, length, network_settings.max_packet_length);
                                    break;
                                }


                                match read_socket.read_exact(&mut buffer[..length]).await {
                                    Ok(_) => (),
                                    Err(err) => {
                                        error!("Encountered error while reading stream of length {} [{}]: {}", length, conn_id, err);
                                        break;
                                    }
                                }

                                trace!("Read buffer of length {}", length);

                                let packet: NetworkPacket = match bincode::deserialize(&buffer[..length]) {
                                    Ok(packet) => packet,
                                    Err(err) => {
                                        error!("Failed to decode network packet from [{}]: {}", conn_id, err);
                                        break;
                                    }
                                };

                                trace!("Created a network packet");

                                match recv_message_map.get_mut(&packet.kind[..]) {
                                    Some(mut packets) => packets.push((conn_id, packet.data)),
                                    None => {
                                        error!("Could not find existing entries for message kinds: {:?}", packet);
                                    }
                                }

                                debug!("Received new message of length: {}", length);
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
                            let mut send_socket = send_socket;

                            while let Some(message) = recv_message.recv().await {
                                let encoded = match bincode::serialize(&message) {
                                    Ok(encoded) => encoded,
                                    Err(err) =>  {
                                        error!("Could not encode packet {:?}: {}", message, err);
                                        continue;
                                    }
                                };

                                let len = encoded.len();

                                match send_socket.write_u32(len as u32).await {
                                    Ok(_) => (),
                                    Err(err) => {
                                        error!("Could not send packet length: {:?}: {}", len, err);
                                        return;
                                    }
                                }

                                match send_socket.write_all(&encoded).await {
                                    Ok(_) => (),
                                    Err(err) => {
                                        error!("Could not send packet: {:?}: {}", message, err);
                                        return;
                                    }
                                }
                            }
                        }),
                        send_message,
                        addr: new_conn.addr,
                    },
                );

                network_events.send(ServerNetworkEvent::Connected(conn_id));
            }

            Err(err) => {
                network_events.send(ServerNetworkEvent::Error(err));
            }
        }
    }

    let disconnected_connections = &server.disconnected_connections.receiver;

    for disconnected_connection in disconnected_connections.try_iter() {
        server
            .established_connections
            .remove(&disconnected_connection);
        network_events.send(ServerNetworkEvent::Disconnected(disconnected_connection));
    }
}

/// A utility trait on [`AppBuilder`] to easily register [`ServerMessage`]s
pub trait AppNetworkServerMessage {
    /// Register a server message type
    ///
    /// ## Details
    /// This will:
    /// - Add a new event type of [`NetworkData<T>`]
    /// - Register the type for transformation over the wire
    /// - Internal bookkeeping
    fn listen_for_server_message<T: ServerMessage>(&mut self) -> &mut Self;
}

impl AppNetworkServerMessage for App {
    fn listen_for_server_message<T: ServerMessage>(&mut self) -> &mut Self {
        let server = self.world.get_resource::<NetworkServer>().expect("Could not find `NetworkServer`. Be sure to include the `ServerPlugin` before listening for server messages.");

        debug!("Registered a new ServerMessage: {}", T::NAME);

        assert!(
            !server.recv_message_map.contains_key(T::NAME),
            "Duplicate registration of ServerMessage: {}",
            T::NAME
        );
        server.recv_message_map.insert(T::NAME, Vec::new());
        self.add_event::<NetworkData<T>>();
        self.add_system_to_stage(CoreStage::PreUpdate, register_server_message::<T>)
    }
}

fn register_server_message<T>(
    net_res: ResMut<NetworkServer>,
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

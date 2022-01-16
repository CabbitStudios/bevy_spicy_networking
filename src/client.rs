use std::{net::SocketAddr, sync::Arc};

use bevy::prelude::*;
use dashmap::DashMap;
use derive_more::Display;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, ToSocketAddrs},
    runtime::Runtime,
    sync::mpsc::{unbounded_channel, UnboundedSender},
    task::JoinHandle,
};

use crate::{
    error::NetworkError,
    network_message::{ClientMessage, NetworkMessage, ServerMessage},
    ClientNetworkEvent, ConnectionId, NetworkData, NetworkPacket, NetworkSettings, SyncChannel,
};

#[derive(Display)]
#[display(fmt = "Server connection to {}", peer_addr)]
struct ServerConnection {
    peer_addr: SocketAddr,
    receive_task: JoinHandle<()>,
    send_task: JoinHandle<()>,
    send_message: UnboundedSender<NetworkPacket>,
}

impl ServerConnection {
    fn stop(self) {
        self.receive_task.abort();
        self.send_task.abort();
    }
}

/// An instance of a [`NetworkClient`] is used to connect to a remote server
/// using [`NetworkClient::connect`]
pub struct NetworkClient {
    runtime: Runtime,
    server_connection: Option<ServerConnection>,
    recv_message_map: Arc<DashMap<&'static str, Vec<Box<dyn NetworkMessage>>>>,
    network_events: SyncChannel<ClientNetworkEvent>,
    connection_events: SyncChannel<(TcpStream, SocketAddr, NetworkSettings)>,
}

impl std::fmt::Debug for NetworkClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(conn) = self.server_connection.as_ref() {
            write!(f, "NetworkClient [Connected to {}]", conn.peer_addr)?;
        } else {
            write!(f, "NetworkClient [Not Connected]")?;
        }

        Ok(())
    }
}

impl NetworkClient {
    pub(crate) fn new() -> NetworkClient {
        NetworkClient {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Could not build tokio runtime"),
            server_connection: None,
            recv_message_map: Arc::new(DashMap::new()),
            network_events: SyncChannel::new(),
            connection_events: SyncChannel::new(),
        }
    }

    /// Connect to a remote server
    ///
    /// ## Note
    /// This will disconnect you first from any existing server connections
    pub fn connect(
        &mut self,
        addr: impl ToSocketAddrs + Send + 'static,
        network_settings: NetworkSettings,
    ) {
        debug!("Starting connection");

        self.disconnect();

        let network_error_sender = self.network_events.sender.clone();
        let connection_event_sender = self.connection_events.sender.clone();

        self.runtime.spawn(async move {
            let stream = match TcpStream::connect(addr).await {
                Ok(stream) => stream,
                Err(error) => {
                    match network_error_sender
                        .send(ClientNetworkEvent::Error(NetworkError::Connection(error)))
                    {
                        Ok(_) => (),
                        Err(err) => {
                            error!("Could not send error event: {}", err);
                        }
                    }

                    return;
                }
            };

            let addr = stream
                .peer_addr()
                .expect("Could not fetch peer_addr of existing stream");

            match connection_event_sender.send((stream, addr, network_settings)) {
                Ok(_) => (),
                Err(err) => {
                    error!("Could not initiate connection: {}", err);
                }
            }

            debug!("Connected to: {:?}", addr);
        });
    }

    /// a server
    ///
    /// This operation is idempotent and simply does nothing when you are
    /// not connected to anything
    pub fn disconnect(&mut self) {
        if let Some(conn) = self.server_connection.take() {
            conn.stop();

            let _ = self
                .network_events
                .sender
                .send(ClientNetworkEvent::Disconnected);
        }
    }

    /// Send a message to the connected server, returns `Err(NetworkError::NotConnected)` if
    /// the connection hasn't been established yet
    pub fn send_message<T: ServerMessage>(&self, message: T) -> Result<(), NetworkError> {
        debug!("Sending message to server");
        let server_connection = match self.server_connection.as_ref() {
            Some(server) => server,
            None => return Err(NetworkError::NotConnected),
        };

        let packet = NetworkPacket {
            kind: String::from(T::NAME),
            data: Box::new(message),
        };

        match server_connection.send_message.send(packet) {
            Ok(_) => (),
            Err(err) => {
                error!("Server disconnected: {}", err);
                return Err(NetworkError::NotConnected);
            }
        }

        Ok(())
    }

    /// Returns true if the client has an established connection
    ///
    /// # Note
    /// This may return true even if the connection has already been broken on the server side.
    pub fn is_connected(&self) -> bool {
        self.server_connection.is_some()
    }
}

/// A utility trait on [`App`] to easily register [`ClientMessage`]s
pub trait AppNetworkClientMessage {
    /// Register a client message type
    ///
    /// ## Details
    /// This will:
    /// - Add a new event type of [`NetworkData<T>`]
    /// - Register the type for transformation over the wire
    /// - Internal bookkeeping
    fn listen_for_client_message<T: ClientMessage>(&mut self) -> &mut Self;
}

impl AppNetworkClientMessage for App {
    fn listen_for_client_message<T: ClientMessage>(&mut self) -> &mut Self {
        let client = self.world.get_resource::<NetworkClient>().expect("Could not find `NetworkClient`. Be sure to include the `ClientPlugin` before listening for client messages.");

        debug!("Registered a new ClientMessage: {}", T::NAME);

        assert!(
            !client.recv_message_map.contains_key(T::NAME),
            "Duplicate registration of ClientMessage: {}",
            T::NAME
        );
        client.recv_message_map.insert(T::NAME, Vec::new());

        self.add_event::<NetworkData<T>>();
        self.add_system_to_stage(CoreStage::PreUpdate, register_client_message::<T>.system())
    }
}

fn register_client_message<T>(
    net_res: ResMut<NetworkClient>,
    mut events: EventWriter<NetworkData<T>>,
) where
    T: ClientMessage,
{
    let mut messages = match net_res.recv_message_map.get_mut(T::NAME) {
        Some(messages) => messages,
        None => return,
    };

    events.send_batch(
        messages
            .drain(..)
            .flat_map(|msg| msg.downcast())
            .map(|msg| {
                NetworkData::new(
                    ConnectionId::server(
                        net_res
                            .server_connection
                            .as_ref()
                            .map(|conn| conn.peer_addr),
                    ),
                    *msg,
                )
            }),
    );
}

pub fn handle_connection_event(
    mut net_res: ResMut<NetworkClient>,
    mut events: EventWriter<ClientNetworkEvent>,
) {
    let (connection, peer_addr, network_settings) =
        match net_res.connection_events.receiver.try_recv() {
            Ok(event) => event,
            Err(_err) => {
                return;
            }
        };

    let (read_socket, send_socket) = connection.into_split();
    let recv_message_map = net_res.recv_message_map.clone();
    let (send_message, recv_message) = unbounded_channel();
    let network_event_sender = net_res.network_events.sender.clone();
    let network_event_sender_two = net_res.network_events.sender.clone();

    net_res.server_connection = Some(ServerConnection {
        peer_addr,
        send_task: net_res.runtime.spawn(async move {
            let mut recv_message = recv_message;
            let mut send_socket = send_socket;

            debug!("Starting new server connection, sending task");

            while let Some(message) = recv_message.recv().await {
                let encoded = match bincode::serialize(&message) {
                    Ok(encoded) => encoded,
                    Err(err) => {
                        error!("Could not encode packet {:?}: {}", message, err);
                        continue;
                    }
                };

                let len = encoded.len();
                debug!("Sending a new message of size: {}", len);

                match send_socket.write_u32(len as u32).await {
                    Ok(_) => (),
                    Err(err) => {
                        error!("Could not send packet length: {:?}: {}", len, err);
                        break;
                    }
                }

                trace!("Sending the content of the message!");

                match send_socket.write_all(&encoded).await {
                    Ok(_) => (),
                    Err(err) => {
                        error!("Could not send packet: {:?}: {}", message, err);
                        break;
                    }
                }

                trace!("Succesfully written all!");
            }

            let _ = network_event_sender_two.send(ClientNetworkEvent::Disconnected);
        }),
        receive_task: net_res.runtime.spawn(async move {
            let mut read_socket = read_socket;
            let network_settings = network_settings;
            let recv_message_map = recv_message_map;

            let mut buffer: Vec<u8> = (0..network_settings.max_packet_length).map(|_| 0).collect();
            loop {
                let length = match read_socket.read_u32().await {
                    Ok(len) => len as usize,
                    Err(err) => {
                        error!(
                            "Encountered error while fetching length [{}]: {}",
                            peer_addr, err
                        );
                        break;
                    }
                };

                if length > network_settings.max_packet_length {
                    error!(
                        "Received too large packet from [{}]: {} > {}",
                        peer_addr, length, network_settings.max_packet_length
                    );
                    break;
                }

                match read_socket.read_exact(&mut buffer[..length]).await {
                    Ok(_) => (),
                    Err(err) => {
                        error!(
                            "Encountered error while fetching stream of length {} [{}]: {}",
                            length, peer_addr, err
                        );
                        break;
                    }
                }

                let packet: NetworkPacket = match bincode::deserialize(&buffer[..length]) {
                    Ok(packet) => packet,
                    Err(err) => {
                        error!(
                            "Failed to decode network packet from [{}]: {}",
                            peer_addr, err
                        );
                        break;
                    }
                };

                match recv_message_map.get_mut(&packet.kind[..]) {
                    Some(mut packets) => packets.push(packet.data),
                    None => {
                        error!(
                            "Could not find existing entries for message kinds: {:?}",
                            packet
                        );
                    }
                }
                debug!("Received message from: {}", peer_addr);
            }

            let _ = network_event_sender.send(ClientNetworkEvent::Disconnected);
        }),
        send_message,
    });

    events.send(ClientNetworkEvent::Connected);
}

pub fn send_client_network_events(
    client_server: ResMut<NetworkClient>,
    mut client_network_events: EventWriter<ClientNetworkEvent>,
) {
    client_network_events.send_batch(client_server.network_events.receiver.try_iter());
}

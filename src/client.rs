use std::{net::SocketAddr, sync::Arc};

use bevy::prelude::*;
use dashmap::DashMap;
use derive_more::Display;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
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
    _receive_task: JoinHandle<()>,
    _send_task: JoinHandle<()>,
    send_message: UnboundedSender<NetworkPacket>,
}

/// An instance of a [`NetworkClient`] is used to connect to a remote server
/// using [`NetworkClient::connect`]
pub struct NetworkClient {
    runtime: Runtime,
    server_connection: Option<ServerConnection>,
    recv_message_map: Arc<DashMap<&'static str, Vec<Box<dyn NetworkMessage>>>>,
    network_events: SyncChannel<ClientNetworkEvent>,
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
        }
    }

    /// Connect to a remote server
    pub fn connect(
        &mut self,
        addr: impl ToSocketAddrs + Send,
        network_settings: NetworkSettings,
    ) -> Result<(), NetworkError> {
        debug!("Starting connection");
        let connection = self
            .runtime
            .block_on(async move { TcpStream::connect(addr).await })?;

        let peer_addr = connection.peer_addr()?;

        debug!("Connected to: {:?}", peer_addr);

        let (read_socket, send_socket) = connection.into_split();
        let recv_message_map = self.recv_message_map.clone();
        let (send_message, recv_message) = unbounded_channel();
        let network_event_sender = self.network_events.sender.clone();

        self.server_connection = Some(ServerConnection {
            peer_addr,
            _send_task: self.runtime.spawn(async move {
                let mut recv_message = recv_message;
                let mut bufwriter = BufWriter::new(send_socket);

                debug!("Starting new server connection, sending task");

                while let Some(message) = recv_message.recv().await {
                    let encoded = match serde_json::to_vec(&message) {
                        Ok(encoded) => encoded,
                        Err(err) => {
                            error!("Could not encode packet {:?}: {}", message, err);
                            continue;
                        }
                    };

                    let len = encoded.len();
                    debug!("Sending a new message of size: {}", len);

                    match bufwriter.write_u32(len as u32).await {
                        Ok(_) => (),
                        Err(err) => {
                            error!("Could not send packet length: {:?}: {}", len, err);
                            return;
                        }
                    }

                    match bufwriter.write_all(&encoded).await {
                        Ok(_) => (),
                        Err(err) => {
                            error!("Could not send packet: {:?}: {}", message, err);
                            return;
                        }
                    }
                }
            }),
            _receive_task: self.runtime.spawn(async move {
                let read_socket = read_socket;
                let network_settings = network_settings;
                let recv_message_map = recv_message_map;

                let mut bufstream = BufReader::new(read_socket);

                let mut buffer: Vec<u8> =
                    (0..network_settings.max_packet_length).map(|_| 0).collect();
                loop {
                    let length = match bufstream.read_u32().await {
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

                    match bufstream.read_exact(&mut buffer[..length]).await {
                        Ok(_) => (),
                        Err(err) => {
                            error!(
                                "Encountered error while fetching stream of length {} [{}]: {}",
                                length, peer_addr, err
                            );
                            break;
                        }
                    }

                    let packet: NetworkPacket = match serde_json::from_slice(&buffer[..length]) {
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

        match self
            .network_events
            .sender
            .send(ClientNetworkEvent::Connected)
        {
            Ok(_) => (),
            Err(_) => {
                error!("Could not send connected event");
                return Err(NetworkError::NotConnected);
            }
        }

        Ok(())
    }

    /// Send a message to the connected server, returns `Err(NetworkError::NotConnected)` if
    /// the connection couldn't be established yet
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
}

/// A utility trait on [`AppBuilder`] to easily register [`ClientMessage`]s
pub trait AppNetworkClientMessage {
    /// Register a client message type
    /// 
    /// ## Details
    /// This will:
    /// - Add a new event type of [`NetworkData<Box<T>>`]
    /// - Register the type for transformation over the wire
    /// - Internal bookkeeping
    fn add_client_message<T: ClientMessage>(&mut self);
}

impl AppNetworkClientMessage for AppBuilder {
    fn add_client_message<T: ClientMessage>(&mut self) {
        let client = self.world().get_resource::<NetworkClient>().unwrap();

        client.recv_message_map.insert(T::NAME, Vec::new());

        self.add_event::<NetworkData<Box<T>>>();
        self.add_system_to_stage(CoreStage::PreUpdate, register_client_message::<T>.system());
    }
}

fn register_client_message<T>(
    net_res: ResMut<NetworkClient>,
    mut events: EventWriter<NetworkData<Box<T>>>,
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
                    msg,
                )
            }),
    );
}

pub fn send_client_network_events(
    client_server: ResMut<NetworkClient>,
    mut client_network_events: EventWriter<ClientNetworkEvent>,
) {
    client_network_events.send_batch(client_server.network_events.receiver.try_iter());
}

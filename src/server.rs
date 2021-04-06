use std::{net::SocketAddr, sync::Arc};

use bevy::{prelude::*, utils::Uuid};
use dashmap::DashMap;
use derive_more::Display;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
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
    _receive_task: JoinHandle<()>,
    _send_task: JoinHandle<()>,
    send_message: UnboundedSender<NetworkPacket>,
    addr: SocketAddr,
}

impl std::fmt::Debug for ClientConnection {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        f.debug_struct("ClientConnection")
            .field("id", &self.id)
            .field("addr", &self.addr)
            .finish()
    }
}

pub struct NetworkServer {
    runtime: Runtime,
    recv_message_map: Arc<DashMap<&'static str, Vec<(ConnectionId, Box<dyn NetworkMessage>)>>>,
    established_connections: Arc<DashMap<ConnectionId, ClientConnection>>,
    new_connections: SyncChannel<Result<NewIncomingConnection, NetworkError>>,
    disconnected_connections: SyncChannel<ConnectionId>,
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
        }
    }

    pub fn listen(
        &self,
        addr: impl ToSocketAddrs + Send,
    ) -> Result<(), NetworkError> {
        let listener = self
            .runtime
            .block_on(async move { TcpListener::bind(addr).await })?;

        let new_connections = self.new_connections.sender.clone();

        let listen_loop = async move {
            let new_connections = new_connections;
            loop {
                let resp = match listener.accept().await {
                    Ok((socket, addr)) => Ok(NewIncomingConnection { socket, addr }),
                    Err(error) => Err(error.into()),
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

        self.runtime.spawn(listen_loop);

        Ok(())
    }

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
                        _receive_task: server.runtime.spawn(async move {
                            let read_socket = read_socket;
                            let recv_message_map = recv_message_map;
                            let network_settings = network_settings;

                            let mut bufstream = BufReader::new(read_socket);

                            let mut buffer = Vec::with_capacity(network_settings.max_packet_length);
                            loop {
                                let length = match bufstream.read_u32().await {
                                    Ok(len) => len as usize,
                                    Err(err) => {
                                        error!("Encountered error while fetching length [{}]: {}", conn_id, err);
                                        break;
                                    }
                                };


                                if length > network_settings.max_packet_length {
                                    error!("Received too large packet from [{}]: {} > {}", conn_id, length, network_settings.max_packet_length);
                                    break;
                                }


                                match bufstream.read_exact(&mut buffer[..length]).await {
                                    Ok(_) => (),
                                    Err(err) => {
                                        error!("Encountered error while fetching stream of length {} [{}]: {}", length, conn_id, err);
                                        break;
                                    }
                                }

                                let packet: NetworkPacket = match serde_json::from_slice(&buffer[..length]) {
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
                        _send_task: server.runtime.spawn(async move {
                            let mut recv_message = recv_message;
                            let mut bufwriter = BufWriter::new(send_socket);

                            while let Some(message) = recv_message.recv().await {
                                let encoded = match serde_json::to_vec(&message) {
                                    Ok(encoded) => encoded,
                                    Err(err) =>  {
                                        error!("Could not encode packet {:?}: {}", message, err);
                                        continue;
                                    }
                                };

                                let len = encoded.len();

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

pub trait AppNetworkServerMessage {
    fn add_server_message<T: ServerMessage>(&mut self);
}

impl AppNetworkServerMessage for AppBuilder {
    fn add_server_message<T: ServerMessage>(&mut self) {
        let client = self.world().get_resource::<NetworkServer>().unwrap();

        client.recv_message_map.insert(T::NAME, Vec::new());
        self.add_event::<NetworkData<Box<T>>>();
        self.add_system_to_stage(CoreStage::PreUpdate, register_server_message::<T>.system());
    }
}

fn register_server_message<T>(
    net_res: ResMut<NetworkServer>,
    mut events: EventWriter<NetworkData<Box<T>>>,
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
            .flat_map(|(conn, msg)| msg.downcast().map(|msg| NetworkData::new(conn, msg))),
    );
}

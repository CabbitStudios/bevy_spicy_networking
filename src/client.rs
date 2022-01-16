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
use async_trait::async_trait;

use crate::{
    error::NetworkError,
    network_message::{ClientMessage, NetworkMessage, ServerMessage},
    ClientNetworkEvent, ConnectionId, NetworkData, NetworkPacket, SyncChannel,
};

/// A trait used by [`NetworkClient`] to drive a client, this is responsible
/// for generating the futures that carryout the underlying client logic.
#[async_trait]
pub trait NetworkClientProvider: 'static + Send + Sync{
    /// This is to configure particular protocols
    type NetworkSettings: Send + Sync + Clone;

    /// This is the type given to the listener to start listening.
    type ConnectInfo: Send;

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

    /// Recieve a connection.
    async fn connect(connection_info: Self::ConnectInfo) -> Result<Self::Socket, Self::ProtocolErrors>;

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


#[derive(Display)]
#[display(fmt = "Server connection")]
struct ServerConnection {
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
pub struct NetworkClient<NCP: NetworkClientProvider> {
    runtime: Runtime,
    server_connection: Option<ServerConnection>,
    recv_message_map: Arc<DashMap<&'static str, Vec<Box<dyn NetworkMessage>>>>,
    network_events: SyncChannel<ClientNetworkEvent<NCP>>,
    connection_events: SyncChannel<NCP::Socket>,
    provider: NCP,
}

impl<NCP: NetworkClientProvider> std::fmt::Debug for NetworkClient<NCP> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(conn) = self.server_connection.as_ref() {
            write!(f, "NetworkClient [Connected to server]")?;
        } else {
            write!(f, "NetworkClient [Not Connected]")?;
        }

        Ok(())
    }
}

impl<NCP: NetworkClientProvider> NetworkClient<NCP> {
    pub(crate) fn new(provider: NCP) -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Could not build tokio runtime"),
            server_connection: None,
            recv_message_map: Arc::new(DashMap::new()),
            network_events: SyncChannel::new(),
            connection_events: SyncChannel::new(),
            provider
        }
    }

    /// Connect to a remote server
    ///
    /// ## Note
    /// This will disconnect you first from any existing server connections
    pub fn connect<'a>(
        &mut self,
        connect_info: impl Into<NCP::ConnectInfo>,
    ) {
        debug!("Starting connection");

        self.disconnect();

        let network_error_sender = self.network_events.sender.clone();
        let connection_event_sender = self.connection_events.sender.clone();
        let connect_info = connect_info.into();

        self.runtime.spawn(async move {
            let stream = match NCP::connect(connect_info).await {
                Ok(stream) => stream,
                Err(error) => {
                    match network_error_sender
                        .send(ClientNetworkEvent::Error(NetworkError::Provider(error)))
                    {
                        Ok(_) => (),
                        Err(err) => {
                            error!("Could not send error event: {}", err);
                        }
                    }

                    return;
                }
            };


            match connection_event_sender.send(stream) {
                Ok(_) => (),
                Err(err) => {
                    error!("Could not initiate connection: {}", err);
                }
            }

            debug!("Connected to server!");
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
    pub fn send_message<T: ServerMessage>(&self, message: T) -> Result<(), NetworkError<()>> {
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
    fn listen_for_client_message<T: ClientMessage, NCP: NetworkClientProvider>(&mut self) -> &mut Self;
}

impl AppNetworkClientMessage for App {
    fn listen_for_client_message<T: ClientMessage, NCP: NetworkClientProvider>(&mut self) -> &mut Self {
        let client = self.world.get_resource::<NetworkClient<NCP>>().expect("Could not find `NetworkClient`. Be sure to include the `ClientPlugin` before listening for client messages.");

        debug!("Registered a new ClientMessage: {}", T::NAME);

        assert!(
            !client.recv_message_map.contains_key(T::NAME),
            "Duplicate registration of ClientMessage: {}",
            T::NAME
        );
        client.recv_message_map.insert(T::NAME, Vec::new());

        self.add_event::<NetworkData<T>>();
        self.add_system_to_stage(CoreStage::PreUpdate, register_client_message::<T, NCP>)
    }
}

fn register_client_message<T, NCP: NetworkClientProvider>(
    net_res: ResMut<NetworkClient<NCP>>,
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
                    ConnectionId::server(),
                    *msg,
                )
            }),
    );
}

/// Pushes messages into the network event queue.
pub fn handle_connection_event<NCP: NetworkClientProvider>(
    mut net_res: ResMut<NetworkClient<NCP>>,
    mut events: EventWriter<ClientNetworkEvent<NCP>>,
    network_settings: Res<NCP::NetworkSettings>
) {
    let connection =
        match net_res.connection_events.receiver.try_recv() {
            Ok(event) => event,
            Err(_err) => {
                return;
            }
        };

    let (read_half, write_half) = NCP::split(connection);
    let recv_message_map = net_res.recv_message_map.clone();
    let (send_message, recv_message) = unbounded_channel();
    let network_event_sender = net_res.network_events.sender.clone();
    let network_event_sender_two = net_res.network_events.sender.clone();
    let read_network_settings = network_settings.clone();
    let write_network_settings = network_settings.clone();

    net_res.server_connection = Some(ServerConnection {
        send_task: net_res.runtime.spawn(async move {
            let mut recv_message = recv_message;
            let mut write_half = write_half;
            let mut write_params = NCP::init_write(&write_network_settings);

            debug!("Starting new server connection, sending task");

            while let Some(message) = recv_message.recv().await {

                match NCP::send_message(&message, &mut write_half, &write_network_settings, &mut write_params).await {
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
            let mut read_half = read_half;
            let recv_message_map = recv_message_map;
            let mut read_params = NCP::init_read(&read_network_settings);

            loop {
                let packet = match NCP::read_message(&mut read_half, &read_network_settings, &mut read_params).await {
                    Ok(packet) => packet,
                    Err(err) => {
                        error!("Failed to decode network packet from server: {}", err);
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
                debug!("Received message from server");
            }

            let _ = network_event_sender.send(ClientNetworkEvent::Disconnected);
        }),
        send_message,
    });

    events.send(ClientNetworkEvent::Connected);
}

/// Takes events and forwards them to the server.
pub fn send_client_network_events<NCP: NetworkClientProvider>(
    client_server: ResMut<NetworkClient<NCP>>,
    mut client_network_events: EventWriter<ClientNetworkEvent<NCP>>,
) {
    client_network_events.send_batch(client_server.network_events.receiver.try_iter());
}

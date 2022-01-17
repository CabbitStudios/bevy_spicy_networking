use std::net::SocketAddr;

use bevy::prelude::{error, trace, debug};
use bevy_spicy_networking::{async_trait, server::NetworkServerProvider, client::NetworkClientProvider, NetworkPacket, ClientNetworkEvent, error::NetworkError};
use tokio::{net::{TcpListener, TcpStream, tcp::{OwnedReadHalf, OwnedWriteHalf}}, io::{AsyncReadExt, AsyncWriteExt}, runtime::Runtime, sync::mpsc::{UnboundedSender, UnboundedReceiver}};

#[derive(Default)]
pub struct TokioTcpStreamServerProvider;

unsafe impl Send for TokioTcpStreamServerProvider{}
unsafe impl Sync for TokioTcpStreamServerProvider{}

#[async_trait]
impl NetworkServerProvider for TokioTcpStreamServerProvider{
    
    type NetworkSettings = NetworkSettings;

    type Socket = TcpStream;

    type ReadHalf = OwnedReadHalf;

    type WriteHalf = OwnedWriteHalf;

    async fn accept_loop(network_settings: Self::NetworkSettings, new_connections: UnboundedSender<Self::Socket>, errors: UnboundedSender<NetworkError>){
        let listener = match TcpListener::bind(network_settings.addr).await {
            Ok(listener) => listener,
            Err(err) => {
                match errors.send(NetworkError::Listen(err)) {
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
                Ok((socket, addr)) => socket,
                Err(error) => {
                    errors.send(NetworkError::Accept(error));
                    continue;
                },
            };

            match new_connections.send(resp) {
                Ok(_) => (),
                Err(err) => {
                    error!("Cannot accept new connections, channel closed: {}", err);
                    break;
                }
            }
        }
    }

    async fn recv_loop(mut read_half: Self::ReadHalf, messages: UnboundedSender<NetworkPacket>, settings: Self::NetworkSettings){
        let mut buffer: Vec<u8> = (0..settings.max_packet_length).map(|_| 0).collect();
        loop {
            let length = match read_half.read_u32().await {
                Ok(len) => len as usize,
                Err(err) => {
                    error!(
                        "Encountered error while fetching length: {}",
                        err
                    );
                    break;
                }
            };

            if length > settings.max_packet_length {
                error!(
                    "Received too large packet: {} > {}",
                    length, settings.max_packet_length
                );
                break;
            }

            match read_half.read_exact(&mut buffer[..length]).await {
                Ok(_) => (),
                Err(err) => {
                    error!(
                        "Encountered error while fetching stream of length {}: {}",
                        length, err
                    );
                    break;
                }
            }

            let packet: NetworkPacket = match bincode::deserialize(&buffer[..length]) {
                Ok(packet) => packet,
                Err(err) => {
                    error!(
                        "Failed to decode network packet from: {}",
                        err
                    );
                    break;
                }
            };

            if let Err(_) = messages.send(packet){
                error!("Failed to send decoded message to Spicy");
                break;
            }
        }
    }

    async fn send_loop(mut write_half: Self::WriteHalf, mut messages: UnboundedReceiver<NetworkPacket>, settings: Self::NetworkSettings){
        while let Some(message) = messages.recv().await {
            let encoded = match bincode::serialize(&message) {
                Ok(encoded) => encoded,
                Err(err) => {
                    error!("Could not encode packet {:?}: {}", message, err);
                    continue;
                }
            };

            let len = encoded.len();
            debug!("Sending a new message of size: {}", len);

            match write_half.write_u32(len as u32).await {
                Ok(_) => (),
                Err(err) => {
                    error!("Could not send packet length: {:?}: {}", len, err);
                    break;
                }
            }

            trace!("Sending the content of the message!");

            match write_half.write_all(&encoded).await {
                Ok(_) => (),
                Err(err) => {
                    error!("Could not send packet: {:?}: {}", message, err);
                    break;
                }
            }

            trace!("Succesfully written all!");
        }
    }
    

    fn split(combined: Self::Socket) -> (Self::ReadHalf, Self::WriteHalf){
        combined.into_split()
    }
}

#[derive(Default)]
pub struct TokioTcpStreamClientProvider;

unsafe impl Send for TokioTcpStreamClientProvider{}
unsafe impl Sync for TokioTcpStreamClientProvider{}

#[async_trait]
impl NetworkClientProvider for TokioTcpStreamClientProvider{
    
    type NetworkSettings = NetworkSettings;

    type Socket = TcpStream;

    type ReadHalf = OwnedReadHalf;

    type WriteHalf = OwnedWriteHalf;

    async fn connect_task(network_settings: Self::NetworkSettings, new_connections: UnboundedSender<Self::Socket>, errors: UnboundedSender<ClientNetworkEvent>){
        let stream = match TcpStream::connect(network_settings.addr).await {
            Ok(stream) => stream,
            Err(error) => {
                match errors
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

        match new_connections.send(stream) {
            Ok(_) => (),
            Err(err) => {
                error!("Could not initiate connection: {}", err);
            }
        }

        debug!("Connected to: {:?}", addr);
    }

    async fn recv_loop(mut read_half: Self::ReadHalf, messages: UnboundedSender<NetworkPacket>, settings: Self::NetworkSettings){
        let mut buffer: Vec<u8> = (0..settings.max_packet_length).map(|_| 0).collect();
        loop {
            let length = match read_half.read_u32().await {
                Ok(len) => len as usize,
                Err(err) => {
                    error!(
                        "Encountered error while fetching length: {}",
                        err
                    );
                    break;
                }
            };

            if length > settings.max_packet_length {
                error!(
                    "Received too large packet: {} > {}",
                    length, settings.max_packet_length
                );
                break;
            }

            match read_half.read_exact(&mut buffer[..length]).await {
                Ok(_) => (),
                Err(err) => {
                    error!(
                        "Encountered error while fetching stream of length {}: {}",
                        length, err
                    );
                    break;
                }
            }

            let packet: NetworkPacket = match bincode::deserialize(&buffer[..length]) {
                Ok(packet) => packet,
                Err(err) => {
                    error!(
                        "Failed to decode network packet from: {}",
                        err
                    );
                    break;
                }
            };

            if let Err(_) = messages.send(packet){
                error!("Failed to send decoded message to Spicy");
                break;
            }
        }
    }
    
    async fn send_loop(mut write_half: Self::WriteHalf, mut messages: UnboundedReceiver<NetworkPacket>, settings: Self::NetworkSettings){
        while let Some(message) = messages.recv().await {
            let encoded = match bincode::serialize(&message) {
                Ok(encoded) => encoded,
                Err(err) => {
                    error!("Could not encode packet {:?}: {}", message, err);
                    continue;
                }
            };

            let len = encoded.len();
            debug!("Sending a new message of size: {}", len);

            match write_half.write_u32(len as u32).await {
                Ok(_) => (),
                Err(err) => {
                    error!("Could not send packet length: {:?}: {}", len, err);
                    break;
                }
            }

            trace!("Sending the content of the message!");

            match write_half.write_all(&encoded).await {
                Ok(_) => (),
                Err(err) => {
                    error!("Could not send packet: {:?}: {}", message, err);
                    break;
                }
            }

            trace!("Succesfully written all!");
        }
    }

    fn split(combined: Self::Socket) -> (Self::ReadHalf, Self::WriteHalf){
        combined.into_split()
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
    pub addr: SocketAddr,
}

impl NetworkSettings{
    pub fn new(addr: impl Into<SocketAddr>) -> Self{
        Self{
            max_packet_length: 10 * 1024 * 1024,
            addr: addr.into()
        }
    }
}
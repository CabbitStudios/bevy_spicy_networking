use std::net::SocketAddr;

use bevy::prelude::{error, trace, debug, info};
use bevy_spicy_networking::{async_trait, server::NetworkServerProvider, client::NetworkClientProvider, NetworkPacket, ClientNetworkEvent, error::NetworkError, async_channel::{Sender, Receiver, unbounded}};
use mio::{net::{TcpStream, TcpListener}, io::{BufReader, BufWriter, WriteExt, ReadExt}};

#[derive(Default)]
pub struct TcpServerProvider;

unsafe impl Send for TcpServerProvider{}
unsafe impl Sync for TcpServerProvider{}

#[async_trait]
impl NetworkServerProvider for TcpServerProvider{
    
    type NetworkSettings = NetworkSettings;

    type Socket = TcpStream;

    type ReadHalf = TcpStream;

    type WriteHalf = TcpStream;

    async fn accept_loop(network_settings: Self::NetworkSettings, new_connections: Sender<Self::Socket>, errors: Sender<NetworkError>){
        let listener = match TcpListener::bind(network_settings.addr).await {
            Ok(listener) => listener,
            Err(err) => {
                match errors.send(NetworkError::Listen(err)).await {
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

            match new_connections.send(resp).await {
                Ok(_) => (),
                Err(err) => {
                    error!("Cannot accept new connections, channel closed: {}", err);
                    break;
                }
            }
            info!("New Connection Made!");
        }
    }

    async fn recv_loop(mut read_half: Self::ReadHalf, messages: Sender<NetworkPacket>, settings: Self::NetworkSettings){
        let mut buffer: Vec<u8> = (0..settings.max_packet_length).map(|_| 0).collect();
        loop {
            info!("Reading message length");
            let length = match read_half.read_exact(&mut buffer[..8]).await {
                Ok(len) => {
                    let bytes = &buffer[..8];
                    u64::from_le_bytes(bytes.try_into().unwrap()) as usize
                },
                Err(err) => {
                    error!(
                        "Encountered error while fetching length: {}",
                        err
                    );
                    break;
                }
            };
            info!("Message length: {}", length);

            if length > settings.max_packet_length {
                error!(
                    "Received too large packet: {} > {}",
                    length, settings.max_packet_length
                );
                break;
            }

            info!("Reading message into buffer");
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
            info!("Message read");

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

            if let Err(_) = messages.send(packet).await{
                error!("Failed to send decoded message to Spicy");
                break;
            }
            info!("Message read");
        }
    }

    async fn send_loop(mut write_half: Self::WriteHalf, mut messages: Receiver<NetworkPacket>, settings: Self::NetworkSettings){
        while let Ok(message) = messages.recv().await {
            let encoded = match bincode::serialize(&message) {
                Ok(encoded) => encoded,
                Err(err) => {
                    error!("Could not encode packet {:?}: {}", message, err);
                    continue;
                }
            };

            let len = encoded.len() as u64;
            debug!("Sending a new message of size: {}", len);

            match write_half.write(&len.to_le_bytes()).await {
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
        (BufReader::new(combined.clone()), BufWriter::new(combined))
    }
}

#[derive(Default)]
pub struct TcpClientProvider;

unsafe impl Send for TcpClientProvider{}
unsafe impl Sync for TcpClientProvider{}

#[async_trait]
impl NetworkClientProvider for TcpClientProvider{
    
    type NetworkSettings = NetworkSettings;

    type Socket = TcpStream;

    type ReadHalf = BufReader<TcpStream>;

    type WriteHalf = BufWriter<TcpStream>;

    async fn connect_task(network_settings: Self::NetworkSettings, new_connections: Sender<Self::Socket>, errors: Sender<ClientNetworkEvent>){
        info!("Beginning connection");
        let stream = match TcpStream::connect(network_settings.addr).await {
            Ok(stream) => stream,
            Err(error) => {
                match errors
                    .send(ClientNetworkEvent::Error(NetworkError::Connection(error))).await
                {
                    Ok(_) => (),
                    Err(err) => {
                        error!("Could not send error event: {}", err);
                    }
                }

                return;
            }
        };

        info!("Connected!");

        let addr = stream
            .peer_addr()
            .expect("Could not fetch peer_addr of existing stream");

        match new_connections.send(stream).await {
            Ok(_) => (),
            Err(err) => {
                error!("Could not initiate connection: {}", err);
            }
        }

        debug!("Connected to: {:?}", addr);
    }

    async fn recv_loop(mut read_half: Self::ReadHalf, messages: Sender<NetworkPacket>, settings: Self::NetworkSettings){
        let mut buffer: Vec<u8> = (0..settings.max_packet_length).map(|_| 0).collect();
        loop {
            info!("Reading message length");
            let length = match read_half.read_exact(&mut buffer[..8]).await {
                Ok(len) => {
                    let bytes = &buffer[..8];
                    u64::from_le_bytes(bytes.try_into().unwrap()) as usize
                },
                Err(err) => {
                    error!(
                        "Encountered error while fetching length: {}",
                        err
                    );
                    break;
                }
            };
            info!("Info read");

            if length > settings.max_packet_length {
                error!(
                    "Received too large packet: {} > {}",
                    length, settings.max_packet_length
                );
                break;
            }

            info!("Reading message into buffer");
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
            info!("Message read");

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

            if let Err(_) = messages.send(packet).await {
                error!("Failed to send decoded message to Spicy");
                break;
            }
        }
    }
    
    async fn send_loop(mut write_half: Self::WriteHalf, mut messages: Receiver<NetworkPacket>, settings: Self::NetworkSettings){
        while let Ok(message) = messages.recv().await {

            info!("Sending message!");

            let encoded = match bincode::serialize(&message) {
                Ok(encoded) => encoded,
                Err(err) => {
                    error!("Could not encode packet {:?}: {}", message, err);
                    continue;
                }
            };

            let len = encoded.len() as u64;
            debug!("Sending a new message of size: {}", len);

            match write_half.write(&len.to_le_bytes()).await {
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

            info!("Message sent!");
        }
    }

    fn split(combined: Self::Socket) -> (Self::ReadHalf, Self::WriteHalf){
        (BufReader::new(combined.clone()), BufWriter::new(combined))
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
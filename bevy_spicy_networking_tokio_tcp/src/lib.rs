use std::net::SocketAddr;

use bevy::prelude::{error, trace, debug};
use bevy_spicy_networking::{async_trait, server::NetworkServerProvider, client::NetworkClientProvider, NetworkPacket};
use tokio::{net::{TcpListener, TcpStream, tcp::{OwnedReadHalf, OwnedWriteHalf}}, io::{AsyncReadExt, AsyncWriteExt}, runtime::Runtime};

#[derive(Default)]
pub struct TokioTcpStreamServerProvider;

unsafe impl Send for TokioTcpStreamServerProvider{}
unsafe impl Sync for TokioTcpStreamServerProvider{}

#[async_trait]
impl NetworkServerProvider for TokioTcpStreamServerProvider{
    
    type NetworkSettings = NetworkSettings;

    type ListenInfo = SocketAddr;

    type Listener = TcpListener;

    type Socket = TcpStream;

    type ReadHalf = OwnedReadHalf;

    type WriteHalf = OwnedWriteHalf;

    type ReadParams = Vec<u8>;

    type WriteParams = ();

    type ListenParams = ();

    type ProtocolErrors = TokioTCPProviderErrors;

    fn init_listen<'a> (settings: &'a Self::NetworkSettings, runtime: &'a Runtime) -> Self::ListenParams{
        
    }

    async fn listen(listen_info: Self::ListenInfo, listen_params: Self::ListenParams) -> Result<Self::Listener, Self::ProtocolErrors>{
        TcpListener::bind(listen_info).await.map_err(|e| TokioTCPProviderErrors::ConnectError)
    }

    async fn accept<'l>(listener: &'l mut Self::Listener) -> Result<Self::Socket, Self::ProtocolErrors>{
        match listener.accept().await{
            Ok((socket, addr)) => {
                match socket.set_nodelay(true) {
                    Ok(_) => (),
                    Err(e) => error!("Could not set nodelay for [{}]: {}", addr, e),
                };
                Ok(socket)
            },
            Err(err) => Err(TokioTCPProviderErrors::ConnectError)
        }
    }

    fn init_read<'a>(settings: &'a Self::NetworkSettings, runtime: &'a Runtime) -> Self::ReadParams{
        (0..settings.max_packet_length).map(|_| 0).collect()
    }

    async fn read_message<'a>(read_half: &'a mut Self::ReadHalf, settings: &'a Self::NetworkSettings, read_params: &'a mut Self::ReadParams) -> Result<NetworkPacket, Self::ProtocolErrors>{
        trace!("Listening for length!");

        let length = match read_half.read_u32().await {
            Ok(len) => len as usize,
            Err(err) => {
                // If we get an EOF here, the connection was broken and we simply report a 'disconnected' signal
                error!("Encountered error while reading length {}", err);
                return Err(TokioTCPProviderErrors::ReadError);
            }
        };

        trace!("Received packet with length: {}", length);

        if length > settings.max_packet_length {
            error!("Received too large packet from {} > {}", length, settings.max_packet_length);
            return Err(TokioTCPProviderErrors::PacketSizeError);
        }


        match read_half.read_exact(&mut read_params[..length]).await {
            Ok(_) => (),
            Err(err) => {
                error!("Encountered error while reading stream of length {}: {}", length, err);
                return Err(TokioTCPProviderErrors::ReadError);
            }
        }

        trace!("Read buffer of length {}", length);


        trace!("Created a network packet");

        let res = bincode::deserialize(&read_params[..length]).map_err(|e| TokioTCPProviderErrors::DeserializeError);
        
        debug!("Received new message of length: {}", length);

        res
    }

    fn init_write<'a>(settings: &'a Self::NetworkSettings, runtime: &'a Runtime) -> Self::WriteParams{
        ()
    }

    async fn send_message<'a>(message: &'a NetworkPacket, write_half: &'a mut Self::WriteHalf, settings: &'a Self::NetworkSettings, write_params: &'a mut Self::WriteParams) -> Result<(), Self::ProtocolErrors>{
        let encoded = match bincode::serialize(&message) {
            Ok(encoded) => encoded,
            Err(err) =>  {
                return Err(TokioTCPProviderErrors::SerializeError);
            }
        };

        let len = encoded.len();

        match write_half.write_u32(len as u32).await {
            Ok(_) => (),
            Err(err) => {
                return Err(TokioTCPProviderErrors::WriteError);
            }
        }

        write_half.write_all(&encoded).await.map_err(|e| TokioTCPProviderErrors::WriteError)
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

    type ConnectInfo = SocketAddr;

    type Socket = TcpStream;

    type ReadHalf = OwnedReadHalf;

    type WriteHalf = OwnedWriteHalf;

    type ReadParams = Vec<u8>;

    type WriteParams = ();

    type ProtocolErrors = TokioTCPProviderErrors;

    async fn connect(connect_info: Self::ConnectInfo) -> Result<Self::Socket, Self::ProtocolErrors>{
        Self::Socket::connect(connect_info).await.map_err(|e| TokioTCPProviderErrors::ConnectError)
    }

    async fn read_message<'a>(read_half: &'a mut Self::ReadHalf, settings: &'a Self::NetworkSettings, read_params: &'a mut Self::ReadParams) -> Result<NetworkPacket, Self::ProtocolErrors>{
        let length = match read_half.read_u32().await {
            Ok(len) => len as usize,
            Err(err) => {
                error!(
                    "Encountered error while fetching length : {}",
                    err
                );
                return Err(TokioTCPProviderErrors::ReadError);
            }
        };

        if length > settings.max_packet_length {
            error!(
                "Received too large packet from: {} > {}",
                length, settings.max_packet_length
            );
            return Err(TokioTCPProviderErrors::PacketSizeError);
        }

        match read_half.read_exact(&mut read_params[..length]).await {
            Ok(_) => (),
            Err(err) => {
                error!(
                    "Encountered error while fetching stream of length {}: {}",
                    length, err
                );
                return Err(TokioTCPProviderErrors::ReadError);
            }
        }

        trace!("Read buffer of length {}", length);


        trace!("Created a network packet");

        let res = bincode::deserialize(&read_params[..length]).map_err(|e| TokioTCPProviderErrors::DeserializeError);
        
        debug!("Received new message of length: {}", length);

        res
    }
    
    async fn send_message<'a>(message: &'a NetworkPacket, write_half: &'a mut Self::WriteHalf, settings: &'a Self::NetworkSettings, write_params: &'a mut Self::WriteParams) -> Result<(), Self::ProtocolErrors>{
        let encoded = match bincode::serialize(&message) {
            Ok(encoded) => encoded,
            Err(err) => {
                error!("Could not encode packet {:?}: {}", message, err);
                return Err(TokioTCPProviderErrors::SerializeError);
            }
        };

        let len = encoded.len();
        debug!("Sending a new message of size: {}", len);

        match write_half.write_u32(len as u32).await {
            Ok(_) => (),
            Err(err) => {
                error!("Could not send packet length: {:?}: {}", len, err);
                return Err(TokioTCPProviderErrors::WriteError);
            }
        }

        trace!("Sending the content of the message!");

        match write_half.write_all(&encoded).await {
            Ok(_) => Ok(()),
            Err(err) => {
                return Err(TokioTCPProviderErrors::WriteError);
            }
        }
    }

    fn init_read<'a> (settings: &'a Self::NetworkSettings) -> Self::ReadParams{
        (0..settings.max_packet_length).map(|_| 0).collect()
    }

    fn init_write<'a> (settings: &'a Self::NetworkSettings) -> Self::WriteParams{} 

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
}

impl Default for NetworkSettings {
    fn default() -> Self {
        NetworkSettings {
            max_packet_length: 10 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub enum TokioTCPProviderErrors{
    ReadError,
    WriteError,
    SerializeError,
    DeserializeError,
    PacketSizeError,
    ConnectError,
}

impl std::fmt::Display for TokioTCPProviderErrors{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self{
            TokioTCPProviderErrors::ReadError => write!(f, "ReadError"),
            TokioTCPProviderErrors::WriteError => write!(f, "WriteError"),
            TokioTCPProviderErrors::DeserializeError => write!(f, "DeserializeError"),
            TokioTCPProviderErrors::SerializeError => write!(f, "SerializeError"),
            TokioTCPProviderErrors::PacketSizeError => write!(f, "PacketSizeError"),
            TokioTCPProviderErrors::ConnectError => write!(f, "ConnectError")
        }
    }
}
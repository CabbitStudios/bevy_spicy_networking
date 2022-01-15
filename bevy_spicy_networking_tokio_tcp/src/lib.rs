use std::net::SocketAddr;

use bevy_spicy_networking::{async_trait, server::NetworkServerProvider, NetworkPacket};
use tokio::{net::{TcpListener, TcpStream, tcp::{OwnedReadHalf, OwnedWriteHalf}}, io::{AsyncReadExt, AsyncWriteExt}};

#[derive(Default)]
pub struct TokioTcpStreamServerProvider;

unsafe impl Send for TokioTcpStreamServerProvider{}
unsafe impl Sync for TokioTcpStreamServerProvider{}

#[async_trait]
impl NetworkServerProvider for TokioTcpStreamServerProvider{
    
    type NetworkSettings = NetworkSettings;

    type ListenInfo = SocketAddr;

    type ClientInfo = SocketAddr;

    type Listener = TcpListener;

    type Socket = TcpStream;

    type ReadHalf = OwnedReadHalf;

    type WriteHalf = OwnedWriteHalf;

    type ReadParams = Vec<u8>;

    type WriteParams = ();

    type ProtocolErrors = std::io::Error;

    async fn listen(listen_info: Self::ListenInfo) -> Result<Self::Listener, Self::ProtocolErrors>{
        TcpListener::bind(listen_info).await
    }

    async fn accept<'l>(listener: &'l mut Self::Listener) -> Result<(Self::Socket, Self::ClientInfo), Self::ProtocolErrors>{
        match listener.accept().await{
            Ok((socket, addr)) => {
                match socket.set_nodelay(true) {
                    Ok(_) => (),
                    Err(e) => {}//error!("Could not set nodelay for [{}]: {}", new_conn, e),
                };
                Ok((socket, addr))
            },
            Err(err) => Err(err)
        }
    }

    fn init_read<'a>(settings: &'a Self::NetworkSettings) -> Self::ReadParams{
        (0..settings.max_packet_length).map(|_| 0).collect()
    }

    async fn read_message<'a>(read_half: &'a mut Self::ReadHalf, settings: &'a Self::NetworkSettings, read_params: &'a mut Self::ReadParams) -> Result<NetworkPacket, Self::ProtocolErrors>{
        //trace!("Listening for length!");

        let length = match read_half.read_u32().await {
            Ok(len) => len as usize,
            Err(err) => {
                // If we get an EOF here, the connection was broken and we simply report a 'disconnected' signal
                return Err(err);

                //error!("Encountered error while reading length [{}]: {}", conn_id, err);
            }
        };

        //trace!("Received packet with length: {}", length);

        if length > settings.max_packet_length {
            //error!("Received too large packet from [{}]: {} > {}", conn_id, length, network_settings.max_packet_length);
        }


        match read_half.read_exact(&mut read_params[..length]).await {
            Ok(_) => (),
            Err(err) => {
                //error!("Encountered error while reading stream of length {} [{}]: {}", length, conn_id, err);
            }
        }

        //trace!("Read buffer of length {}", length);


        //trace!("Created a network packet");

        let res = bincode::deserialize(&read_params[..length]).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, "Deserialize Error"));
        
        //debug!("Received new message of length: {}", length);

        res
    }

    fn init_write<'a>(settings: &'a Self::NetworkSettings) -> Self::WriteParams{
        ()
    }

    async fn send_message<'a>(message: &'a NetworkPacket, write_half: &'a mut Self::WriteHalf, settings: &'a Self::NetworkSettings, write_params: &'a mut Self::WriteParams) -> Result<(), Self::ProtocolErrors>{
        let encoded = match bincode::serialize(&message) {
            Ok(encoded) => encoded,
            Err(err) =>  {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "Deserialize Error"));
            }
        };

        let len = encoded.len();

        match write_half.write_u32(len as u32).await {
            Ok(_) => (),
            Err(err) => {
                return Err(err);
            }
        }

        write_half.write_all(&encoded).await
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
}

impl Default for NetworkSettings {
    fn default() -> Self {
        NetworkSettings {
            max_packet_length: 10 * 1024 * 1024,
        }
    }
}
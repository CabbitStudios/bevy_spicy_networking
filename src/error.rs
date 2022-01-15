use crate::ConnectionId;

#[derive(thiserror::Error, Debug)]
pub enum NetworkError<PE: std::fmt::Debug> {
    #[error("An error occured when accepting a new connnection: {0}")]
    Accept(std::io::Error),
    #[error("Could not find connection with id: {0}")]
    ConnectionNotFound(ConnectionId),
    #[error("Connection closed with id: {0}")]
    ChannelClosed(ConnectionId),
    #[error("Not connected to any server")]
    NotConnected,
    #[error("An error occured when trying to start listening for new connections: {0}")]
    Listen(std::io::Error),
    #[error("An error occured when trying to connect: {0}")]
    Connection(std::io::Error),
    #[error("An error coming from the underlying provider")]
    Provider(PE)
}

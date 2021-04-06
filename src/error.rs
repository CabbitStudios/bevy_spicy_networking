use crate::ConnectionId;

#[derive(thiserror::Error, Debug)]
pub enum NetworkError {
    #[error("A generic io error occured: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Could not find connection with id: {0}")]
    ConnectionNotFound(ConnectionId),
    #[error("Connection closed with id: {0}")]
    ChannelClosed(ConnectionId),
    #[error("Not connected to any server")]
    NotConnected,
}

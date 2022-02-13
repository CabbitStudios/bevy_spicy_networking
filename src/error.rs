use crate::ConnectionId;

/// Internal errors used by Spicy
#[derive(thiserror::Error, Debug)]
pub enum NetworkError {
    /// Error occured when accepting a new connection.
    #[error("An error occured when accepting a new connnection: {0}")]
    Accept(std::io::Error),

    /// Connection couldn't be found.
    #[error("Could not find connection with id: {0}")]
    ConnectionNotFound(ConnectionId),

    /// Failed to send across channel because it was closed.
    #[error("Connection closed with id: {0}")]
    ChannelClosed(ConnectionId),

    /// Can't send as there is no connection.
    #[error("Not connected to any server")]
    NotConnected,

    /// An error occured when trying to listen for connections.
    #[error("An error occured when trying to start listening for new connections: {0}")]
    Listen(std::io::Error),

    /// An error occured when trying to connect.
    #[error("An error occured when trying to connect: {0}")]
    Connection(std::io::Error),

}

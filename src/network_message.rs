use serde::{de::DeserializeOwned, Serialize};

/// Any type that should be sent over the wire has to implement [`NetworkMessage`].
///
/// ## Example
/// ```rust
/// use bevy_spicy_networking::NetworkMessage;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct PlayerInformation {
///     health: usize,
///     position: (u32, u32, u32)
/// }
///
/// #[typetag::serde]
/// impl NetworkMessage for PlayerInformation {}
/// ```
/// You will also have to mark it with either [`ServerMessage`] or [`ClientMessage`] (or both)
/// to signal which direction this message can be sent.

/**
A marker trait to signal that this message should be sent *to* a server

## Note

You can implement both [`ServerMessage`] and [`ClientMessage`]
*/
pub trait ServerMessage: Serialize + DeserializeOwned + Send + Sync + 'static{
    /// A unique name to identify your message, this needs to be unique __across all included crates__
    ///
    /// A good combination is crate name + struct name
    const NAME: &'static str;
}

/**
A marker trait to signal that this message should be sent *to* a client

## Note

You can implement both [`ClientMessage`] and [`ServerMessage`]
*/
pub trait ClientMessage: Serialize + DeserializeOwned + Send + Sync + 'static{
    /// A unique name to identify your message, this needs to be unique __across all included crates__
    ///
    /// A good combination is crate name + struct name
    const NAME: &'static str;
}

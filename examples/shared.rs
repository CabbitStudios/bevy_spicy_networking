use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use bevy_spicy_networking::{ClientMessage, NetworkMessage, ServerMessage};

/////////////////////////////////////////////////////////////////////
// In this example the client sends `UserChatMessage`s to the server,
// the server then broadcasts to all connected clients.
//
// We use two different types here, because only the server should
// decide the identity of a given connection and thus also sends a
// name.
//
// You can have a single message be sent both ways, it simply needs
// to implement both `ClientMessage` and `ServerMessage`.
/////////////////////////////////////////////////////////////////////

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserChatMessage {
    pub message: String,
}

#[typetag::serde]
impl NetworkMessage for UserChatMessage {}

impl ServerMessage for UserChatMessage {
    const NAME: &'static str = "example:UserChatMessage";
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NewChatMessage {
    pub name: String,
    pub message: String,
}

#[typetag::serde]
impl NetworkMessage for NewChatMessage {}

impl ClientMessage for NewChatMessage {
    const NAME: &'static str = "example:NewChatMessage";
}

#[allow(unused)]
pub fn client_register_network_messages(app: &mut App) {
    use bevy_spicy_networking::AppNetworkClientMessage;

    // The client registers messages that arrives from the server, so that
    // it is prepared to handle them. Otherwise, an error occurs.
    app.listen_for_client_message::<NewChatMessage>();
}

#[allow(unused)]
pub fn server_register_network_messages(app: &mut App) {
    use bevy_spicy_networking::AppNetworkServerMessage;

    // The server registers messages that arrives from a client, so that
    // it is prepared to handle them. Otherwise, an error occurs.
    app.listen_for_server_message::<UserChatMessage>();
}

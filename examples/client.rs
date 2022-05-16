#![allow(clippy::type_complexity)]

use bevy::prelude::*;
use bevy_spicy_networking::{ClientNetworkEvent, NetworkClient, NetworkData, NetworkSettings};
use std::net::SocketAddr;
mod shared;

fn main() {
    let mut app = App::new();

    app.add_plugins(DefaultPlugins);

    // You need to add the `ClientPlugin` first before you can register
    // `ClientMessage`s
    app.add_plugin(bevy_spicy_networking::ClientPlugin);

    // A good way to ensure that you are not forgetting to register
    // any messages is to register them where they are defined!
    shared::client_register_network_messages(&mut app);

    app.add_startup_system(setup_ui);

    app.add_system(handle_connect_button);
    app.add_system(handle_message_button);
    app.add_system(handle_incoming_messages);
    app.add_system(handle_network_events);

    app.init_resource::<GlobalChatSettings>();

    app.add_system_to_stage(CoreStage::PostUpdate, handle_chat_area);

    app.run();
}

///////////////////////////////////////////////////////////////
////////////// Incoming Message Handler ///////////////////////
///////////////////////////////////////////////////////////////

fn handle_incoming_messages(
    mut messages: Query<&mut GameChatMessages>,
    mut new_messages: EventReader<NetworkData<shared::NewChatMessage>>,
) {
    let mut messages = messages.single_mut();

    for new_message in new_messages.iter() {
        messages.add(UserMessage::new(&new_message.name, &new_message.message));
    }
}

fn handle_network_events(
    mut new_network_events: EventReader<ClientNetworkEvent>,
    connect_query: Query<&Children, With<ConnectButton>>,
    mut text_query: Query<&mut Text>,
    mut messages: Query<&mut GameChatMessages>,
) {
    let connect_children = connect_query.single();
    let mut text = text_query.get_mut(connect_children[0]).unwrap();
    let mut messages = messages.single_mut();

    for event in new_network_events.iter() {
        info!("Received event: {:?}", event);
        match event {
            ClientNetworkEvent::Connected => {
                messages.add(SystemMessage::new(
                    "Succesfully connected to server!".to_string(),
                ));
                text.sections[0].value = String::from("Disconnect");
            }

            ClientNetworkEvent::Disconnected => {
                messages.add(SystemMessage::new("Disconnected from server!".to_string()));
                text.sections[0].value = String::from("Connect to server");
            }
            ClientNetworkEvent::Error(err) => {
                messages.add(UserMessage::new(String::from("SYSTEM"), err.to_string()));
            }
        }
    }
}

///////////////////////////////////////////////////////////////
////////////// Data Definitions ///////////////////////////////
///////////////////////////////////////////////////////////////

struct GlobalChatSettings {
    chat_style: TextStyle,
    author_style: TextStyle,
}

impl FromWorld for GlobalChatSettings {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.get_resource::<AssetServer>().unwrap();

        GlobalChatSettings {
            chat_style: TextStyle {
                font: asset_server.load("fonts/OpenSans-Regular.ttf"),
                font_size: 20.,
                color: Color::BLACK,
            },
            author_style: TextStyle {
                font: asset_server.load("fonts/OpenSans-Regular.ttf"),
                font_size: 20.,
                color: Color::RED,
            },
        }
    }
}

enum ChatMessage {
    SystemMessage(SystemMessage),
    UserMessage(UserMessage),
}

impl ChatMessage {
    fn get_author(&self) -> String {
        match self {
            ChatMessage::SystemMessage(_) => "SYSTEM".to_string(),
            ChatMessage::UserMessage(UserMessage { user, .. }) => user.clone(),
        }
    }

    fn get_text(&self) -> String {
        match self {
            ChatMessage::SystemMessage(SystemMessage(msg)) => msg.clone(),
            ChatMessage::UserMessage(UserMessage { message, .. }) => message.clone(),
        }
    }
}

impl From<SystemMessage> for ChatMessage {
    fn from(other: SystemMessage) -> ChatMessage {
        ChatMessage::SystemMessage(other)
    }
}

impl From<UserMessage> for ChatMessage {
    fn from(other: UserMessage) -> ChatMessage {
        ChatMessage::UserMessage(other)
    }
}

struct SystemMessage(String);

impl SystemMessage {
    fn new<T: Into<String>>(msg: T) -> SystemMessage {
        Self(msg.into())
    }
}

struct UserMessage {
    user: String,
    message: String,
}

impl UserMessage {
    fn new<U: Into<String>, M: Into<String>>(user: U, message: M) -> Self {
        UserMessage {
            user: user.into(),
            message: message.into(),
        }
    }
}

#[derive(Component)]
struct ChatMessages<T> {
    messages: Vec<T>,
}

impl<T> ChatMessages<T> {
    fn new() -> Self {
        ChatMessages { messages: vec![] }
    }

    fn add<K: Into<T>>(&mut self, msg: K) {
        let msg = msg.into();
        self.messages.push(msg);
    }
}

type GameChatMessages = ChatMessages<ChatMessage>;

///////////////////////////////////////////////////////////////
////////////// UI Definitions/Handlers ////////////////////////
///////////////////////////////////////////////////////////////

#[derive(Component)]
struct ConnectButton;

fn handle_connect_button(
    mut net: ResMut<NetworkClient>,
    interaction_query: Query<
        (&Interaction, &Children),
        (Changed<Interaction>, With<ConnectButton>),
    >,
    mut text_query: Query<&mut Text>,
    mut messages: Query<&mut GameChatMessages>,
) {
    let mut messages = messages.single_mut();

    for (interaction, children) in interaction_query.iter() {
        let mut text = text_query.get_mut(children[0]).unwrap();
        if let Interaction::Clicked = interaction {
            if net.is_connected() {
                net.disconnect();
            } else {
                text.sections[0].value = String::from("Connecting...");
                messages.add(SystemMessage::new("Connecting to server..."));

                let ip_address = "127.0.0.1".parse().unwrap();

                info!("Address of the server: {}", ip_address);

                let socket_address = SocketAddr::new(ip_address, 9999);

                net.connect(
                    socket_address,
                    NetworkSettings {
                        max_packet_length: 10 * 1024 * 1024,
                    },
                );
            }
        }
    }
}

#[derive(Component)]
struct MessageButton;

fn handle_message_button(
    net: Res<NetworkClient>,
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<MessageButton>)>,
    mut messages: Query<&mut GameChatMessages>,
) {
    let mut messages = messages.single_mut();

    for interaction in interaction_query.iter() {
        if let Interaction::Clicked = interaction {
            match net.send_message(shared::UserChatMessage {
                message: String::from("Hello there!"),
            }) {
                Ok(()) => (),
                Err(err) => messages.add(SystemMessage::new(format!(
                    "Could not send message: {}",
                    err
                ))),
            }
        }
    }
}

#[derive(Component)]
struct ChatArea;

fn handle_chat_area(
    chat_settings: Res<GlobalChatSettings>,
    messages: Query<&GameChatMessages, Changed<GameChatMessages>>,
    mut chat_text_query: Query<&mut Text, With<ChatArea>>,
) {
    let messages = if let Ok(messages) = messages.get_single() {
        messages
    } else {
        return;
    };

    let sections = messages
        .messages
        .iter()
        .flat_map(|msg| {
            std::array::IntoIter::new([
                TextSection {
                    value: format!("{}: ", msg.get_author()),
                    style: chat_settings.author_style.clone(),
                },
                TextSection {
                    value: format!("{}\n", msg.get_text()),
                    style: chat_settings.chat_style.clone(),
                },
            ])
        })
        .collect::<Vec<_>>();

    let mut text = chat_text_query.single_mut();

    text.sections = sections;
}

fn setup_ui(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn_bundle(UiCameraBundle::default());

    commands.spawn_bundle((GameChatMessages::new(),));

    commands
        .spawn_bundle(NodeBundle {
            style: Style {
                size: Size::new(Val::Percent(100.), Val::Percent(100.)),
                justify_content: JustifyContent::SpaceBetween,
                flex_direction: FlexDirection::ColumnReverse,
                ..Default::default()
            },
            color: UiColor::from(Color::NONE),
            ..Default::default()
        })
        .with_children(|parent| {
            parent
                .spawn_bundle(NodeBundle {
                    style: Style {
                        size: Size::new(Val::Percent(100.), Val::Percent(90.)),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .with_children(|parent| {
                    parent
                        .spawn_bundle(TextBundle {
                            ..Default::default()
                        })
                        .insert(ChatArea);
                });
            parent
                .spawn_bundle(NodeBundle {
                    style: Style {
                        size: Size::new(Val::Percent(100.), Val::Percent(10.)),
                        ..Default::default()
                    },
                    color: UiColor::from(Color::GRAY),
                    ..Default::default()
                })
                .with_children(|parent_button_bar| {
                    parent_button_bar
                        .spawn_bundle(ButtonBundle {
                            style: Style {
                                size: Size::new(Val::Percent(50.), Val::Percent(100.)),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .insert(MessageButton)
                        .with_children(|button| {
                            button.spawn_bundle(TextBundle {
                                text: Text::with_section(
                                    "Send Message!",
                                    TextStyle {
                                        font: asset_server.load("fonts/Staatliches-Regular.ttf"),
                                        font_size: 40.,
                                        color: Color::BLACK,
                                    },
                                    TextAlignment {
                                        vertical: VerticalAlign::Center,
                                        horizontal: HorizontalAlign::Center,
                                    },
                                ),
                                ..Default::default()
                            });
                        });

                    parent_button_bar
                        .spawn_bundle(ButtonBundle {
                            style: Style {
                                size: Size::new(Val::Percent(50.), Val::Percent(100.)),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .insert(ConnectButton)
                        .with_children(|button| {
                            button.spawn_bundle(TextBundle {
                                text: Text::with_section(
                                    "Connect to server",
                                    TextStyle {
                                        font: asset_server.load("fonts/Staatliches-Regular.ttf"),
                                        font_size: 40.,
                                        color: Color::BLACK,
                                    },
                                    TextAlignment {
                                        vertical: VerticalAlign::Center,
                                        horizontal: HorizontalAlign::Center,
                                    },
                                ),
                                ..Default::default()
                            });
                        });
                });
        });
}

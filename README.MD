<img align="left" src="assets/spicy_networking_logo.svg" width="128">
<h1 align="center">
    Spicy Networking for Bevy
</h1>

<p align="center">
    <a href="https://crates.io/crates/bevy_spicy_networking">
        <img src="https://img.shields.io/crates/v/bevy_spicy_networking?logo=rust" alt="crates.io">
    </a>
    <a href="https://docs.rs/bevy_spicy_networking">
        <img src="https://docs.rs/bevy_spicy_networking/badge.svg" alt="docs.rs">
    </a>
    <img src="https://img.shields.io/badge/%F0%9F%8C%B6%20Spicy%3F-Yes-red" alt="spicy" />
</p>

> `bevy_spicy_networking` is a solution to the "How do I connect multiple clients to a single server" problem in your [bevy](https://bevyengine.org/) games.

<hr>

Using tokio as the asynchronous backend, it fully integrates into bevy to allow you to quickly add networking to your game.
Simply add either the server/client plugins, register the types of messages you plan on receiving and listen for them as events!

It is meant as a unifying crate for networking. Other crates can extend your game by registering their own messages. This is achieved through the amazing [`typetag`](https://github.com/dtolnay/typetag) crate.

<hr>

### Contents
- [Documentation](#documentation)
  - [Quickstart](#quickstart)
- [Bevy Version Compatibility](#bevy-version-compatibility)
- [Supported Platforms](#supported-platforms)
- [Roadmap](#roadmap)
- [Crates using `bevy_spicy_networking`](#crates-using-bevy_spicy_networking)
- [Contributing](#contributing)


Documentation
-------------

You can check out the [**online documentation**](https://docs.rs/bevy_spicy_networking), or build it yourself by cloning this repo and running `cargo doc -p bevy_spicy_networking`.

For examples, check out the [examples directory](https://github.com/CabbitStudios/bevy_spicy_networking/tree/master/examples).

- In `server.rs` you will find a simple chat server, that broadcasts the messages it receives from clients
- In `client.rs` you will find a simple graphical chat client, where you can connect to a server and send messages to

(Note: Since bevy does not include a text input widget, it is a very simplified demo. This should be easy to extend once the UI part of bevy 
is more complete.)

### Quickstart

1. Add `bevy_spicy_networking`, `serde_derive` and `typetag` to your `Cargo.toml`
2. Create the messages you wish to exchange beetween a client and server, or vice-versa.
    - Implement Serialize and Deserialize from Serde on it
    - Implement `NetworkMessage`, and make sure to annotate it with `typetag::serde`
    - Implement `ServerMessage` when it is sent to the server from a client
    - Implement `ClientMessage` when it is sent to a client from the server

```rust
#[derive(Serialize, Deserialize)]
struct WhisperMessage {
    recipient: UserId,
    message: String,
}

#[typetag::serde]
impl NetworkMessage for WhisperMessage {}

// In this case, its a client sending a message to a server
impl ServerMessage for WhisperMessage {
    const NAME: &'static str = "example:WhisperMessage"; // This name needs to be unique!
    // Otherwise, the recipient will mix up the messages
}
```

3. On the recipient side, register the type to be received

```rust
use bevy_spicy_networking::AppNetworkServerMessage;

let appbuilder: &mut AppBuilder = /* Get an AppBuilder, which is returned by bevy from App::build() */;


// Now whenever a client sends a `WhisperMessage` the server will generate an event of
// `NetworkData<WhisperMessage>` which your application can then handle
appbuilder.listen_for_server_message::<WhisperMessage>();
```

4. Listen for events of that type

```rust
fn handle_incoming_whisper_messages(
    mut whisper_messages: EventReader<NetworkMessage<WhisperMessage>>,
) {
    for whisper_message in whisper_messages.iter() {
        // Handle the whisper
    }
}
```

5. Enjoy easy and 🌶 networking in your game!


Bevy Version Compatibility
--------------------------

Simply pick the version compatible to your bevy version:

| Bevy Spicy Networking | Bevy  |
| :-------------------: | :---: |
|          0.5          |  0.5  |

Any version that is not compatible with the latest bevy version is in maintenance mode.
It will only receive minor bug fixes from my side, or community supplied ones.

Supported Platforms
-------------------

Currently only **Linux** and **Windows** are officially supported, I don't see why **MacOS** wouldn't be, but I do not have a Mac to test. If you have a Mac, and wish to test it out and report back, please let me know!

Roadmap
-------

Currently `bevy_spicy_networking` uses TCP only. This will change at some point, with individual messages being able to specify how they should be delivered. This change will be compatible, or with only minor changes necessary.


Crates using `bevy_spicy_networking`
--------------------

> Currently none, you can help by expanding this list. Just send a PR and add it to the table below!

| Name | Version |
|:----:|:-------:|
|  -   |    -    |

Contributing
------------

To contribute, simply fork the repository and send a PR. Feel free to chat me up on the bevy discord under `@Hemera#2040` if you have any questions or suggestions.

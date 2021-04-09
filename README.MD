🌶🌶🌶 Spicy Networking for Bevy 
================================


[![crates.io](https://img.shields.io/crates/v/bevy_spicy_networking)](https://crates.io/crates/bevy_spicy_networking)
[![docs.rs](https://docs.rs/bevy_spicy_networking/badge.svg)](https://docs.rs/bevy_spicy_networking)

`bevy_spicy_networking` is a solution to the "How do I connect multiple clients to a single server" problem in your [bevy](https://bevyengine.org/) games.

Using tokio as the asynchronous backend, it fully integrates into bevy to allow you to quickly add networking to your game.
Simply add either the server/client plugins, register the types of messages you plan on receiving and listen for them as events! 


Documentation
-------------

You can find the an [**online documentation**](https://docs.rs/bevy_spicy_networking), or build it yourself by cloning this repo and running `cargo doc -p bevy_spicy_networking`.

For examples, be sure to check out the [examples directory](https://github.com/CabbitStudios/bevy_spicy_networking/tree/master/examples).

- In `server.rs` you will find a simple chat server, that broadcasts the messages it receives from clients
- In `client.rs` you will find a simple graphical chat client, where you can connect to a server and send messages to

(Note: Since bevy does not include a text input widget, it is a very simplified demo. This should be easy to extend once the UI part of bevy 
is more complete.)

Bevy Version Compatibility
--------------------------

`bevy_spicy_networking` uses the same minor versioning as `bevy` does, starting at `0.5`. Meaning that `bevy_spicy_networking 0.5` is compatible with `bevy 0.5`.
This also means that this crate will only get stable once bevy is also stable. 

Current crate version: [![docs.rs](https://docs.rs/bevy_spicy_networking/badge.svg)](https://docs.rs/bevy_spicy_networking)  
Current bevy version: [![docs.rs](https://docs.rs/bevy/badge.svg)](https://docs.rs/bevy)

Roadmap
-------


Currently `bevy_spicy_networking` uses TCP only. This will change at some point, with individual messages being able to specify how they should be delivered.


Contributing
------------

To contribute, simply fork the repository and send a PR. Feel free to chat me up on the bevy discord under `@Hemera#2040` if you have any questions or suggestions.
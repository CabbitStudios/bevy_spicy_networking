
use downcast_rs::DowncastSync;

#[typetag::serde(tag = "type")]
pub trait NetworkMessage: DowncastSync {}

downcast_rs::impl_downcast!(sync NetworkMessage);

pub trait ServerMessage: NetworkMessage {
    const NAME: &'static str;
}

pub trait ClientMessage: NetworkMessage {
    const NAME: &'static str;
}

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::tcp::{OwnedWriteHalf, OwnedReadHalf},
};
use std::io::Result;

#[cfg(not(feature = "u16-packetsize"))]
pub(crate) async fn write_size(socket: &mut OwnedWriteHalf, size: usize) -> Result<()> {
    socket.write_u32(size as u32).await
}

#[cfg(feature = "u16-packetsize")]
pub(crate) async fn write_size(socket: &mut OwnedWriteHalf, size: usize) -> Result<()> {
    socket.write_u16(size as u16).await
}

#[cfg(not(feature = "u16-packetsize"))]
pub(crate) async fn read_size(socket: &mut OwnedReadHalf) -> Result<u32> {
    socket.read_u32().await
}

#[cfg(feature = "u16-packetsize")]
pub(crate) async fn read_size(socket: &mut OwnedReadHalf) -> Result<u16> {
    socket.read_u16().await
}

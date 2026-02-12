use anyhow::Result;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::protocol::{deserialize_message, serialize_message, Message};

pub fn get_socket_path() -> PathBuf {
    dirs::runtime_dir()
        .or_else(|| dirs::data_local_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("legion.sock")
}

pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    pub async fn connect() -> Result<Self> {
        let path = get_socket_path();
        let stream = UnixStream::connect(&path).await?;
        Ok(Self { stream })
    }

    pub async fn send(&mut self, msg: Message) -> Result<()> {
        let data = serialize_message(&msg);
        self.stream.write_all(&data).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Message> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;

        let mut full = len_buf.to_vec();
        full.extend(data);

        deserialize_message(&full)
            .ok_or_else(|| anyhow::anyhow!("Failed to deserialize message"))
    }
}

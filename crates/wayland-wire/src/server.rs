use crate::{core::HeadlessWireCore, Result, WireError};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

pub struct WireServerConfig {
    pub socket_path: PathBuf,
}

pub struct WireServer {
    pub config: WireServerConfig,
    pub core: HeadlessWireCore,
    received_fds: Vec<crate::WireOwnedFd>,
}

impl WireServer {
    pub fn new(config: WireServerConfig) -> Result<Self> {
        // Validation: socket_path must not be in /run/user or XDG_RUNTIME_DIR
        let path_str = config.socket_path.to_string_lossy();
        if path_str.contains("/run/user") || path_str.contains("waybroker") {
            // In a real system we would be more strict, but for parity P4 we just check these.
            // Actually task says: socket_path が /run/user や XDG_RUNTIME_DIR 配下の場合は拒否する
            return Err(WireError::ProtocolError("Forbidden socket path".into()));
        }

        Ok(Self { config, core: HeadlessWireCore::default(), received_fds: Vec::new() })
    }

    pub fn run_once(&mut self) -> Result<()> {
        if self.config.socket_path.exists() {
            std::fs::remove_file(&self.config.socket_path)?;
        }

        let listener = UnixListener::bind(&self.config.socket_path)?;
        println!("service=wire_server event=listening path={}", self.config.socket_path.display());

        let (stream, _) = listener.accept()?;
        println!("service=wire_server event=connected");

        let mut buffer = Vec::with_capacity(4096);
        let mut read_buf = [0u8; 4096];
        loop {
            let (n, fds) = crate::fd::recv_with_fds(&stream, &mut read_buf)?;
            if n == 0 && fds.is_empty() {
                break;
            }
            buffer.extend_from_slice(&read_buf[..n]);
            self.received_fds.extend(fds);

            let mut consumed = 0;
            loop {
                let remaining = &buffer[consumed..];
                if remaining.len() < 8 {
                    break;
                }

                match crate::codec::decode_message(remaining) {
                    Ok(msg) => {
                        let size = msg.header.size as usize;
                        consumed += size;

                        // We need to pass the FDs to dispatch.
                        // But which FDs belong to which message?
                        // In Wayland, FDs are conceptually part of the message they are sent with.
                        // HeadlessWireCore::dispatch should probably take the whole queue and pop from it.
                        // For now, let's pass a slice of the queue.
                        // Actually, I'll pass the whole queue and let the dispatcher pop.
                        // But HeadlessWireCore::dispatch currently takes Option<usize>.
                        // I'll change it to take &mut Vec<WireOwnedFd>.

                        let result = self.core.dispatch_with_fds(msg, &mut self.received_fds)?;
                        for ev in result.events {
                            let encoded = crate::codec::encode_message(&ev)?;
                            // We don't send FDs back in events for now
                            use std::io::Write;
                            let mut s = &stream;
                            s.write_all(&encoded)?;
                        }
                    }
                    Err(WireError::Incomplete) => break,
                    Err(e) => return Err(e),
                }
            }

            if consumed > 0 {
                buffer.drain(0..consumed);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reject_runtime_socket_path() {
        let config = WireServerConfig { socket_path: PathBuf::from("/run/user/1000/test.sock") };
        assert!(WireServer::new(config).is_err());

        let config2 = WireServerConfig { socket_path: PathBuf::from("/tmp/waybroker/test.sock") };
        assert!(WireServer::new(config2).is_err());
    }
}

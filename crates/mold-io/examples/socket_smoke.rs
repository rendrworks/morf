use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::thread;

use mold_io::SocketServer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!("mold-io-smoke-{}", std::process::id()));
    let server = SocketServer::bind(&path)?;
    let join = thread::spawn(move || -> std::io::Result<()> {
        let mut peer = server.accept()?;
        let mut request = [0; 4];
        peer.receive(&mut request)?;
        if &request != b"ping" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unexpected request",
            ));
        }
        peer.send(b"pong")
    });
    let mut client = UnixStream::connect(&path)?;
    client.write_all(b"ping")?;
    let mut response = [0; 4];
    client.read_exact(&mut response)?;
    join.join().map_err(|_| "socket server panicked")??;
    fs::remove_file(path)?;
    if &response != b"pong" {
        return Err("unexpected response".into());
    }
    println!("Unix socket ping/pong passed");
    Ok(())
}

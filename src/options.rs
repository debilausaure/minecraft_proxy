use std::net::SocketAddr;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Options {
    /// Socket address to listen to.
    #[arg(short, long)]
    pub listener_socket_addr: SocketAddr,
    /// Address to forward connections to.
    #[arg(short, long)]
    pub server_socket_addr: String,
    /// Minecraft version number.
    #[arg(short = 'v', long)]
    pub minecraft_version: String,
    /// Minecraft protocol version number.
    #[arg(short = 'p', long)]
    pub minecraft_protocol_version: i32,
    /// Minecraft server description.
    #[arg(short = 'd', long)]
    pub minecraft_description: String,
    /// Minecraft server Docker container name.
    #[arg(short = 'c', long)]
    pub minecraft_container_name: String,
}
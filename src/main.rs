#![warn(rust_2018_idioms)]

mod options;

use std::{error::Error, net::SocketAddr, process};

use clap::Parser;
use log::{error, info, warn, LevelFilter};
use mc_server_list_ping::{types::HandshakePacket, *};
use tokio::{
    io,
    net::{TcpListener, TcpStream},
    process::Command,
    sync::{mpsc, oneshot},
    time,
};

use crate::options::Options;

#[derive(Debug)]
enum TaskSignal {
    New(oneshot::Sender<()>),
    Close,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Parse options;
    let options = Options::parse();

    // Setup logger.
    env_logger::builder()
        .filter_level(LevelFilter::Info)
        .parse_env("MC_LOG")
        .init();
    log_panics::init();

    info!(socket:% = options.listener_socket_addr; "listening");
    info!(socket:% = options.server_socket_addr; "proxying");

    // Create a channel that will be used by client tasks to announce client
    // arrivals to the watchdog.
    let (watchdog_notify_channel, watchdog_listen_channel) = mpsc::channel(10);

    // Spawn a watchdog that will handle connection and disconnection events
    // and decide to start / stop the server.
    let watchdog_future = watchdog(watchdog_listen_channel);
    let _watchdog_handle = tokio::spawn(watchdog_future);

    let minecraft_version: &'static str = Box::leak(Box::from(options.minecraft_version));
    let minecraft_description: &'static str = Box::leak(Box::from(options.minecraft_description));
    let fsm = Fsm::new(&minecraft_version, options.minecraft_protocol_version)
        .description(minecraft_description);
    let fsm: &'static Fsm<'_> = Box::leak(Box::new(fsm));

    let listener = TcpListener::bind(options.listener_socket_addr).await?;

    while let Ok((client_stream, _)) = listener.accept().await {
        let new_client_future = handle_new_client(
            client_stream,
            options.server_socket_addr,
            &fsm,
            watchdog_notify_channel.clone(),
        );

        // Create a new task that can be run in parallel with other tasks.
        tokio::spawn(new_client_future);
    }

    Ok(())
}

async fn watchdog(mut watchdog_listen_channel: mpsc::Receiver<TaskSignal>) {
    let mut connection_counter = 0;
    let mut server_running = true;

    loop {
        tokio::select! {
            // If there are no active connections to the server and server is up, start a timer.
            _ = time::sleep(time::Duration::from_secs(60)), if server_running && (connection_counter == 0) => {
                server_running = false;
                info!(reason = "no active users"; "stopping server");
                if !stop_server().await.success(){
                    error!("failed to stop server");
                }
                info!("server stopped");
            },

            // Listen for incoming signals.
            Some(task_signal) = watchdog_listen_channel.recv() => {
                match task_signal {
                    // New client connected to the proxy.
                    TaskSignal::New(task_notify_channel) => {
                        if !server_running {
                            info!("starting minecraft server");
                            if !start_server().await.success() {
                                panic!("failed to start the server");
                            }
                            info!("server started");
                            server_running = true;
                        }
                        // Notify client task that it can connect to the server.
                        connection_counter += 1;
                        task_notify_channel.send(()).expect("failed to notify");
                    }
                    TaskSignal::Close => {
                        connection_counter -= 1;
                    }
                }
            },
        }
    }
}

async fn start_server() -> process::ExitStatus {
    Command::new("curl")
        .args(&[
            "-XPOST",
            "--unix-socket",
            "/var/run/docker.sock",
            "http://localhost/containers/minecraft_server/start",
        ])
        .status()
        .await
        .expect("failed to start the server container")
}

async fn stop_server() -> process::ExitStatus {
    Command::new("curl")
        .args(&[
            "-XPOST",
            "--unix-socket",
            "/var/run/docker.sock",
            "http://localhost/containers/minecraft_server/stop",
        ])
        .status()
        .await
        .expect("failed to stop the server container")
}

async fn handle_new_client(
    client_stream: TcpStream,
    server_socket_addr: SocketAddr,
    fsm: &Fsm<'_>,
    watchdog_notify_channel: mpsc::Sender<TaskSignal>,
) {
    let client_socket_addr = client_stream
        .peer_addr()
        .expect("could not get client address");
    info!(client_socket_addr:%; "handling connection");

    let (client_stream, packet) = match fsm.run(client_stream).await.expect("failed to run fsm") {
        // Server list ping.
        None => {
            info!(client_socket_addr:%; "server pinged");
            return;
        }
        // Server connection.
        Some((client_stream, packet)) => (client_stream, packet),
    };
    info!(client_socket_addr:%; "game connection");

    // Create a channel sent to the watchdog to know whether we can initiate
    // connection to the server or not.
    let (task_notify_channel, task_listen_channel) = oneshot::channel();

    // Let watchdog know a new connection was received.
    watchdog_notify_channel
        .send(TaskSignal::New(task_notify_channel))
        .await
        .expect("failed to notify watchdog");
    // Make sure the server started before proxying.
    task_listen_channel.await.expect("failed to notify listener");

    info!(client_socket_addr:%; "proxying new connection to server");
    let _ = proxy_stream(client_stream, server_socket_addr, packet).await;

    info!(client_socket_addr:%; "connection closed");
    watchdog_notify_channel
        .send(TaskSignal::Close)
        .await
        .expect("failed to notify watchdog");
}

// Proxies the stream to the server.
async fn proxy_stream(
    mut client_stream: TcpStream,
    server_socket_addr: SocketAddr,
    packet: HandshakePacket,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut server_stream = TcpStream::connect(server_socket_addr).await?;
    packet.send(&mut server_stream).await?;

    let (mut read_client, mut write_client) = client_stream.split();
    let (mut read_server, mut write_server) = server_stream.split();

    tokio::select! {
        _ = io::copy(&mut read_client, &mut write_server) => {},
        _ = io::copy(&mut read_server, &mut write_client) => {},
    }

    Ok(())
}

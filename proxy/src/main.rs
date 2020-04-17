#![warn(rust_2018_idioms)]

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio::time;

use std::env;
use std::error::Error;
use std::process;

use mc_server_list_ping::*;
use mc_server_list_ping::types::HandshakePacket;

#[derive(Debug)]
enum TaskSignal {
    New(oneshot::Sender<()>),
    Close,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // panics if no listen addr or server addr is provided
    let listen_addr = env::args()
        .nth(1)
        .expect("You must provide an address to listen on as a first argument");
    let server_addr = env::args()
        .nth(2)
        .expect("You must provide an address to proxy to as a second argument");

    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    //create a channel that will be used by client tasks to announce client arrivals to the watchdog
    let (watchdog_notify_channel, watchdog_listen_channel) = mpsc::channel(10);

    //spawn a watchdog that will handle connection and disconnection events
    //and decide to start / stop the server
    let watchdog_future = watchdog(watchdog_listen_channel);
    let _watchdog_handle = tokio::spawn(watchdog_future);

    let fsm = Fsm::new(
        "1.15.2",
        578,
    ).description(
        "Server not started",
    );
    let fsm = Box::new(fsm);
    let fsm : &'static Fsm<'_> = Box::leak(fsm);

    let mut listener = TcpListener::bind(listen_addr).await?;

    while let Ok((client_stream, _)) = listener.accept().await {
        let new_client_future = handle_new_client(
            client_stream,
            server_addr.clone(),
            &fsm,
            watchdog_notify_channel.clone(),
        );

        // create a new task that can be run in parallel with other tasks
        tokio::spawn(new_client_future);
    }

    Ok(())
}

async fn watchdog(mut watchdog_listen_channel: mpsc::Receiver<TaskSignal>) {
    let mut connection_counter = 0;
    let mut server_running = false;

    loop {
        tokio::select! {
            // if there are no active connections to the server and server is up, start a timer
            _ = time::delay_for(time::Duration::from_secs(10)), if server_running && (connection_counter == 0) => {
                server_running = false;
                print!("Timer elapsed, stopping the server... ");
                if !stop_server().await.success(){
                    panic!("Failed to stop the server :(");
                }
                println!("Server stopped")
            },

            // listen for incoming signals
            Some(task_signal) = watchdog_listen_channel.recv() => {
                match task_signal {
                    // a new client connected to the proxy
                    TaskSignal::New(task_notify_channel) => {
                        if !server_running {
                            print!("Starting the server... ");
                            if !start_server().await.success() {
                                panic!("Failed to start the server :(");
                            }
                            println!("Server started");
                            server_running = true;
                        }
                        // notify client task that it can connect to the server
                        connection_counter += 1;
                        task_notify_channel.send(()).unwrap();
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
    Command::new("docker")
        .args(&["start", "mc_alpine"])
        .status()
        .await
        .expect("Failed to start the server container")
}

async fn stop_server() -> process::ExitStatus {
    Command::new("docker")
        .args(&["stop", "mc_alpine"])
        .status()
        .await
        .expect("Failed to stop the server container")
}

async fn handle_new_client(
    client_stream: TcpStream,
    server_addr: String,
    fsm : &Fsm<'_>,
    mut watchdog_notify_channel: mpsc::Sender<TaskSignal>,
) {
    println!("New connection !");

    let (client_stream, packet) = match fsm.run(client_stream).await.unwrap() {
        // server list ping
        None => return,
        // server connection
        Some((client_stream, packet)) => (client_stream, packet),
    };

    // create a channel sent to the watchdog to know whether
    // we can initiate connection to the server or not
    let (task_notify_channel, task_listen_channel) = oneshot::channel();

    // let watchdog know a new connection was received
    watchdog_notify_channel
        .send(TaskSignal::New(task_notify_channel))
        .await
        .unwrap();
    // make sure the server started before proxying
    task_listen_channel.await.unwrap();

    println!("Proxying the new connection to the server...");
    let _ = proxy_stream(client_stream, server_addr, packet).await;

    println!("Connection closed !");
    watchdog_notify_channel
        .send(TaskSignal::Close)
        .await
        .unwrap();
}

// proxies the stream to the server
async fn proxy_stream(
    mut client_stream: TcpStream,
    server_addr: String,
    packet: HandshakePacket,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut server_stream = TcpStream::connect(server_addr).await?;

    packet.send(&mut server_stream).await?;

    let (mut read_client, mut write_client) = client_stream.split();
    let (mut read_server, mut write_server) = server_stream.split();

    tokio::select! {
        _ = io::copy(&mut read_client, &mut write_server) => {},
        _ = io::copy(&mut read_server, &mut write_client) => {},
    }

    Ok(())
}

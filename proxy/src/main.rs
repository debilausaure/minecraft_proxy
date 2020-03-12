#![warn(rust_2018_idioms)]

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time;

use std::env;
use std::error::Error;

#[derive(Debug)]
enum ClientSignal {
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

    //create a channel that will be used by connected clients to announce their arrival to the watchdog
    let (watchdog_notification_channel, watchdog_receive_channel) = mpsc::channel(10);

    //spawn a watchdog that will handle connection and disconnection events
    let watchdog_future = watchdog(watchdog_receive_channel);
    tokio::spawn(watchdog_future);

    let mut listener = TcpListener::bind(listen_addr).await?;

    while let Ok((client_stream, _)) = listener.accept().await {
        let new_client_future =
            handle_new_client(client_stream, server_addr.clone(), watchdog_notification_channel.clone());

        // create a new task that can be run in parallel with other tasks
        tokio::spawn(new_client_future);
    }

    Ok(())
}

async fn watchdog(
    mut communication_recv_channel: mpsc::Receiver<ClientSignal>,
) {
    let mut connection_counter = 0;
    let mut server_running = false;
    //let mut last_disconnect_timestamp = time::Instant::now();

    loop {
        tokio::select! {
            Some(client_signal) = communication_recv_channel.recv() => {
                match client_signal {
                   ClientSignal::New(client_response_channel) => {
                        if !server_running {
                            server_running = true;
                            println!("Server started");
                        }
                        // notifying client
                        connection_counter += 1;
                        client_response_channel.send(()).unwrap();
                    }
                    ClientSignal::Close => {
                        connection_counter -= 1;
                    }
                }
            },
            _ = time::delay_for(time::Duration::from_secs(10)), if server_running && (connection_counter == 0) => {
                server_running = false;
                println!("Timer elapsed, server shutdown !");
            }
        }
    }

    //while let Some(client_signal) = communication_recv_channel.recv().await {
    //    match client_signal {
    //        ClientSignal::New(client_response_channel) => {
    //            if !server_running {
    //                server_running = true;
    //                println!("Start server !");
    //                println!("Server started");
    //            }
    //            // notifying client
    //            connection_counter += 1;
    //            client_response_channel.send(()).unwrap();
    //        }
    //        ClientSignal::Close => {
    //            connection_counter -= 1;
    //            if connection_counter == 0 {
    //                last_disconnect_timestamp = time::Instant::now();
    //                let last_disconnect_timestamp = last_disconnect_timestamp.clone();
    //                let mut communication_send_channel = communication_send_channel.clone();
    //                tokio::spawn(async move {
    //                    time::delay_for(time::Duration::new(10, 0)).await;
    //                    communication_send_channel
    //                        .send(ClientSignal::Timer(last_disconnect_timestamp))
    //                        .await
    //                        .unwrap();
    //                });
    //            }
    //        }
    //        ClientSignal::Timer(timestamp) => {
    //            if last_disconnect_timestamp == timestamp {
    //                server_running = false;
    //                println!("Shutdown server !");
    //            }
    //        }
    //    }
    //}
}

async fn handle_new_client(
    client_stream: TcpStream,
    server_addr: String,
    mut watchdog_channel: mpsc::Sender<ClientSignal>,
) {
    println!("New connection !");

    let (sender, receiver) = oneshot::channel();

    // let watchdog know a new connection was received
    watchdog_channel
        .send(ClientSignal::New(sender))
        .await
        .unwrap();
    // make sure the server started before proxying
    receiver.await.unwrap();

    println!("Proxying the new connection to the server...");
    let _ = proxy_stream(client_stream, server_addr).await;

    println!("Connection closed !");
    watchdog_channel.send(ClientSignal::Close).await.unwrap();
}

// proxies the stream to the server
async fn proxy_stream(
    mut client_stream: TcpStream,
    server_addr: String,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut server_stream = TcpStream::connect(server_addr).await?;

    let (mut read_client, mut write_client) = client_stream.split();
    let (mut read_server, mut write_server) = server_stream.split();

    tokio::select! {
        _client_to_proxy = io::copy(&mut read_client, &mut write_server) => {},
        _proxy_to_server = io::copy(&mut read_server, &mut write_client) => {},
    }

    Ok(())
}

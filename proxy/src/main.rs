#![warn(rust_2018_idioms)]

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time;

use std::env;
use std::error::Error;

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
    tokio::spawn(watchdog_future);

    let mut listener = TcpListener::bind(listen_addr).await?;

    while let Ok((client_stream, _)) = listener.accept().await {
        let new_client_future = handle_new_client(
            client_stream,
            server_addr.clone(),
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
                println!("Timer elapsed, server shutdown !");
            },

            // listen for incoming signals
            Some(task_signal) = watchdog_listen_channel.recv() => {
                match task_signal {
                    // a new client connected to the proxy
                    TaskSignal::New(task_notify_channel) => {
                        if !server_running {
                            server_running = true;
                            println!("Server started");
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

async fn handle_new_client(
    client_stream: TcpStream,
    server_addr: String,
    mut watchdog_notify_channel: mpsc::Sender<TaskSignal>,
) {
    println!("New connection !");

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
    let _ = proxy_stream(client_stream, server_addr).await;

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
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut server_stream = TcpStream::connect(server_addr).await?;

    let (mut read_client, mut write_client) = client_stream.split();
    let (mut read_server, mut write_server) = server_stream.split();

    tokio::select! {
        _ = io::copy(&mut read_client, &mut write_server) => {},
        _ = io::copy(&mut read_server, &mut write_client) => {},
    }

    Ok(())
}

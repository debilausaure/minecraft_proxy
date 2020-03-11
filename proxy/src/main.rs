#![warn(rust_2018_idioms)]

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::time;

use std::env;
use std::error::Error;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // panics if no listen addr or server addr is provided
    let listen_addr = env::args()
        .nth(1)
        .expect("You must provide an address to listen on...");
    let server_addr = env::args()
        .nth(2)
        .expect("You must provide an address to proxy to...");

    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let connections_counter = Arc::new(RwLock::new(0));

    let mut listener = TcpListener::bind(listen_addr).await?;

    while let Ok((client_stream, _)) = listener.accept().await {
        let new_client_future = handle_new_client(
            client_stream,
            server_addr.clone(),
            connections_counter.clone(),
        );

        // create a new task that can be run in parallel with other tasks
        tokio::spawn(new_client_future);
    }

    Ok(())
}

async fn handle_new_client(
    client_stream: TcpStream,
    server_addr: String,
    connections_counter: Arc<RwLock<usize>>,
) {
    println!("New connection !");

    if *connections_counter.read().await == 0 {
        println!("Starting the server ! Please bear with us...");
    }

    // These two branches will be run concurrently, but not in parallel
    let inc_counter_future = increment_counter(&connections_counter);
    let proxy_future = proxy_stream(client_stream, server_addr);

    println!("Proxying the new connection to the server...");

    // Only try to decrement the counter when both branches completed
    let _ = tokio::join!(proxy_future, inc_counter_future);

    println!("Connection closed !");

    // if there are no more clients, start a shutdown countdown
    if *connections_counter.read().await == 1 {
        println!("No connections left, starting 1m timer...");
        time::delay_for(time::Duration::new(60, 0)).await;
        println!("Server shutdown...");
    }

    decrement_counter(&connections_counter).await;
}

async fn increment_counter(connections_counter: &Arc<RwLock<usize>>) {
    let mut counter = connections_counter.write().await;
    *counter += 1;
}

async fn decrement_counter(connections_counter: &Arc<RwLock<usize>>) {
    let mut counter = connections_counter.write().await;
    *counter -= 1;
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

#![warn(rust_2018_idioms)]

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use futures::future::try_select;
use futures::TryFutureExt;
use futures::FutureExt;

use std::env;
use std::error::Error;
use std::sync::Arc;


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {

    // panics if no listen addr or server addr is provided
    let listen_addr = env::args().nth(1).unwrap();
    let server_addr = env::args().nth(2).unwrap();

    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let connections_counter = Arc::new(RwLock::new(0));

    let mut listener = TcpListener::bind(listen_addr).await?;

    while let Ok((client_stream, _)) = listener.accept().await {
        let proxy_stream = proxy_stream(client_stream, server_addr.clone(), connections_counter.clone()).map(|result| {
            if let Err(e) = result {
                println!("An error occured : {}", e);
            }
        });

        tokio::spawn(proxy);
    }

    Ok(())
}

// this proxies the stream to the server
// however counting connections this way is bad because the function might fail and we would never
// decrease the counter FIXME
async fn proxy_stream(mut client_stream: TcpStream, server_addr: String, connections_counter: Arc<RwLock<usize>>) -> Result<(), Box<dyn Error>> {

    let mut server_stream = TcpStream::connect(server_addr).await?;

    // acquiring the lock on the counter
    {
        let mut counter = connections_counter.write().await;
        *counter += 1;
    }

    println!("New connection opened !");

    let (mut read_client, mut write_client) = client_stream.split();
    let (mut read_server, mut write_server) = server_stream.split();

    let client_to_proxy = io::copy(&mut read_client, &mut write_server);
    let proxy_to_server = io::copy(&mut read_server, &mut write_client);

    try_select(client_to_proxy, proxy_to_server).map_err(|e| {
        match e {
            futures::future::Either::Left((e, _)) => e,
            futures::future::Either::Right((e, _)) => e,
        }
    }).await?;

    //acquiring the lock on the counter
    {
        let mut counter = connections_counter.write().await;
        *counter -= 1;
    }

    println!("Connection closed.");

    Ok(())
}

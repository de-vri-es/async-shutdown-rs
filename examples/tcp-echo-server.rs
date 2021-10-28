use async_shutdown::Shutdown;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() {
	// Create a new shutdown object.
	// We will clone it into all tasks that need it.
	let shutdown = Shutdown::new();

	// Spawn a task to wait for CTRL+C and trigger a shutdown.
	tokio::spawn({
		let shutdown = shutdown.clone();
		async move {
			if let Err(e) = tokio::signal::ctrl_c().await {
				eprintln!("Failed to wait for CTRL+C: {}", e);
				std::process::exit(1);
			} else {
				eprintln!("\nReceived interrupt signal. Shutting down server...");
				shutdown.shutdown();
			}
		}
	});

	// Run the server and set a non-zero exit code if we had an error.
	let exit_code = match run_server(shutdown.clone(), "[::]:9372").await {
		Ok(()) => 0,
		Err(e) => {
			eprintln!("Server task finished with an error: {}", e);
			1
		},
	};

	// Wait for clients to run their cleanup code, then exit.
	// Without this, background tasks could be killed before they can run their cleanup code.
	shutdown.wait_shutdown_complete().await;

	std::process::exit(exit_code);
}

async fn run_server(shutdown: Shutdown, bind_address: &str) -> std::io::Result<()> {
	let server = TcpListener::bind(&bind_address).await?;
	eprintln!("Server listening on {}", bind_address);

	// Simply use `wrap_cancel` for everything, since we do not need clean-up for the listening socket.
	while let Some(connection) = shutdown.wrap_cancel(server.accept()).await {
		let (stream, address) = connection?;
		tokio::spawn(handle_client(shutdown.clone(), stream, address));
	}

	Ok(())
}

async fn handle_client(shutdown: Shutdown, mut stream: TcpStream, address: SocketAddr) {
	eprintln!("Accepted new connection from {}", address);

	// Create a future that can be cancelled, but still prints what happend afterwards.
	let work = {
		let shutdown = shutdown.clone();
		async move {
			match shutdown.wrap_cancel(echo_loop(&mut stream)).await {
				Some(Err(e)) => eprintln!("Error in connection {}: {}", address, e),
				Some(Ok(())) => eprintln!("Connection closed by {}", address),
				None => eprintln!("Shutdown triggered, closing connection with {}", address),
			}
		}
	};

	// Make sure the shutdown doesn't complete until the `work` future finishes.
	//
	// Wrapping the future will fail if the shutdown has already started,
	// in which case we just log a message and drop the future instead of awaiting it.
	match shutdown.wrap_wait(work) {
		Err(_) => eprintln!("Shutdown already started, closing connection with {}", address),
		Ok(work) => work.await,
	}
}

async fn echo_loop(stream: &mut TcpStream) -> std::io::Result<()> {
	// Echo everything we receive back to the peer in a loop.
	let mut buffer = vec![0; 512];
	loop {
		let read = stream.read(&mut buffer).await?;
		if read == 0 {
			break;
		}
		stream.write(&buffer[..read]).await?;
	}

	Ok(())
}

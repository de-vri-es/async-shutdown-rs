# async-shutdown

Runtime agnostic one-stop solution for graceful shutdown in asynchronous code.

This crate addresses two separate but related problems regarding graceful shutdown:
* You have to be able to stop running futures when a shutdown signal is given.
* You have to be able to wait for futures to finish potential clean-up.
* You want to know why the shutdown was triggered (for example to set your process exit code).

All of these problems are handled by the [`ShutdownManager`] struct.

## Stopping running futures
You can get a future to wait for the shutdown signal with [`ShutdownManager::wait_shutdown_triggered()`].
In this case you must write your async code to react to the shutdown signal appropriately.

Alternatively, you can wrap a future to be cancelled (by being dropped) when the shutdown is triggered with [`ShutdownManager::wrap_cancel()`].
This doesn't require the wrapped future to know anything about the shutdown signal,
but it also doesn't allow the future to run custom shutdown code.

To trigger the shutdown signal, simply call [`ShutdownManager::trigger_shutdown(reason)`][`ShutdownManager::trigger_shutdown()`].
The shutdown reason can be any type, as long as it implements [`Clone`].
If you want to pass a non-[`Clone`] object or an object that is expensive to clone, you can wrap it in an [`Arc`].

## Waiting for futures to complete.
You may also want to wait for some futures to complete before actually shutting down instead of just dropping them.
This might be important to cleanly shutdown and prevent data loss.
You can do that with [`ShutdownManager::wait_shutdown_complete()`].
That function returns a future that only completes when the shutdown is "completed".

You must also prevent the shutdown from completing too early by calling [`ShutdownManager::delay_shutdown_token()`] or [`ShutdownManager::wrap_delay_shutdown()`].
The [`ShutdownManager::delay_shutdown_token()`] function gives you a [`DelayShutdownToken`] which prevents the shutdown from completing.
To allow the shutdown to finish, simply drop the token.
Alternatively, [`ShutdownManager::wrap_delay_shutdown()`] wraps an existing future,
and will prevent the shutdown from completing until the future either completes or is dropped.

Note that you can only delay the shutdown completion if it has not completed already.
If the shutdown is already complete those functions will return an error.

You can also use a token to wrap a future with [`DelayShutdownToken::wrap_future()`].
If you already have a token, this allows you to wrap a future without having to worry that the shutdown might already be completed.

## Automatically triggering shutdowns
You can also trigger a shutdown automatically using a [`TriggerShutdownToken`].
Call [`ShutdownManager::trigger_shutdown_token()`] to obtain the token.
When the token is dropped, a shutdown is triggered.

You can use [`ShutdownManager::wrap_trigger_shutdown()`] or [`TriggerShutdownToken::wrap_future()`] to wrap a future.
When the wrapped future completes (or when it is dropped) it will trigger a shutdown.
This can be used as a convenient way to trigger a shutdown when a vital task stops.

## Futures versus Tasks
Be careful when using `JoinHandles` as if they're a regular future.
Depending on your async runtime, when you drop a `JoinHandle` this doesn't normally cause the task to stop.
It may simply detach the join handle from the task, meaning that your task is still running.
If you're not careful, this could still cause data loss on shutdown.
As a rule of thumb, you should usually wrap futures *before* you spawn them on a new task.

## Example

This example is a tokio-based TCP echo server.
It simply echos everything it receives from a peer back to that same peer,
and it uses this crate for graceful shutdown.

This example is also available in the repository as under the name [`tcp-echo-server`] if you want to run it locally.

[`tcp-echo-server`]: https://github.com/de-vri-es/async-shutdown-rs/blob/main/examples/tcp-echo-server.rs

```rust
use async_shutdown::ShutdownManager;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() {
    // Create a new shutdown object.
    // We will clone it into all tasks that need it.
    let shutdown = ShutdownManager::new();

    // Spawn a task to wait for CTRL+C and trigger a shutdown.
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                eprintln!("Failed to wait for CTRL+C: {}", e);
                std::process::exit(1);
            } else {
                eprintln!("\nReceived interrupt signal. Shutting down server...");
                shutdown.trigger_shutdown(0).ok();
            }
        }
    });

    // Run the server and set a non-zero exit code if we had an error.
    let exit_code = match run_server(shutdown.clone(), "[::]:9372").await {
        Ok(()) => {
            shutdown.trigger_shutdown(0).ok();
        },
        Err(e) => {
            eprintln!("Server task finished with an error: {}", e);
            shutdown.trigger_shutdown(1).ok();
        },
    };

    // Wait for clients to run their cleanup code, then exit.
    // Without this, background tasks could be killed before they can run their cleanup code.
    let exit_code = shutdown.wait_shutdown_complete().await;

    std::process::exit(exit_code);
}

async fn run_server(shutdown: ShutdownManager<i32>, bind_address: &str) -> std::io::Result<()> {
    let server = TcpListener::bind(&bind_address).await?;
    eprintln!("Server listening on {}", bind_address);

    // Simply use `wrap_cancel` for everything, since we do not need clean-up for the listening socket.
    // See `handle_client` for a case where a future is given the time to perform logging after the shutdown was triggered.
    while let Ok(connection) = shutdown.wrap_cancel(server.accept()).await {
        let (stream, address) = connection?;
        tokio::spawn(handle_client(shutdown.clone(), stream, address));
    }

    Ok(())
}

async fn handle_client(shutdown: ShutdownManager<i32>, mut stream: TcpStream, address: SocketAddr) {
    eprintln!("Accepted new connection from {}", address);

    // Make sure the shutdown doesn't complete until the delay token is dropped.
    //
    // Getting the token will fail if the shutdown has already started,
    // in which case we just log a message and return.
    //
    // If you already have a future that should be allowed to complete,
    // you can also use `shutdown.wrap_delay_shutdown(...)`.
    // Here it is easier to use a token though.
    let _delay_token = match shutdown.delay_shutdown_token() {
        Ok(token) => token,
        Err(_) => {
            eprintln!("Shutdown already started, closing connection with {}", address);
            return;
        }
    };

    // Now run the echo loop, but cancel it when the shutdown is triggered.
    match shutdown.wrap_cancel(echo_loop(&mut stream)).await {
        Ok(Err(e)) => eprintln!("Error in connection {}: {}", address, e),
        Ok(Ok(())) => eprintln!("Connection closed by {}", address),
        Err(_exit_code) => eprintln!("Shutdown triggered, closing connection with {}", address),
    }

    // The delay token will be dropped here, allowing the shutdown to complete.
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
```

[`ShutdownManager`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html
[`ShutdownManager::wait_shutdown_triggered()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.wait_shutdown_triggered
[`ShutdownManager::wrap_cancel()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.wrap_cancel
[`ShutdownManager::trigger_shutdown()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.trigger_shutdown
[`Clone`]: https://doc.rust-lang.org/stable/std/clone/trait.Clone.html
[`Arc`]: https://doc.rust-lang.org/stable/std/sync/struct.Arc.html
[`ShutdownManager::wait_shutdown_complete()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.wait_shutdown_complete
[`ShutdownManager::delay_shutdown_token()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.delay_shutdown_token
[`ShutdownManager::wrap_delay_shutdown()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.wrap_delay_shutdown
[`DelayShutdownToken`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.DelayShutdownToken.html
[`DelayShutdownToken::wrap_future()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.DelayShutdownToken.html#method.wrap_future
[`TriggerShutdownToken`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.TriggerShutdownToken.html
[`ShutdownManager::trigger_shutdown_token()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.trigger_shutdown_token
[`ShutdownManager::wrap_trigger_shutdown()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.ShutdownManager.html#method.wrap_trigger_shutdown
[`TriggerShutdownToken::wrap_future()`]: https://docs.rs/async-shutdown/latest/async_shutdown/struct.TriggerShutdownToken.html#method.wrap_future

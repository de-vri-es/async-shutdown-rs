//! Runtime agnostic one-stop solution for graceful shutdown in asynchronous code.
//!
//! This crate addresses two separate but related problems regarding graceful shutdown:
//! * You have to be able to stop running futures when a shutdown signal is given.
//! * You have to be able to wait for futures to finish potential clean-up.
//! * You want to know why the shutdown was triggered (for example to set your process exit code).
//!
//! All of these problems are handled by the [`ShutdownManager`] struct.
//!
//! # Stopping running futures
//! You can get a future to wait for the shutdown signal with [`ShutdownManager::wait_shutdown_triggered()`].
//! In this case you must write your async code to react to the shutdown signal appropriately.
//!
//! Alternatively, you can wrap a future to be cancelled (by being dropped) when the shutdown is triggered with [`ShutdownManager::wrap_cancel()`].
//! This doesn't require the wrapped future to know anything about the shutdown signal,
//! but it also doesn't allow the future to run custom shutdown code.
//!
//! To trigger the shutdown signal, simply call [`ShutdownManager::trigger_shutdown(reason)`][`ShutdownManager::trigger_shutdown()`].
//! The shutdown reason can be any type, as long as it implements [`Clone`].
//! If you want to pass a non-[`Clone`] object or an object that is expensive to clone, you can wrap it in an [`Arc`].
//!
//! # Waiting for futures to complete.
//! You may also want to wait for some futures to complete before actually shutting down instead of just dropping them.
//! This might be important to cleanly shutdown and prevent data loss.
//! You can do that with [`ShutdownManager::wait_shutdown_complete()`].
//! That function returns a future that only completes when the shutdown is "completed".
//!
//! You must also prevent the shutdown from completing too early by calling [`ShutdownManager::delay_shutdown_token()`] or [`ShutdownManager::wrap_delay_shutdown()`].
//! The [`ShutdownManager::delay_shutdown_token()`] function gives you a [`DelayShutdownToken`] which prevents the shutdown from completing.
//! To allow the shutdown to finish, simply drop the token.
//! Alternatively, [`ShutdownManager::wrap_delay_shutdown()`] wraps an existing future,
//! and will prevent the shutdown from completing until the future either completes or is dropped.
//!
//! Note that you can only delay the shutdown completion if it has not completed already.
//! If the shutdown is already complete those functions will return an error.
//!
//! You can also use a token to wrap a future with [`DelayShutdownToken::wrap_future()`].
//! If you already have a token, this allows you to wrap a future without having to worry that the shutdown might already be completed.
//!
//! # Automatically triggering shutdowns
//! You can also trigger a shutdown automatically using a [`TriggerShutdownToken`].
//! Call [`ShutdownManager::trigger_shutdown_token()`] to obtain the token.
//! When the token is dropped, a shutdown is triggered.
//!
//! You can use [`ShutdownManager::wrap_trigger_shutdown()`] or [`TriggerShutdownToken::wrap_future()`] to wrap a future.
//! When the wrapped future completes (or when it is dropped) it will trigger a shutdown.
//! This can be used as a convenient way to trigger a shutdown when a vital task stops.
//!
//! # Futures versus Tasks
//! Be careful when using `JoinHandles` as if they're a regular future.
//! Depending on your async runtime, when you drop a `JoinHandle` this doesn't normally cause the task to stop.
//! It may simply detach the join handle from the task, meaning that your task is still running.
//! If you're not careful, this could still cause data loss on shutdown.
//! As a rule of thumb, you should usually wrap futures *before* you spawn them on a new task.
//!
//! # Example
//!
//! This example is a tokio-based TCP echo server.
//! It simply echos everything it receives from a peer back to that same peer,
//! and it uses this crate for graceful shutdown.
//!
//! This example is also available in the repository as under the name [`tcp-echo-server`] if you want to run it locally.
//!
//! [`tcp-echo-server`]: https://github.com/de-vri-es/async-shutdown-rs/blob/main/examples/tcp-echo-server.rs
//!
//! ```no_run
//! use async_shutdown::ShutdownManager;
//! use std::net::SocketAddr;
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//! use tokio::net::{TcpListener, TcpStream};
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create a new shutdown object.
//!     // We will clone it into all tasks that need it.
//!     let shutdown = ShutdownManager::new();
//!
//!     // Spawn a task to wait for CTRL+C and trigger a shutdown.
//!     tokio::spawn({
//!         let shutdown = shutdown.clone();
//!         async move {
//!             if let Err(e) = tokio::signal::ctrl_c().await {
//!                 eprintln!("Failed to wait for CTRL+C: {}", e);
//!                 std::process::exit(1);
//!             } else {
//!                 eprintln!("\nReceived interrupt signal. Shutting down server...");
//!                 shutdown.trigger_shutdown(0).ok();
//!             }
//!         }
//!     });
//!
//!     // Run the server and set a non-zero exit code if we had an error.
//!     let exit_code = match run_server(shutdown.clone(), "[::]:9372").await {
//!         Ok(()) => {
//!             shutdown.trigger_shutdown(0).ok();
//!         },
//!         Err(e) => {
//!             eprintln!("Server task finished with an error: {}", e);
//!             shutdown.trigger_shutdown(1).ok();
//!         },
//!     };
//!
//!     // Wait for clients to run their cleanup code, then exit.
//!     // Without this, background tasks could be killed before they can run their cleanup code.
//!     let exit_code = shutdown.wait_shutdown_complete().await;
//!
//!     std::process::exit(exit_code);
//! }
//!
//! async fn run_server(shutdown: ShutdownManager<i32>, bind_address: &str) -> std::io::Result<()> {
//!     let server = TcpListener::bind(&bind_address).await?;
//!     eprintln!("Server listening on {}", bind_address);
//!
//!     // Simply use `wrap_cancel` for everything, since we do not need clean-up for the listening socket.
//!     // See `handle_client` for a case where a future is given the time to perform logging after the shutdown was triggered.
//!     while let Ok(connection) = shutdown.wrap_cancel(server.accept()).await {
//!         let (stream, address) = connection?;
//!         tokio::spawn(handle_client(shutdown.clone(), stream, address));
//!     }
//!
//!     Ok(())
//! }
//!
//! async fn handle_client(shutdown: ShutdownManager<i32>, mut stream: TcpStream, address: SocketAddr) {
//!     eprintln!("Accepted new connection from {}", address);
//!
//!     // Make sure the shutdown doesn't complete until the delay token is dropped.
//!     //
//!     // Getting the token will fail if the shutdown has already started,
//!     // in which case we just log a message and return.
//!     //
//!     // If you already have a future that should be allowed to complete,
//!     // you can also use `shutdown.wrap_delay_shutdown(...)`.
//!     // Here it is easier to use a token though.
//!     let _delay_token = match shutdown.delay_shutdown_token() {
//!         Ok(token) => token,
//!         Err(_) => {
//!             eprintln!("Shutdown already started, closing connection with {}", address);
//!             return;
//!         }
//!     };
//!
//!     // Now run the echo loop, but cancel it when the shutdown is triggered.
//!     match shutdown.wrap_cancel(echo_loop(&mut stream)).await {
//!         Ok(Err(e)) => eprintln!("Error in connection {}: {}", address, e),
//!         Ok(Ok(())) => eprintln!("Connection closed by {}", address),
//!         Err(_exit_code) => eprintln!("Shutdown triggered, closing connection with {}", address),
//!     }
//!
//!     // The delay token will be dropped here, allowing the shutdown to complete.
//! }
//!
//! async fn echo_loop(stream: &mut TcpStream) -> std::io::Result<()> {
//!     // Echo everything we receive back to the peer in a loop.
//!     let mut buffer = vec![0; 512];
//!     loop {
//!         let read = stream.read(&mut buffer).await?;
//!         if read == 0 {
//!             break;
//!         }
//!         stream.write(&buffer[..read]).await?;
//!     }
//!
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::Waker;

mod shutdown_complete;
pub use shutdown_complete::ShutdownComplete;

mod shutdown_signal;
pub use shutdown_signal::ShutdownSignal;

mod wrap_cancel;
pub use wrap_cancel::WrapCancel;

mod wrap_trigger_shutdown;
pub use wrap_trigger_shutdown::WrapTriggerShutdown;

mod wrap_delay_shutdown;
pub use wrap_delay_shutdown::WrapDelayShutdown;

/// Shutdown manager for asynchronous tasks and futures.
///
/// The shutdown manager allows you to:
/// * Signal futures to shutdown or forcibly cancel them (by dropping them).
/// * Wait for futures to perform their clean-up after a shutdown was triggered.
/// * Retrieve the shutdown reason after the shutdown was triggered.
///
/// The shutdown manager can be cloned and shared with multiple tasks.
/// Each clone uses the same internal state.
#[derive(Clone)]
pub struct ShutdownManager<T: Clone> {
	inner: Arc<Mutex<ShutdownManagerInner<T>>>,
}

impl<T: Clone> ShutdownManager<T> {
	/// Create a new shutdown manager.
	#[inline]
	pub fn new() -> Self {
		Self {
			inner: Arc::new(Mutex::new(ShutdownManagerInner::new())),
		}
	}

	/// Check if the shutdown has been triggered.
	#[inline]
	pub fn is_shutdown_triggered(&self) -> bool {
		self.inner.lock().unwrap().shutdown_reason.is_some()
	}

	/// Check if the shutdown has completed.
	#[inline]
	pub fn is_shutdown_completed(&self) -> bool {
		let inner = self.inner.lock().unwrap();
		inner.shutdown_reason.is_some() && inner.delay_tokens == 0
	}

	/// Get the shutdown reason, if the shutdown has been triggered.
	///
	/// Returns [`None`] if the shutdown has not been triggered yet.
	#[inline]
	pub fn shutdown_reason(&self) -> Option<T> {
		self.inner.lock().unwrap().shutdown_reason.clone()
	}

	/// Asynchronously wait for the shutdown to be triggered.
	///
	/// This returns a future that completes when the shutdown is triggered.
	/// The future can be cloned and sent to other threads or tasks freely.
	///
	/// If the shutdown is already triggered, the returned future immediately resolves.
	///
	/// You can also use `ShutdownSignal::wrap_cancel()` of the returned object
	/// to automatically cancel a future when the shutdown signal is received.
	/// This is identical to `Self::wrap_cancel()`.
	#[inline]
	pub fn wait_shutdown_triggered(&self) -> ShutdownSignal<T> {
		ShutdownSignal {
			inner: self.inner.clone(),
		}
	}

	/// Asynchronously wait for the shutdown to complete.
	///
	/// This returns a future that completes when the shutdown is complete.
	/// The future can be cloned and sent to other threads or tasks freely.
	///
	/// The shutdown is complete when all [`DelayShutdownToken`] are dropped
	/// and all [`WrapDelayShutdown`] futures have completed or are dropped.
	#[inline]
	pub fn wait_shutdown_complete(&self) -> ShutdownComplete<T> {
		ShutdownComplete {
			inner: self.inner.clone(),
		}
	}

	/// Trigger the shutdown.
	///
	/// This will cause all [`ShutdownSignal`] and [`WrapCancel`] futures associated with this shutdown manager to be resolved.
	///
	/// The shutdown will not be considered complete until all [`DelayShutdownTokens`][DelayShutdownToken] are dropped.
	///
	/// If the shutdown was already started, this function returns an error.
	#[inline]
	pub fn trigger_shutdown(&self, reason: T) -> Result<(), ShutdownAlreadyStarted<T>> {
		self.inner.lock().unwrap().shutdown(reason)
	}

	/// Wrap a future so that it is cancelled (dropped) when the shutdown is triggered.
	///
	/// The returned future completes with `Err(shutdown_reason)` if the shutdown is triggered,
	/// and with `Ok(x)` if the wrapped future completes first.
	#[inline]
	pub fn wrap_cancel<F: Future>(&self, future: F) -> WrapCancel<T, F> {
		self.wait_shutdown_triggered().wrap_cancel(future)
	}

	/// Wrap a future to cause a shutdown when the future completes or when it is dropped.
	#[inline]
	pub fn wrap_trigger_shutdown<F: Future>(&self, shutdown_reason: T, future: F) -> WrapTriggerShutdown<T, F> {
		self.trigger_shutdown_token(shutdown_reason).wrap_future(future)
	}

	/// Wrap a future to delay shutdown completion until the wrapped future completes or until it is dropped.
	///
	/// The returned future transparently completes with the value of the wrapped future.
	/// However, the shutdown will not be considered complete until the wrapped future completes or is dropped.
	///
	/// If the shutdown has already completed, this function returns an error.
	#[inline]
	pub fn wrap_delay_shutdown<F: Future>(&self, future: F) -> Result<WrapDelayShutdown<T, F>, ShutdownAlreadyCompleted<T>> {
		Ok(self.delay_shutdown_token()?.wrap_future(future))
	}

	/// Get a token that delays shutdown completion as long as it exists.
	///
	/// The manager keeps track of all the tokens it hands out.
	/// The tokens can be cloned and sent to different threads and tasks.
	/// All tokens (including the clones) must be dropped before the shutdown is considered to be complete.
	///
	/// If the shutdown has already completed, this function returns an error.
	///
	/// If you want to delay the shutdown until a future completes,
	/// consider using [`Self::wrap_delay_shutdown()`] instead.
	#[inline]
	pub fn delay_shutdown_token(&self) -> Result<DelayShutdownToken<T>, ShutdownAlreadyCompleted<T>> {
		let mut inner = self.inner.lock().unwrap();
		// Shutdown already completed, can't delay completion anymore.
		if inner.delay_tokens == 0 {
			if let Some(reason) = &inner.shutdown_reason {
				return Err(ShutdownAlreadyCompleted::new(reason.clone()));
			}
		}

		inner.increase_delay_count();
		Ok(DelayShutdownToken {
			inner: self.inner.clone(),
		})
	}

	/// Get a token that triggers a shutdown when dropped.
	///
	/// When a [`TriggerShutdownToken`] is dropped, the shutdown is triggered automatically.
	/// This applies to *any* token.
	/// If you clone a token five times and drop one of them, it will trigger a shutdown/
	///
	/// You can also use [`Self::wrap_trigger_shutdown()`] to wrap a future so that a shutdown is triggered
	/// when the future completes or if it is dropped.
	#[inline]
	pub fn trigger_shutdown_token(&self, shutdown_reason: T) -> TriggerShutdownToken<T> {
		TriggerShutdownToken {
			shutdown_reason: Arc::new(Mutex::new(Some(shutdown_reason))),
			inner: self.inner.clone(),
		}
	}
}

impl<T: Clone> Default for ShutdownManager<T> {
	#[inline]
	fn default() -> Self {
		Self::new()
	}
}

/// Token that delays shutdown completion as long as it exists.
///
/// The token can be cloned and sent to different threads and tasks freely.
///
/// All clones must be dropped before the shutdown can complete.
pub struct DelayShutdownToken<T: Clone> {
	inner: Arc<Mutex<ShutdownManagerInner<T>>>,
}

impl<T: Clone> DelayShutdownToken<T> {
	/// Wrap a future to delay shutdown completion until the wrapped future completes or until it is dropped.
	///
	/// This consumes the token to avoid keeping an unused token around by accident, which would delay shutdown indefinitely.
	/// If you wish to use the token multiple times, you can clone it first:
	/// ```
	/// # let shutdown = async_shutdown::ShutdownManager::<()>::new();
	/// # let delay_shutdown_token = shutdown.delay_shutdown_token().unwrap();
	/// # let future = async { () };
	/// let future = delay_shutdown_token.clone().wrap_future(future);
	/// ```
	///
	/// The returned future transparently completes with the value of the wrapped future.
	/// However, the shutdown will not be considered complete until the future completes or is dropped.
	#[inline]
	pub fn wrap_future<F: Future>(self, future: F) -> WrapDelayShutdown<T, F> {
		WrapDelayShutdown {
			delay_token: Some(self),
			future,
		}
	}
}

impl<T: Clone> Clone for DelayShutdownToken<T> {
	#[inline]
	fn clone(&self) -> Self {
		self.inner.lock().unwrap().increase_delay_count();
		DelayShutdownToken {
			inner: self.inner.clone(),
		}
	}
}

impl<T: Clone> Drop for DelayShutdownToken<T> {
	#[inline]
	fn drop(&mut self) {
		self.inner.lock().unwrap().decrease_delay_count();
	}
}

/// Token that triggers a shutdown when it is dropped.
///
/// The token can be cloned and sent to different threads and tasks freely.
/// If *one* of the cloned tokens is dropped, a shutdown is triggered.
/// Even if the rest of the clones still exist.
#[derive(Clone)]
pub struct TriggerShutdownToken<T: Clone> {
	shutdown_reason: Arc<Mutex<Option<T>>>,
	inner: Arc<Mutex<ShutdownManagerInner<T>>>,
}

impl<T: Clone> TriggerShutdownToken<T> {
	/// Wrap a future to trigger a shutdown when it completes or is dropped.
	///
	/// This consumes the token to avoid accidentally dropping the token
	/// after wrapping a future and instantly causing a shutdown.
	///
	/// If you need to keep the token around, you can clone it first:
	/// ```
	/// # let trigger_shutdown_token = async_shutdown::ShutdownManager::new().trigger_shutdown_token(());
	/// # let future = async { () };
	/// let future = trigger_shutdown_token.clone().wrap_future(future);
	/// ```
	#[inline]
	pub fn wrap_future<F: Future>(self, future: F) -> WrapTriggerShutdown<T, F> {
		WrapTriggerShutdown {
			trigger_shutdown_token: Some(self),
			future,
		}
	}

	/// Drop the token without causing a shutdown.
	///
	/// This is equivalent to calling [`std::mem::forget()`] on the token.
	#[inline]
	pub fn forget(self) {
		std::mem::forget(self)
	}
}

impl<T: Clone> Drop for TriggerShutdownToken<T> {
	#[inline]
	fn drop(&mut self) {
		let mut inner = self.inner.lock().unwrap();
		let reason = self.shutdown_reason.lock().unwrap().take();
		if let Some(reason) = reason {
			inner.shutdown(reason).ok();
		}
	}
}

struct ShutdownManagerInner<T> {
	/// The shutdown reason.
	shutdown_reason: Option<T>,

	/// Number of delay tokens in existence.
	///
	/// Must reach 0 before shutdown can complete.
	delay_tokens: usize,

	/// Tasks to wake when a shutdown is triggered.
	on_shutdown: Vec<Waker>,

	/// Tasks to wake when the shutdown is complete.
	on_shutdown_complete: Vec<Waker>,
}

impl<T: Clone> ShutdownManagerInner<T> {
	fn new() -> Self {
		Self {
			shutdown_reason: None,
			delay_tokens: 0,
			on_shutdown_complete: Vec::new(),
			on_shutdown: Vec::new(),
		}
	}

	fn increase_delay_count(&mut self) {
		self.delay_tokens += 1;
	}

	fn decrease_delay_count(&mut self) {
		self.delay_tokens -= 1;
		if self.delay_tokens == 0 {
			self.notify_shutdown_complete();
		}
	}

	fn shutdown(&mut self, reason: T) -> Result<(), ShutdownAlreadyStarted<T>> {
		match &self.shutdown_reason {
			Some(original_reason) => {
				Err(ShutdownAlreadyStarted::new(original_reason.clone(), reason))
			},
			None => {
				self.shutdown_reason = Some(reason);
				for abort in std::mem::take(&mut self.on_shutdown) {
					abort.wake()
				}
				if self.delay_tokens == 0 {
					self.notify_shutdown_complete()
				}
				Ok(())
			},
		}
	}

	fn notify_shutdown_complete(&mut self) {
		for waiter in std::mem::take(&mut self.on_shutdown_complete) {
			waiter.wake()
		}
	}
}

/// Error returned when you try to trigger the shutdown multiple times on the same [`ShutdownManager`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownAlreadyStarted<T> {
	/// The shutdown reason of the already started shutdown.
	pub shutdown_reason: T,

	/// The provided reason that was ignored because the shutdown was already started.
	pub ignored_reason: T,
}

impl<T> ShutdownAlreadyStarted<T> {
	pub(crate) const fn new(shutdown_reason: T, ignored_reason:T ) -> Self {
		Self { shutdown_reason, ignored_reason }
	}
}

impl<T: std::fmt::Debug> std::error::Error for ShutdownAlreadyStarted<T> {}

impl<T> std::fmt::Display for ShutdownAlreadyStarted<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "shutdown has already started, can not delay shutdown completion")
	}
}

/// Error returned when trying to delay a shutdown that has already completed.
#[derive(Debug)]
#[non_exhaustive]
pub struct ShutdownAlreadyCompleted<T> {
	/// The shutdown reason of the already completed shutdown.
	pub shutdown_reason: T,
}

impl<T> ShutdownAlreadyCompleted<T> {
	pub(crate) const fn new(shutdown_reason: T) -> Self {
		Self { shutdown_reason }
	}
}

impl<T: std::fmt::Debug> std::error::Error for ShutdownAlreadyCompleted<T> {}

impl<T> std::fmt::Display for ShutdownAlreadyCompleted<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "shutdown has already completed, can not delay shutdown completion")
	}
}

//! One-stop solution for graceful shutdown in asynchronous code.
//!
//! This crate addresses two separate but related problems regarding graceful shutdown:
//! * You have to be able to stop running futures when a shutdown signal is given.
//! * You have to be able to wait for futures to finish potential clean-up.
//!
//! Both issues are handled by the [`Shutdown`] struct.
//!
//! # Stopping running futures
//! To stop running futures, you can get a future to wait for the shutdown signal with [`Shutdown::wait_for_shutdown_signal()`].
//! In this case you must write your async code to react to the shutdown signal appropriately.
//!
//! Alternatively, you can wrap a future to be cancelled (by being dropped) when the shutdown starts with [`Shutdown::cancel_on_shutdown()`] or [`ShutdownSignal::cancel_on_shutdown()`].
//! This doesn't require the wrapped future to know anything about the shutdown signal,
//! but it also doesn't allow the future to run custom shutdown code.
//!
//! To trigger the shutdown signal, simply call [`Shutdown::shutdown()`].
//!
//! # Waiting for futures to complete.
//! If you have futures that run custom shutdown code (as opposed to just dropping the futures),
//! you will want to wait for that cleanup code to finish.
//! You can do that with [`Shutdown::wait_for_shutdown_complete()`].
//! That function returns a future that only completes when the shutdown is "completed".
//!
//! You must also prevent the shutdown from completing too early by calling [`Shutdown::delay_shutdown_token()`] or [`Shutdown::delay_shutdown_for()`].
//! Note that this can only be done before a shutdown has completed.
//! If the shutdown is already complete those functions will return an error.
//!
//! The [`Shutdown::delay_shutdown_token()`] function gives you a [`DelayShutdownToken`] which prevents the shutdown from completing.
//! To allow the shutdown to finish, simply drop the token.
//! Alternatively, [`Shutdown::delay_shutdown_for()`] wraps an existing future,
//! and will prevent the shutdown from completing until the future either completes or is dropped.
//!
//! You can also use a token to wrap a future with [`DelayShutdownToken::delay_shutdown_for()`].
//! This has the advantage that it can never fail:
//! the fact that you have a token means the shutdown has not finished yet.
//! ```

#![warn(missing_docs)]

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::Waker;

mod delay_shutdown;
pub use delay_shutdown::DelayShutdown;

mod cancel_on_shutdown;
pub use cancel_on_shutdown::CancelOnShutdown;

mod shutdown_complete;
pub use shutdown_complete::ShutdownComplete;

mod shutdown_signal;
pub use shutdown_signal::ShutdownSignal;

/// Shutdown manager for asynchronous tasks and futures.
///
/// The shutdown manager serves two separate but related purposes:
/// * It can signal futures to shutdown or forcibly cancel them (by dropping them).
/// * It can wait for wait for futures to do their clean-up.
///
/// The shutdown manager can be cloned and shared with multiple tasks.
/// Each clone uses the same internal state.
pub struct Shutdown {
	inner: Arc<Mutex<ShutdownInner>>,
}

impl Shutdown {
	/// Create a new shutdown manager.
	#[inline]
	pub fn new() -> Self {
		Self {
			inner: Arc::new(Mutex::new(ShutdownInner::new())),
		}
	}

	/// Check if the shutdown has been started.
	pub fn shutdown_started(&self) -> bool {
		self.inner.lock().unwrap().shutdown
	}

	/// Check if the shutdown has been completed.
	pub fn shutdown_completed(&self) -> bool {
		let inner = self.inner.lock().unwrap();
		inner.shutdown && inner.delay_tokens == 0
	}

	/// Asynchronously wait for a shutdown to be triggered.
	///
	/// This returns a future that completes when a shutdown is triggered.
	/// The future can be cloned and sent to other threads or tasks freely.
	///
	/// If a shutdown is already triggered, the returned future immediately resolves.
	#[inline]
	pub fn wait_for_shutdown_signal(&self) -> ShutdownSignal {
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
	/// and all [`DelayShutdown`] futures have completed.
	#[inline]
	pub fn wait_for_shutdown_complete(&self) -> ShutdownComplete {
		ShutdownComplete {
			inner: self.inner.clone(),
		}
	}

	/// Start the shutdown.
	///
	/// This will complete all [`ShutdownSignal`] and [`CancelOnShutdown`] futures associated with this shutdown manager.
	///
	/// The shutdown will not complete until all registered futures complete.
	///
	/// If the shutdown was already started, this function is a no-op.
	#[inline]
	pub fn shutdown(&self) {
		self.inner.lock().unwrap().shutdown();
	}

	/// Wrap a future so that it is cancelled when a shutdown is triggered.
	///
	/// The returned future completes with `None` when a shutdown is triggered,
	/// and with `Some(x)` when the wrapped future completes.
	///
	/// The wrapped future is dropped when the shutdown starts before the future completed.
	/// If the wrapped future completes before the shutdown signal arrives, it is not dropped.
	#[inline]
	pub fn cancel_on_shutdown<F: Future>(&self, future: F) -> CancelOnShutdown<F> {
		CancelOnShutdown {
			shutdown_signal: self.wait_for_shutdown_signal(),
			future: Some(future),
		}
	}

	/// Wrap a future to delay shutdown completion until it completes.
	///
	/// The returned future transparently completes with the value of the wrapped future.
	/// However, the shutdown will not be considered complete until the future completes or is dropped.
	///
	/// If the shutdown has already started, this function returns an error.
	#[inline]
	pub fn delay_shutdown_for<F: Future>(&self, future: F) -> Result<DelayShutdown<F>, ShutdownAlreadyCompleted> {
		Ok(DelayShutdown {
			delay_token: Some(self.delay_shutdown_token()?),
			future,
		})
	}

	/// Get a token to delay shutdown completion.
	///
	/// The manager keeps track of all the tokens it hands out.
	/// The tokens can be cloned and sent to different threads and tasks.
	/// All tokens (including the clones) must be dropped before the shutdown is considered to be complete.
	///
	/// If the shutdown has already started, this function returns an error.
	///
	/// If you want to delay the shutdown until a future completes,
	/// consider using [`Self::delay_shutdown_for()`] instead.
	#[inline]
	pub fn delay_shutdown_token(&self) -> Result<DelayShutdownToken, ShutdownAlreadyCompleted> {
		let mut inner = self.inner.lock().unwrap();
		// Shutdown already started, can't delay completion anymore.
		if inner.shutdown {
			Err(ShutdownAlreadyCompleted::new())
		} else {
			inner.increase_delay_count();
			Ok(DelayShutdownToken {
				inner: self.inner.clone(),
			})
		}
	}
}

impl Default for Shutdown {
	fn default() -> Self {
		Self::new()
	}
}

/// Token that delays completion of a shutdown as long as it exists.
///
/// The token can be cloned and sent to different threads and tasks freely.
///
/// All clones must be dropped before the shutdown can complete.
pub struct DelayShutdownToken {
	inner: Arc<Mutex<ShutdownInner>>,
}

impl DelayShutdownToken {
	/// Wrap a future to delay shutdown completion until it completes.
	///
	/// The returned future transparently completes with the value of the wrapped future.
	/// However, the shutdown will not be considered complete until the future completes or is dropped.
	#[inline]
	pub fn delay_shutdown_for<F: Future>(&self, future: F) -> DelayShutdown<F> {
		DelayShutdown {
			delay_token: Some(self.clone()),
			future,
		}
	}
}

impl Clone for DelayShutdownToken {
	fn clone(&self) -> Self {
		self.inner.lock().unwrap().increase_delay_count();
		DelayShutdownToken {
			inner: self.inner.clone(),
		}
	}
}

impl Drop for DelayShutdownToken {
	fn drop(&mut self) {
		self.inner.lock().unwrap().decrease_delay_count();
	}
}

struct ShutdownInner {
	/// Flag indicating if a shutdown is triggered.
	shutdown: bool,

	/// Number of delay tokens in existence.
	///
	/// Must reach 0 before shutdown can complete.
	delay_tokens: usize,

	/// Tasks to wake when a shutdown is triggered.
	on_shutdown: Vec<Waker>,

	/// Tasks to wake when the shutdown is complete.
	on_shutdown_complete: Vec<Waker>,
}

impl ShutdownInner {
	fn new() -> Self {
		Self {
			delay_tokens: 0,
			on_shutdown_complete: Vec::new(),
			shutdown: false,
			on_shutdown: Vec::new(),
		}
	}

	fn increase_delay_count(&mut self) {
		self.delay_tokens += 1;
	}

	fn decrease_delay_count(&mut self) {
		self.delay_tokens -= 1;
		if self.delay_tokens == 0 {
			for waiter in std::mem::take(&mut self.on_shutdown_complete) {
				waiter.wake()
			}
		}
	}

	fn shutdown(&mut self) {
		self.shutdown = true;
		for abort in std::mem::take(&mut self.on_shutdown) {
			abort.wake()
		}
	}
}

/// Error that occurs when trying to delay a shutdown that has already completed.
pub struct ShutdownAlreadyCompleted {
	_priv: (),
}

impl ShutdownAlreadyCompleted {
	pub(crate) const fn new() -> Self {
		Self {
			_priv: (),
		}
	}
}

impl std::fmt::Debug for ShutdownAlreadyCompleted {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "ShutdownAlreadyStarted")
	}
}

impl std::fmt::Display for ShutdownAlreadyCompleted {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "shutdown has already started, can not delay shutdown completion")
	}
}

impl std::error::Error for ShutdownAlreadyCompleted {}

//! Runtime agnostic one-stop solution for graceful shutdown in asynchronous code.
//!
//! This crate addresses two separate but related problems regarding graceful shutdown:
//! * You have to be able to stop running futures when a shutdown signal is given.
//! * You have to be able to wait for futures to finish potential clean-up.
//!
//! Both issues are handled by the [`Shutdown`] struct.
//!
//! # Stopping running futures
//! To stop running futures, you can get a future to wait for the shutdown signal with [`Shutdown::wait_shutdown()`].
//! In this case you must write your async code to react to the shutdown signal appropriately.
//!
//! Alternatively, you can wrap a future to be cancelled (by being dropped) when the shutdown starts with [`Shutdown::wrap_cancel()`].
//! This doesn't require the wrapped future to know anything about the shutdown signal,
//! but it also doesn't allow the future to run custom shutdown code.
//!
//! To trigger the shutdown signal, simply call [`Shutdown::shutdown()`].
//!
//! # Waiting for futures to complete.
//! If you have futures that run custom shutdown code (as opposed to just dropping the futures),
//! you will want to wait for that cleanup code to finish.
//! You can do that with [`Shutdown::wait_shutdown_complete()`].
//! That function returns a future that only completes when the shutdown is "completed".
//!
//! You must also prevent the shutdown from completing too early by calling [`Shutdown::delay_shutdown_token()`] or [`Shutdown::wrap_wait()`].
//! Note that this can only be done before a shutdown has completed.
//! If the shutdown is already complete those functions will return an error.
//!
//! The [`Shutdown::delay_shutdown_token()`] function gives you a [`DelayShutdownToken`] which prevents the shutdown from completing.
//! To allow the shutdown to finish, simply drop the token.
//! Alternatively, [`Shutdown::wrap_wait()`] wraps an existing future,
//! and will prevent the shutdown from completing until the future either completes or is dropped.
//!
//! You can also use a token to wrap a future with [`DelayShutdownToken::wrap_wait()`].
//! This has the advantage that it can never fail:
//! the fact that you have a token means the shutdown has not finished yet.
//!
//! # Automatically triggering shutdowns
//! You can also cause a shutdown when vital tasks or futures stop.
//! Call [`Shutdown::vital_token()`] to obtain a "vital" token.
//! When a vital token is dropped, a shutdown is triggered.
//!
//! You can also wrap a future to cause a shutdown on completion using [`Shutdown::wrap_vital()`] or [`VitalToken::wrap_vital()`].
//! This can be used as a convenient way to terminate all asynchronous tasks when a vital task stops.
//!
//! # Futures versus Tasks
//! Be careful when using `JoinHandles` as if they're a regular future.
//! Depending on your async runtime, when you drop a `JoinHandle` this doesn't necessarily cause the task to stop.
//! It may simply detach the join handle from the task, meaning that your task is still running.
//! If you're not careful, this could still cause data loss on shutdown.
//! As a rule of thumb, you should usually wrap futures *before* you spawn them on a new task.

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

mod wrap_vital;
pub use wrap_vital::WrapVital;

mod wrap_wait;
pub use wrap_wait::WrapWait;

/// Shutdown manager for asynchronous tasks and futures.
///
/// The shutdown manager serves two separate but related purposes:
/// * To signal futures to shutdown or forcibly cancel them (by dropping them).
/// * To wait for futures to perform their clean-up after a shutdown was triggered.
///
/// The shutdown manager can be cloned and shared with multiple tasks.
/// Each clone uses the same internal state.
#[derive(Clone)]
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
	///
	/// You can also use `ShutdownSignal::wrap_cancel()` of the returned object
	/// to automatically cancel a future when the shutdown signal is received.
	/// This is identical to `Self::wrap_cancel()`, but can be done if you only have a `ShutdownSignal`.
	#[inline]
	pub fn wait_shutdown(&self) -> ShutdownSignal {
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
	/// and all [`WrapWait`] futures have completed.
	#[inline]
	pub fn wait_shutdown_complete(&self) -> ShutdownComplete {
		ShutdownComplete {
			inner: self.inner.clone(),
		}
	}

	/// Start the shutdown.
	///
	/// This will complete all [`ShutdownSignal`] and [`WrapCancel`] futures associated with this shutdown manager.
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
	pub fn wrap_cancel<F: Future>(&self, future: F) -> WrapCancel<F> {
		self.wait_shutdown().wrap_cancel(future)
	}

	/// Wrap a future to cause a shutdown when it completes or is dropped.
	#[inline]
	pub fn wrap_vital<F: Future>(&self, future: F) -> WrapVital<F> {
		self.vital_token().wrap_vital(future)
	}

	/// Wrap a future to delay shutdown completion until it completes or is dropped.
	///
	/// The returned future transparently completes with the value of the wrapped future.
	/// However, the shutdown will not be considered complete until the future completes or is dropped.
	///
	/// If the shutdown has already completed, this function returns an error.
	#[inline]
	pub fn wrap_wait<F: Future>(&self, future: F) -> Result<WrapWait<F>, ShutdownAlreadyCompleted> {
		Ok(self.delay_shutdown_token()?.wrap_wait(future))
	}

	/// Get a token to delay shutdown completion.
	///
	/// The manager keeps track of all the tokens it hands out.
	/// The tokens can be cloned and sent to different threads and tasks.
	/// All tokens (including the clones) must be dropped before the shutdown is considered to be complete.
	///
	/// If the shutdown has already completed, this function returns an error.
	///
	/// If you want to delay the shutdown until a future completes,
	/// consider using [`Self::wrap_wait()`] instead.
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

	/// Get a token that represents a vital task or future.
	///
	/// When a [`VitalToken`] is dropped, the shutdown is triggered automatically.
	/// This applies to *any* token.
	/// If you clone a token five times and drop one a shutdown is triggered,
	/// even though four tokens still exist.
	///
	/// You can also use [`Self::wrap_vital()`] to wrap a future so that a shutdown is triggered
	/// when the future completes or if it is dropped.
	#[inline]
	pub fn vital_token(&self) -> VitalToken {
		VitalToken {
			inner: self.inner.clone(),
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
	/// This consumes the token to avoid keeping an unused token around by accident, which would delay shutdown indefinately.
	/// If you wish to use the token multiple times, you can clone it first: `token.clone().wrap_wait(...)`.
	///
	/// The returned future transparently completes with the value of the wrapped future.
	/// However, the shutdown will not be considered complete until the future completes or is dropped.
	#[inline]
	pub fn wrap_wait<F: Future>(self, future: F) -> WrapWait<F> {
		WrapWait {
			delay_token: Some(self),
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

/// Token that triggers a shutdown when it is dropped.
///
/// The token can be cloned and sent to different threads and tasks freely.
/// If *one* of the cloned tokens is dropped, a shutdown is triggered.
/// Even if the the rest of the clones still exist.
#[derive(Clone)]
pub struct VitalToken {
	inner: Arc<Mutex<ShutdownInner>>,
}

impl VitalToken {
	/// Wrap a future to cause a shutdown when it completes or is dropped.
	///
	/// This consumes the token to avoid accidentally dropping the token
	/// after wrapping a future and instantly causing a shutdown.
	/// If you need to keep the token around, you can clone it first:
	/// ```no_compile
	/// let future = vital_token.clone().wrap_future(future);
	/// ```
	#[inline]
	pub fn wrap_vital<F: Future>(self, future: F) -> WrapVital<F> {
		WrapVital {
			vital_token: Some(self),
			future,
		}
	}

	/// Drop the token without causing a shutdown.
	///
	/// This is equivalent to calling [`std::mem::forget()`] on the token.
	pub fn forget(self) {
		std::mem::forget(self)
	}
}

impl Drop for VitalToken {
	fn drop(&mut self) {
		let mut inner = self.inner.lock().unwrap();
		inner.shutdown();
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
			self.notify_shutdown_complete();
		}
	}

	fn shutdown(&mut self) {
		self.shutdown = true;
		for abort in std::mem::take(&mut self.on_shutdown) {
			abort.wake()
		}
		if self.delay_tokens == 0 {
			self.notify_shutdown_complete()
		}
	}

	fn notify_shutdown_complete(&mut self) {
		for waiter in std::mem::take(&mut self.on_shutdown_complete) {
			waiter.wake()
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

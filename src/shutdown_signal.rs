use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::{waker_list::WakerToken, LockGuard, ShutdownManagerInner, WrapCancel};

/// A future to wait for a shutdown signal.
///
/// The future completes when the associated [`ShutdownManager`][crate::ShutdownManager] triggers a shutdown.
///
/// The shutdown signal can be cloned and sent between threads freely.
pub struct ShutdownSignal<T: Clone> {
	pub(crate) inner: LockGuard<ShutdownManagerInner<T>>,
	pub(crate) waker_token: Option<WakerToken>,
}

impl<T: Clone> Clone for ShutdownSignal<T> {
	fn clone(&self) -> Self {
		// Clone only the reference to the shutdown manager, not the waker token.
		// The waker token is personal to each future.
		Self {
			inner: self.inner.clone(),
			waker_token: None,
		}
	}
}

impl<T: Clone> Drop for ShutdownSignal<T> {
	fn drop(&mut self) {
		if let Some(token) = self.waker_token.take() {
			let mut inner = self.inner.borrow_mut();
			inner.on_shutdown.deregister(token);
		}
	}
}

impl<T: Clone> ShutdownSignal<T> {
	/// Wrap a future so that it is cancelled when a shutdown is triggered.
	///
	/// The returned future completes with `Err(reason)` containing the shutdown reason if a shutdown is triggered,
	/// and with `Ok(x)` when the wrapped future completes.
	///
	/// The wrapped future is dropped if the shutdown starts before the wrapped future completes.
	#[inline]
	pub fn wrap_cancel<F: Future>(&self, future: F) -> WrapCancel<T, F> {
		WrapCancel {
			shutdown_signal: self.clone(),
			future: Ok(future),
		}
	}
}

impl<T: Clone> Future for ShutdownSignal<T> {
	type Output = T;

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		let me = self.get_mut();
		let mut inner = me.inner.borrow_mut();

		// We're being polled, so we should deregister the waker (if any).
		if let Some(token) = me.waker_token.take() {
			inner.on_shutdown.deregister(token);
		}

		if let Some(reason) = inner.shutdown_reason.clone() {
			// Shutdown started, so we're ready.
			Poll::Ready(reason)
		} else {
			// We're not ready, so register the waker to wake us on shutdown start.
			me.waker_token = Some(inner.on_shutdown.register(context.waker().clone()));
			Poll::Pending
		}
	}
}

#[cfg(test)]
mod test {
	use assert2::assert;
	use std::future::Future;
	use std::pin::Pin;
	use std::task::Poll;

	/// Wrapper around a future to poll it only once.
	struct PollOnce<'a, F>(&'a mut F);

	impl<'a, F: std::marker::Unpin + Future> Future for PollOnce<'a, F> {
		type Output = Poll<F::Output>;

		fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
			Poll::Ready(Pin::new(&mut self.get_mut().0).poll(cx))
		}
	}

	/// Poll a future once.
	async fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
		PollOnce(future).await
	}

	#[tokio::test]
	async fn waker_list_doesnt_grow_infinitely() {
		let shutdown = crate::ShutdownManager::<()>::new();
		for i in 0..100_000 {
			#[cfg(feature = "multi-thread")]
			let task = tokio::task::spawn(shutdown.wrap_cancel(async move {
				tokio::task::yield_now().await;
			}));
			#[cfg(not(feature = "multi-thread"))]
			let ls = tokio::task::LocalSet::new();
			#[cfg(not(feature = "multi-thread"))]
			let task = ls.run_until(shutdown.wrap_cancel(async move {
				tokio::task::yield_now().await;
				tokio::io::Result::Ok(())
			}));
			assert!(let Ok(Ok(())) = task.await, "task = {i}");
		}

		// Since we wait for each task to complete before spawning another,
		// the total amount of waker slots used should be only 1.
		let inner = shutdown.inner.borrow();
		assert!(inner.on_shutdown.total_slots() == 1);
		assert!(inner.on_shutdown.empty_slots() == 1);
	}

	#[tokio::test]
	async fn cloning_does_not_clone_waker_token() {
		let shutdown = crate::ShutdownManager::<()>::new();

		let mut signal = shutdown.wait_shutdown_triggered();
		assert!(let None = &signal.waker_token);

		assert!(let Poll::Pending = poll_once(&mut signal).await);
		assert!(let Some(_) = &signal.waker_token);

		let mut cloned = signal.clone();
		assert!(let None = &cloned.waker_token);
		assert!(let Some(_) = &signal.waker_token);

		assert!(let Poll::Pending = poll_once(&mut cloned).await);
		assert!(let Some(_) = &cloned.waker_token);
		assert!(let Some(_) = &signal.waker_token);

		{
			let inner = shutdown.inner.borrow();
			assert!(inner.on_shutdown.total_slots() == 2);
			assert!(inner.on_shutdown.empty_slots() == 0);
		}

		{
			drop(signal);
			let inner = shutdown.inner.borrow();
			assert!(inner.on_shutdown.empty_slots() == 1);
		}

		{
			drop(cloned);
			let inner = shutdown.inner.borrow();
			assert!(inner.on_shutdown.empty_slots() == 2);
		}
	}
}

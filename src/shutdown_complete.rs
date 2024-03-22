use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::waker_list::WakerToken;
use crate::ShutdownManagerInner;

/// Future to wait for a shutdown to complete.
pub struct ShutdownComplete<T: Clone> {
	pub(crate) inner: Arc<Mutex<ShutdownManagerInner<T>>>,
	pub(crate) waker_token: Option<WakerToken>,
}

impl<T: Clone> Clone for ShutdownComplete<T> {
	fn clone(&self) -> Self {
		// Clone only the reference to the shutdown manager, not the waker token.
		// The waker token is personal to each future.
		Self {
			inner: self.inner.clone(),
			waker_token: None,
		}
	}
}

impl<T: Clone> Drop for ShutdownComplete<T> {
	fn drop(&mut self) {
		if let Some(token) = self.waker_token.take() {
			let mut inner = self.inner.lock().unwrap();
			inner.on_shutdown_complete.deregister(token);
		}
	}
}

impl<T: Clone> Future for ShutdownComplete<T> {
	type Output = T;

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		let me = self.get_mut();
		let mut inner = me.inner.lock().unwrap();

		// We're being polled, so we should deregister the waker (if any).
		if let Some(token) = me.waker_token.take() {
			inner.on_shutdown_complete.deregister(token);
		}

		// Check if the shutdown is completed.
		if inner.delay_tokens == 0 {
			if let Some(reason) = inner.shutdown_reason.clone() {
				return Poll::Ready(reason);
			}
		}

		// We're not ready, so register the waker to wake us on shutdown completion.
		me.waker_token = Some(inner.on_shutdown_complete.register(context.waker().clone()));

		Poll::Pending
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
			let mut wait_shutdown_complete = shutdown.wait_shutdown_complete();
			let task = tokio::spawn(async move {
				assert!(let Poll::Pending = poll_once(&mut wait_shutdown_complete).await);
			});
			assert!(let Ok(()) = task.await, "task = {i}");
		}

		// Since we wait for each task to complete before spawning another,
		// the total amount of waker slots used should be only 1.
		let inner = shutdown.inner.lock().unwrap();
		assert!(inner.on_shutdown_complete.total_slots() == 1);
		assert!(inner.on_shutdown_complete.empty_slots() == 1);
	}

	#[tokio::test]
	async fn cloning_does_not_clone_waker_token() {
		let shutdown = crate::ShutdownManager::<()>::new();

		let mut signal = shutdown.wait_shutdown_complete();
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
			let inner = shutdown.inner.lock().unwrap();
			assert!(inner.on_shutdown_complete.total_slots() == 2);
			assert!(inner.on_shutdown_complete.empty_slots() == 0);
		}

		{
			drop(signal);
			let inner = shutdown.inner.lock().unwrap();
			assert!(inner.on_shutdown_complete.empty_slots() == 1);
		}

		{
			drop(cloned);
			let inner = shutdown.inner.lock().unwrap();
			assert!(inner.on_shutdown_complete.empty_slots() == 2);
		}
	}
}

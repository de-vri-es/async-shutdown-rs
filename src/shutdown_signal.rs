use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::{WrapCancel, ShutdownManagerInner};

/// A future to wait for a shutdown signal.
///
/// The future completes when the associated [`ShutdownManager`][crate::ShutdownManager] triggers a shutdown.
///
/// The shutdown signal can be cloned and sent between threads freely.
#[derive(Clone)]
pub struct ShutdownSignal<T: Clone> {
	pub(crate) inner: Arc<Mutex<ShutdownManagerInner<T>>>,
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
		let me = self.as_ref();
		let mut inner = me.inner.lock().unwrap();
		if let Some(reason) = inner.shutdown_reason.clone() {
			Poll::Ready(reason)
		} else {
			inner.on_shutdown.push(context.waker().clone());
			Poll::Pending
		}
	}
}

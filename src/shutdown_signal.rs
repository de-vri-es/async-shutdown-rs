use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::{WrapCancel, ShutdownInner};

/// A future to wait for a shutdown signal.
///
/// The future completes when the associated [`Shutdown`][crate::Shutdown] triggers a shutdown.
///
/// The shutdown signal can be cloned and sent between threads freely.
#[derive(Clone)]
pub struct ShutdownSignal {
	pub(crate) inner: Arc<Mutex<ShutdownInner>>,
}

impl ShutdownSignal {
	/// Wrap a future so that it is cancelled when a shutdown is triggered.
	///
	/// The returned future completes with `None` when a shutdown is triggered,
	/// and with `Some(x)` when the wrapped future completes.
	///
	/// The wrapped future is dropped when the shutdown starts before the future completed.
	/// If the wrapped future completes before the shutdown signal arrives, it is not dropped.
	#[inline]
	pub fn wrap_cancel<F: Future>(&self, future: F) -> WrapCancel<F> {
		WrapCancel {
			shutdown_signal: self.clone(),
			future: Some(future),
		}
	}
}

impl Future for ShutdownSignal {
	type Output = ();

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		let me = self.as_ref();
		let mut inner = me.inner.lock().unwrap();
		if inner.shutdown {
			Poll::Ready(())
		} else {
			inner.on_shutdown.push(context.waker().clone());
			Poll::Pending
		}
	}
}

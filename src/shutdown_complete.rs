use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::ShutdownManagerInner;

/// Future to wait for a shutdown to complete.
pub struct ShutdownComplete<T: Clone> {
	pub(crate) inner: Arc<Mutex<ShutdownManagerInner<T>>>,
}

impl<T: Clone> Future for ShutdownComplete<T> {
	type Output = T;

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		let me = self.as_ref();
		let mut inner = me.inner.lock().unwrap();
		if inner.delay_tokens == 0 {
			if let Some(reason) = inner.shutdown_reason.clone() {
				Poll::Ready(reason)
			} else {
				inner.on_shutdown_complete.push(context.waker().clone());
				Poll::Pending
			}
		} else {
			inner.on_shutdown_complete.push(context.waker().clone());
			Poll::Pending
		}
	}
}

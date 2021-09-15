use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use crate::ShutdownInner;

/// Future to wait for a shutdown to complete.
pub struct ShutdownComplete {
	pub(crate) inner: Arc<Mutex<ShutdownInner>>,
}

impl Future for ShutdownComplete {
	type Output = ();

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		let me = self.as_ref();
		let mut inner = me.inner.lock().unwrap();
		if inner.delay_tokens == 0 {
			Poll::Ready(())
		} else {
			inner.on_shutdown_complete.push(context.waker().clone());
			Poll::Pending
		}
	}
}

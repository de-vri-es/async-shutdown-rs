use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::DelayShutdownToken;

/// Wrapped future that delays shutdown completion until it completes or until it is droppped.
#[must_use = "futures must be polled to make progress"]
pub struct WrapDelayShutdown<T: Clone, F> {
	pub(crate) delay_token: Option<DelayShutdownToken<T>>,
	pub(crate) future: F,
}

impl<T: Clone, F: Future> Future for WrapDelayShutdown<T, F> {
	type Output = F::Output;

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		// SAFETY: We never move `future`, so we can not violate the requirements of `F`.
		unsafe {
			let me = self.get_unchecked_mut();
			match Pin::new_unchecked(&mut me.future).poll(context) {
				Poll::Pending => Poll::Pending,
				Poll::Ready(value) => {
					me.delay_token = None;
					Poll::Ready(value)
				},
			}
		}
	}
}

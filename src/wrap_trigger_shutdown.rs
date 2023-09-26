use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::TriggerShutdownToken;

/// Wrapped future that triggers a shutdown when it completes or is dropped.
#[must_use = "futures must be polled to make progress"]
pub struct WrapTriggerShutdown<T: Clone, F> {
	pub(crate) trigger_shutdown_token: Option<TriggerShutdownToken<T>>,
	pub(crate) future: F,
}

impl<T: Clone, F: Future> Future for WrapTriggerShutdown<T, F> {
	type Output = F::Output;

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		// SAFETY: We never move `future`, so we can not violate the requirements of `F`.
		unsafe {
			let me = self.get_unchecked_mut();
			match Pin::new_unchecked(&mut me.future).poll(context) {
				Poll::Pending => Poll::Pending,
				Poll::Ready(value) => {
					me.trigger_shutdown_token = None;
					Poll::Ready(value)
				},
			}
		}
	}
}

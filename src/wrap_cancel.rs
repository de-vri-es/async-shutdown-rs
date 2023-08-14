use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::shutdown_signal::ShutdownSignal;

/// Wrapped future that is automatically cancelled when a shutdown is triggered.
///
/// The wrapped future is dropped when a shutdown is triggered before the future completes.
/// The wrapped future is *not* dropped if it completes before the shutdown signal is received.
#[must_use = "futures must be polled to make progress"]
pub struct WrapCancel<F> {
	pub(crate) shutdown_signal: ShutdownSignal,
	pub(crate) future: Option<F>,
}

impl<F: Future> Future for WrapCancel<F> {
	type Output = Option<F::Output>;

	#[inline]
	fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		// SAFETY: We never move `future`, so we can not violate the requirements of `F`.
		let me = unsafe { self.get_unchecked_mut() };

		match &mut me.future {
			None => return Poll::Ready(None),
			Some(future) => {
				// SAFETY: We never move `future`, so we can not violate the requirements of `F`.
				let future = unsafe { Pin::new_unchecked(future) };
				if let Poll::Ready(value) = future.poll(context) {
					return Poll::Ready(Some(value));
				}
			},
		};

		// Otherwise check if the shutdown signal has been given.
		Pin::new(&mut me.shutdown_signal)
			.poll(context)
			.map(|()| None)
	}
}

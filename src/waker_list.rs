use std::task::Waker;

/// A list of wakers.
#[derive(Debug, Default)]
pub struct WakerList {
	/// The wakers (with possibly empty slots)
	wakers: Vec<Option<Waker>>,

	/// The empty slots in the list.
	empty_slots: Vec<usize>,

	/// The current epoch, increased whenever `wake_all` is called.
	epoch: usize,
}

pub struct WakerToken {
	epoch: usize,
	index: usize,
}

impl WakerList {
	/// Create a new empty list of wakers.
	pub fn new() -> Self {
		Self::default()
	}

	/// Register a waker to be woken up when `wake_all` is called.
	///
	/// Returns a token that can be used to unregister the waker again.
	pub fn register(&mut self, waker: Waker) -> WakerToken {
		if let Some(index) = self.empty_slots.pop() {
			debug_assert!(self.wakers[index].is_none());
			self.wakers[index] = Some(waker);
			self.token(index)
		} else {
			self.wakers.push(Some(waker));
			self.token(self.wakers.len() - 1)
		}
	}

	/// Deregister a waker so it will not be woken up by `wake_all` any more.
	///
	/// This should be called when a future that registered the waker is dropped,
	/// to prevent the list of wakers growing infinitely large.
	///
	/// # Panic
	/// May panic now or later if you give this function a token from another [`WakerList`].
	pub fn deregister(&mut self, token: WakerToken) -> Option<Waker> {
		if self.epoch != token.epoch {
			None
		} else if let Some(waker) = self.wakers[token.index].take() {
			self.empty_slots.push(token.index);
			Some(waker)
		} else {
			None
		}
	}

	/// Wake all wakers, clear the list and increase the epoch.
	#[allow(clippy::manual_flatten)] // Ssssh.
	pub fn wake_all(&mut self) {
		for waker in &mut self.wakers {
			if let Some(waker) = waker.take() {
				waker.wake()
			}
		}
		self.wakers.clear();
		self.empty_slots.clear();
		self.epoch += 1;
	}

	/// Create a token for the current epoch with the given index.
	fn token(&self, index: usize) -> WakerToken {
		WakerToken {
			epoch: self.epoch,
			index,
		}
	}

	/// Get the total number of waker slots.
	///
	/// This includes empty slots.
	#[cfg(test)]
	pub fn total_slots(&self) -> usize {
		self.wakers.len()
	}

	/// Get the number of empty waker slots.
	#[cfg(test)]
	pub fn empty_slots(&self) -> usize {
		self.empty_slots.len()
	}
}

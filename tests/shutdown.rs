use assert2::{assert, let_assert};
use futures::future;
use std::future::Future;
use std::time::Duration;

use async_shutdown::ShutdownManager;
use tokio::task::JoinHandle;

#[cfg(feature = "multi-thread")]
fn spawn<F>(fut: F) -> JoinHandle<F::Output>
where
	F: Future + Send + 'static,
	F::Output: Send + 'static,
{
	tokio::spawn(fut)
}
#[cfg(not(feature = "multi-thread"))]
fn spawn<F>(fut: F) -> JoinHandle<F::Output>
where
	F: Future + 'static,
	F::Output: 'static,
{
	tokio::task::spawn_local(fut)
}

#[track_caller]
fn test_timeout(test: impl Future<Output = ()>) {
	#[cfg(feature = "multi-thread")]
	let_assert!(
		Ok(runtime) = tokio::runtime::Runtime::new(),
		"failed to initialize tokio runtime"
	);
	#[cfg(not(feature = "multi-thread"))]
	let_assert!(
		Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build(),
		"failed to initialize tokio runtime"
	);
	#[cfg(feature = "multi-thread")]
	runtime.block_on(async move {
		let test = tokio::time::timeout(Duration::from_millis(100), test);
		assert!(let Ok(()) = test.await, "test timed out");
	});
	#[cfg(not(feature = "multi-thread"))]
	{
		let local = tokio::task::LocalSet::new();

		local.block_on(&runtime, async move {
			let test = tokio::time::timeout(Duration::from_millis(100), test);
			assert!(let Ok(()) = test.await, "test timed out");
		})
	}
}

#[test]
fn shutdown() {
	// Create a `ShutdownManager` manager and instantly trigger a shutdown,
	test_timeout(async {
		let shutdown = ShutdownManager::new();
		assert!(let Ok(()) = shutdown.trigger_shutdown(1));
		assert!(shutdown.wait_shutdown_triggered().await == 1);
		assert!(shutdown.wait_shutdown_complete().await == 1);
	});

	// Trigger the shutdown from a task after a short sleep.
	test_timeout(async {
		let shutdown = ShutdownManager::new();

		spawn({
			let shutdown = shutdown.clone();
			async move {
				tokio::time::sleep(Duration::from_millis(20)).await;
				assert!(let Ok(()) = shutdown.trigger_shutdown(2));
			}
		});
		assert!(shutdown.wait_shutdown_triggered().await == 2);
		assert!(shutdown.wait_shutdown_complete().await == 2);
	});
}

#[test]
fn shutdown_only_works_once() {
	let shutdown = ShutdownManager::new();
	assert!(let Ok(()) = shutdown.trigger_shutdown("first"));
	assert!(let Err(async_shutdown::ShutdownAlreadyStarted {
		shutdown_reason: "first",
		ignored_reason: "second",
		..
	}) = shutdown.trigger_shutdown("second"));
}

#[test]
fn wrap_cancel() {
	// Spawn a never-completing future that is automatically dropped on shutdown.
	test_timeout(async {
		let shutdown = ShutdownManager::new();
		let task = spawn(shutdown.wrap_cancel(future::pending::<()>()));
		assert!(let Ok(()) = shutdown.trigger_shutdown("goodbye!"));
		let_assert!(Ok(Err(reason)) = task.await);
		assert!(reason == "goodbye!");
	});

	// Same test, but use a ShutdownSignal to wrap the future.
	test_timeout(async {
		let shutdown = ShutdownManager::new();
		let wait_shutdown_triggered = shutdown.wait_shutdown_triggered();
		let task = spawn(wait_shutdown_triggered.wrap_cancel(future::pending::<()>()));
		assert!(let Ok(()) = shutdown.trigger_shutdown("wooh"));
		let_assert!(Ok(Err(reason)) = task.await);
		assert!(reason == "wooh");
	});
}

#[test]
fn wrap_cancel_no_shutdown() {
	// Spawn an already ready future and verify that it can complete if no shutdown happens.
	test_timeout(async {
		let shutdown = ShutdownManager::<()>::new();
		let task = spawn(shutdown.wrap_cancel(future::ready(10)));
		assert!(let Ok(Ok(10)) = task.await);
	});

	// Same test, but use a future that first sleeps and then completes.
	test_timeout(async {
		let shutdown = ShutdownManager::<()>::new();
		let task = spawn(shutdown.wrap_cancel(async {
			tokio::time::sleep(Duration::from_millis(20)).await;
			10u32
		}));
		assert!(let Ok(Ok(10)) = task.await);
	});
}

#[test]
fn delay_token() {
	// Delay shutdown completion using a delay-shutdown token.
	test_timeout(async {
		let shutdown = ShutdownManager::new();

		// Create a token to delay shutdown completion.
		let_assert!(Ok(delay) = shutdown.delay_shutdown_token());

		assert!(let Ok(()) = shutdown.trigger_shutdown(10));
		shutdown.wait_shutdown_triggered().await;
		assert!(shutdown.is_shutdown_completed() == false);

		// Move the token into a task where it is dropped after a short sleep.
		spawn(async move {
			tokio::time::sleep(Duration::from_millis(20)).await;
			drop(delay);
		});

		shutdown.wait_shutdown_complete().await;
	});
}

#[test]
fn wrap_delay() {
	// Spawn a future that delays shutdown as long as it is running.
	test_timeout(async {
		let shutdown = ShutdownManager::new();
		let_assert!(Ok(delay) = shutdown.delay_shutdown_token());
		assert!(let Ok(()) = shutdown.trigger_shutdown(10));

		spawn(delay.wrap_future(async move {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}));

		shutdown.wait_shutdown_triggered().await;
		shutdown.wait_shutdown_complete().await;
	});
}

#[test]
fn wait_for_shutdown_complete_in_task_without_delay_tokens() {
	test_timeout(async {
		let shutdown = ShutdownManager::new();

		spawn({
			let shutdown = shutdown.clone();
			async move {
				tokio::time::sleep(Duration::from_millis(10)).await;
				assert!(let Ok(()) = shutdown.trigger_shutdown(10));
			}
		});

		let task = spawn({
			let shutdown = shutdown.clone();
			async move {
				assert!(shutdown.wait_shutdown_complete().await == 10);
			}
		});

		assert!(let Ok(()) = task.await);
	});
}

#[test]
fn delay_token_too_late() {
	// Try go get a delay-shutdown token after the shutdown completed.
	let shutdown = ShutdownManager::new();
	assert!(let Ok(()) = shutdown.trigger_shutdown(()));
	assert!(let Err(async_shutdown::ShutdownAlreadyCompleted { .. }) = shutdown.delay_shutdown_token());
	assert!(let Err(async_shutdown::ShutdownAlreadyCompleted { .. }) = shutdown.wrap_delay_shutdown(future::pending::<()>()));
}

#[test]
fn vital_token() {
	// Trigger a shutdown by dropping a token.
	test_timeout(async {
		let shutdown = ShutdownManager::new();

		let trigger_shutdown_token = shutdown.trigger_shutdown_token("stop!");
		drop(trigger_shutdown_token);

		assert!(shutdown.wait_shutdown_triggered().await == "stop!");
		assert!(shutdown.wait_shutdown_complete().await == "stop!");
	});

	// Same test, but drop the vital token in a task.
	test_timeout(async {
		let shutdown = ShutdownManager::new();

		let vital = shutdown.trigger_shutdown_token("done");
		spawn(async move {
			tokio::time::sleep(Duration::from_millis(20)).await;
			drop(vital);
		});

		assert!(shutdown.wait_shutdown_triggered().await == "done");
		assert!(shutdown.wait_shutdown_complete().await == "done");
	});
}

#[test]
fn wrap_vital() {
	// Trigger a shutdown by dropping a vital token from a task after a short sleep.
	test_timeout(async {
		let shutdown = ShutdownManager::new();

		spawn(shutdown.wrap_trigger_shutdown("sleep done", async move {
			tokio::time::sleep(Duration::from_millis(20)).await;
		}));

		assert!(shutdown.wait_shutdown_triggered().await == "sleep done");
		assert!(shutdown.wait_shutdown_complete().await == "sleep done");
	});

	// Same test, but now use a future that is instantly ready.
	test_timeout(async {
		let shutdown = ShutdownManager::new();

		// Trigger the shutdown by dropping a vital token from a task after a short sleep.
		spawn(shutdown.wrap_trigger_shutdown("stop", future::ready(())));

		assert!(shutdown.wait_shutdown_triggered().await == "stop");
		assert!(shutdown.wait_shutdown_complete().await == "stop");
	});
}

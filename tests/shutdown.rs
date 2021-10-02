use assert2::{assert, let_assert};
use futures::future;
use std::future::Future;
use std::time::Duration;

use async_shutdown::Shutdown;

#[track_caller]
fn test_timeout(test: impl Future<Output = ()>) {
	let_assert!(Ok(runtime) = tokio::runtime::Runtime::new(), "failed to initialize tokio runtime");
	runtime.block_on(async move {
		let test = tokio::time::timeout(Duration::from_millis(100), test);
		assert!(let Ok(()) = test.await, "test timed out");
	});
}

#[test]
fn shutdown() {
	// Create a `Shutdown` manager and instantly trigger a shutdown,
	test_timeout(async {
		let shutdown = Shutdown::new();
		shutdown.shutdown();
		shutdown.wait_shutdown().await;
		shutdown.wait_shutdown_complete().await;
	});

	// Trigger the shutdown from a task after a short sleep.
	test_timeout(async {
		let shutdown = Shutdown::new();

		tokio::spawn({
			let shutdown = shutdown.clone();
			async move {
				tokio::time::sleep(Duration::from_millis(20)).await;
				shutdown.shutdown();
			}
		});

		shutdown.wait_shutdown().await;
		shutdown.wait_shutdown_complete().await;
		assert!(true);
	});
}

#[test]
fn wrap_cancel() {
	// Spawn a never-completing future that is automatically dropped on shutdown.
	test_timeout(async {
		let shutdown = Shutdown::new();
		let task = tokio::spawn(shutdown.wrap_cancel(future::pending::<()>()));
		shutdown.shutdown();
		assert!(let Ok(None) = task.await);
	});

	// Same test, but use a ShutdownSignal to wrap the future.
	test_timeout(async {
		let shutdown = Shutdown::new();
		let wait_shutdown = shutdown.wait_shutdown();
		let task = tokio::spawn(wait_shutdown.wrap_cancel(future::pending::<()>()));
		shutdown.shutdown();
		assert!(let Ok(None) = task.await);
	});
}

#[test]
fn wrap_cancel_no_shutdown() {
	// Spawn an already ready future and verify that it can complete if no shutdown happens.
	test_timeout(async {
		let shutdown = Shutdown::new();
		let task = tokio::spawn(shutdown.wrap_cancel(future::ready(10)));
		assert!(let Ok(Some(10)) = task.await);
	});

	// Same test, but use a future that first sleeps and then completes.
	test_timeout(async {
		let shutdown = Shutdown::new();
		let task = tokio::spawn(shutdown.wrap_cancel(async {
			tokio::time::sleep(Duration::from_millis(20)).await;
			10u32
		}));
		assert!(let Ok(Some(10)) = task.await);
	});
}

#[test]
fn delay_token() {
	// Delay shutdown completion using a delay-shutdown token.
	test_timeout(async {
		let shutdown = Shutdown::new();

		// Create a token to delay shutdown completion.
		let_assert!(Ok(delay) = shutdown.delay_shutdown_token());

		shutdown.shutdown();
		shutdown.wait_shutdown().await;

		// Move the token into a task where it is dropped after a short sleep.
		tokio::spawn(async move {
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
		let shutdown = Shutdown::new();
		let_assert!(Ok(delay) = shutdown.delay_shutdown_token());
		shutdown.shutdown();

		tokio::spawn(delay.wrap_wait(async move {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}));

		shutdown.wait_shutdown().await;
		shutdown.wait_shutdown_complete().await;
	});
}

#[test]
fn delay_token_too_late() {
	// Try go get a delay-shutdown token after the shutdown completed.
	let shutdown = Shutdown::new();
	shutdown.shutdown();
	assert!(let Err(async_shutdown::ShutdownAlreadyCompleted { .. }) = shutdown.delay_shutdown_token());
	assert!(let Err(async_shutdown::ShutdownAlreadyCompleted { .. }) = shutdown.wrap_wait(future::pending::<()>()));
}

#[test]
fn vital_token() {
	// Trigger a shutdown by dropping a vital token.
	test_timeout(async {
		let shutdown = Shutdown::new();

		let vital = shutdown.vital_token();
		drop(vital);

		shutdown.wait_shutdown().await;
		shutdown.wait_shutdown_complete().await;
	});

	// Same test, but drop the vital token in a task.
	test_timeout(async {
		let shutdown = Shutdown::new();

		let vital = shutdown.vital_token();
		tokio::spawn(async move {
			tokio::time::sleep(Duration::from_millis(20)).await;
			drop(vital);
		});

		shutdown.wait_shutdown().await;
		shutdown.wait_shutdown_complete().await;
		assert!(true);
	});
}

#[test]
fn wrap_vital() {
	// Trigger a shutdown by dropping a vital token from a task after a short sleep.
	test_timeout(async {
		let shutdown = Shutdown::new();

		tokio::spawn(shutdown.wrap_vital(async move {
			tokio::time::sleep(Duration::from_millis(20)).await;
		}));

		shutdown.wait_shutdown().await;
		shutdown.wait_shutdown_complete().await;
		assert!(true);
	});

	// Same test, but now use a future that is instantly ready.
	test_timeout(async {
		let shutdown = Shutdown::new();

		// Trigger the shutdown by dropping a vital token from a task after a short sleep.
		tokio::spawn(shutdown.wrap_vital(future::ready(())));

		shutdown.wait_shutdown().await;
		shutdown.wait_shutdown_complete().await;
		assert!(true);
	});
}

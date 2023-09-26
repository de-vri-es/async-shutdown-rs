# Unreleased:
* Rename `Shutdown` struct to `ShutdownManager`.
* Rename `ShutdownManager` methods:
  * `shutdown()` is now called `trigger_shutdown()`
  * `shutdown_started()` is now called `is_shutdown_triggered()`.
  * `shutdown_completed()` is now called `is_shutdown_completed()`.
  * `wrap_vital()` is now called `wrap_trigger_shutdown()`.
  * `wrap_wait()` is now called `wrap_delay_shutdown()`.
* Rename `VitalToken` to `TriggerShutdownToken`.
* Rename `TriggerShutdownToken::wrap_vital()` to `wrap_future()`.
* Rename `DelayShutdownToken::wrap_wait()` to `wrap_future()`.
* All types now take a generic parameter `T: Clone` for the shutdown reason.
* Add a parameter for the shutdown reason in `ShutdownManager` methods `trigger_shutdown()`, `wrap_trigger_shutdown()` and `trigger_shutdown_token()`.
* Add `ShutdownManager::shutdown_reason()` to retrieve the shutdown reason.
* Return the shutdown reason from `ShutdownManager::wait_shutdown_triggered()` and `ShutdownManager::wait_shutdown_complete()`.

# Version 0.1.3 - 2023-08-14
* Mark all future wrappers as `#[must_use]`.

# Version 0.1.2 - 2021-10-30
* Update README.

# Version 0.1.1 - 2021-10-30
* Improve TCP echo server example.

# Version 0.1.0 - 2021-10-28
* Add an example with a tokio-based TCP echo server.

# Version 0.1.0-beta2 - 2021-10-04
* Regenerate README.md from library documentation.

# Version 0.1.0-beta1 - 2021-10-04
* Change `DelayShutdownToken::wrap_wait()` to consume the token.
* Do not consume the `Shutdown` object in `Shutdown::wrap_vital()`.
* Rename `wait_shutdown()` to `wait_shutdown_triggered()`.

# Version 0.1.0-alpha4 - 2021-09-29
* Fix `shutdown` and `shutdown_complete` notification.

# Version 0.1.0-alpha3 - 2021-09-22
* Add missing `Clone` impl for `Shutdown`.

# Version 0.1.0-alpha2 - 2021-09-22
* Fix crate name in README.

# Version 0.1.0-alpha1 - 2021-09-22
* Initial release.

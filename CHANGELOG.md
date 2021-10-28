# Version 0.1.0 - 2021-10-28
* Add an example with a tokio-based TCP echo server.

# Version 0.1.0-beta2 - 2021-10-04
* Regenerate README.md from library documentation.

# Version 0.1.0-beta1 - 2021-10-04
* Change `DelayShutdownToken::wrap_wait()` to constume the token.
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

# Contributing

The active Windows application is in `source/rust/`; see its README for prerequisites and test commands. Keep changes focused and describe the observed behavior, the fix and how it was checked.

Before submitting code, run Rust unit tests and Clippy with warnings denied. Run relevant Python regression checks for UI, protocol or packaging changes. A synthetic test result must not be presented as proof of real-device compatibility. Windows CI builds the source and runs noninteractive checks; interactive UI checks run locally.

UI text lives in `source/rust/locales/ui.tsv`. Preserve its tab-separated structure and provide all ten translations when adding a key. Preserve third-party notices when updating protocol code, fonts or codecs.

For bug reports, include app version, Windows build, iOS version, device model and reproduction steps. Remove device names, addresses, keys and other personal information from logs. Never upload `*identity.key`, settings, packet captures or an entire logs folder without reviewing it.

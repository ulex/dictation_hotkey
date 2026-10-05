# macOS validation

Validated locally on Apple Silicon on 2026-10-05 with Rust 1.99.0 and Apple Swift 6.1.2.

- `cargo fmt --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked`: 33 passed, 1 ignored (one-hour disk spool simulation). No microphone, clipboard, injected input or authenticated network tests run.
- `cargo check --locked --target x86_64-pc-windows-msvc`: passed on macOS.
- `cargo clippy --locked --all-targets --target x86_64-pc-windows-msvc -- -D warnings`: passed.
- `python3 tools/package_macos.py`: Apple Silicon release build, bundled native dylib, ad-hoc signing and ZIP/checksum generation passed.
- `codesign --verify --deep --strict`: passed for the Apple Silicon app bundle.
- Mach-O dependencies inspected: bundle-local adapter and system Apple/Swift frameworks; development library search path removed from the packaged executable.
- Both privacy/entitlement plist files pass `plutil -lint`.

The new tests cover shared realtime warmup order, flush/end, premature provider completion, cancellation, macOS shortcut aliases/limits, private config/backup permissions, single-instance lock release and crashed multipart-file cleanup.

The native GUI was not launched, and live microphone capture, audio conversion, permission prompts, output insertion, authenticated realtime/batch service, login items, fallback against real target applications and clean-machine behavior remain unverified. See the macOS checklist in BUILDING.md. Intel macOS has a CI job but was not built or run locally. Windows runtime behavior was not retested on a Windows desktop. Packages are ad-hoc signed, not notarized.

## origin/native merge validation (2026-10-05)

Merged remote commit `03a02f3`, retaining the macOS platform split and moving all remote Windows controller changes into `src/platform/windows/app.rs`. Both macOS build guidance and the remote Windows UI/icon guidance are preserved.

- `cargo fmt --check`: passed.
- `cargo test --locked`: 34 passed (33 library tests and 1 icon resource test), 1 ignored.
- `cargo clippy --locked --all-targets -- -D warnings`: passed on Apple Silicon.
- `cargo check --locked --target x86_64-pc-windows-msvc`: passed.
- `cargo clippy --locked --all-targets --target x86_64-pc-windows-msvc -- -D warnings`: passed, including Windows tests.
- `python3 tools/package_macos.py`: release build, bundle, ZIP/checksum and signing verification passed.
- Explicit `codesign --verify --deep --strict` and privacy/entitlement plist lint: passed.
- Packaged executable links to `@rpath/libDictationMac.dylib` and the system library.

Windows release linking was not performed: this host has no MSVC linker or cargo-xwin installation. Windows desktop tests, Intel macOS builds, and live microphone/input/login-item/authenticated service checks remain unverified. No interactive tests were run.

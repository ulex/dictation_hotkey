# Baseline/resource measurement status

Native Windows build/smoke checks and a 30-second idle sample are now recorded in [WINDOWS_VALIDATION.md](WINDOWS_VALIDATION.md), with raw data under [windows/](windows/). Successful microphone capture and authenticated transcription remain blocked by the validation session's missing capture endpoint/API key. No legacy Python performance baseline, active-session benchmark, or clean-machine run has been measured.

`sample-process.ps1` is a general Windows data collection helper; `../tools/measure-native-idle.ps1` launches/samples/closes the native release automatically. Retain raw CSV alongside OS/build, mic, SDK pin, network, hashes, and actions performed. Include the PyInstaller parent/child processes and unpacked extraction directory when comparing against the legacy Python build.

| Build | Platform | Executable | Zip | SHA-256 | Idle/active private bytes |
| --- | --- | ---: | ---: | --- | --- |
| Native release, MSVC static CRT (Rust 1.99.0, cargo-xwin) | Linux cross-build for Windows x64 | 472,064 bytes | 233,711 bytes | `6c8e8679383820d646be48852943a006998cac31f76bfd592d86283f4675f0b7` exe; `3b9c15f219e2e41170f6bc6d3efa248e05374ad47c5996f5bfb66430dd1b727c` zip | Not measured |
| Native release, MSVC static CRT (Rust 1.99.0, MSVC 14.44) | Windows 11 x64 build 26200 | 489,472 bytes | 240,406 bytes | `8b539fca613e828c3d9922c5dc05b165a8af312f2fba475f6dad07ef03f834a3` exe; ZIP hash in `native/dist/SHA256SUMS.txt` | Idle mean 2,085,274 bytes; active not measured |
| Existing Python/PyInstaller | Windows baseline pending | Not measured | Not measured | Pending | Not measured |

The table records the initial Windows validation build. A follow-up Win+H interception patch has since been built into both `native/dist/` and root `dist/` (490,496-byte EXE, SHA-256 `7a5fe24ae935016d7ef21bcb3faceccc129888a00712fb13cefeeb94a0ddb1b5`). The previous root executable is backed up as `dist/DictationHotkey-before-hotkey-fix.exe`. No idle measurement was repeated for the patched executable. After the desktop reconnected, its shell-level Win+H regression passed; the original executable reproduced the unwanted Start-menu activation. Actual microphone recording remains unvalidated. Idle values are short observations, not evidence of long-session leak behavior or performance improvement over Python. See the validation report for remaining acceptance checks.

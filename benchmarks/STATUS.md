# Baseline/resource measurement status

No Windows performance baseline has been measured here; Linux cannot evaluate working-set, first-text latency, microphone behavior, Windows hotkeys, RDP, tray activation, WinHTTP, or clean-machine dependencies. `sample-process.ps1` is a Windows data collection helper; run it with the native release and retain raw CSV alongside OS/build, mic, SDK pin, network, hashes, and actions performed. Include the PyInstaller parent/child processes and unpacked extraction directory when comparing against the legacy Python build.

| Build | Platform | Executable | Zip | SHA-256 | Idle/active private bytes |
| --- | --- | ---: | ---: | --- | --- |
| Native release, MSVC static CRT (Rust 1.99.0, cargo-xwin) | Linux cross-build for Windows x64 | 472,064 bytes | 233,711 bytes | `6c8e8679383820d646be48852943a006998cac31f76bfd592d86283f4675f0b7` exe; `3b9c15f219e2e41170f6bc6d3efa248e05374ad47c5996f5bfb66430dd1b727c` zip | Not measured |
| Existing Python/PyInstaller | Windows baseline pending | Not measured | Not measured | Pending | Not measured |

The native release build is available locally under `dist/` after packaging. Windows smoke tests and memory/performance measurements are still required before making resource claims beyond artifact size.

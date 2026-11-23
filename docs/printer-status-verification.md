# Printer status Phase 3: software and hardware verification

This document distinguishes **actual test execution** from simulated observations. A successful release CI run is **not** evidence that a USB thermal printer will respond to DLE EOT, that paper actually exited, or that Windows reports a powered-off printer accurately.

## Automated test layers

| Layer | CI command/job | What it measures | What it cannot establish |
| --- | --- | --- | --- |
| Pure flag precedence | `cargo test --locked` (`printer_fault_tests`) | Full 8,192-combination status matrix, attribute combinations, no synthetic false-green USB | Actual manufacturer status bits |
| Device identity | Same Rust unit tests | Explicit one-to-one USBPRINT path matching, duplicate binding rejection, two identical model identifiers | Validity of an operator-entered physical mapping |
| Synthetic device faults | Same Rust unit tests | Powered-off ambiguity, offline, paper-out, cover/error, junk/unknown, network source metadata | What any physical USB printer reports |
| Deterministic fuzz | Same Rust unit tests | 20,000 synthesized status/attribute combinations and JSON roundtrips | Security or hardware fuzzing beyond defined input domain |
| Concurrency | Rust tests, `timed_out_blocking_win32_worker_keeps_exclusive_discovery_permit` | A timed-out synchronous worker retains its exclusive permit until it exits | Windows RPC that never returns: worker may remain blocked indefinitely |
| Windows API compilation | `windows` CI on windows-latest | Locked Windows 0.62.2 bindings, MSVC target, clippy, `cargo test`, release build | Printer hardware behavior |
| Local HTTP security | `scripts/smoke-local.py` (Linux and Windows) | Origin/Host/branch isolation, no unauthorized `/test-print`, cached schema | Browser UI in restora-web |
| Real spooler integration | `windows-spooler-integration` on windows-2022 | Disposable local print queues, real EnumPrinters, event or 30s refresh, queue pause/resume, spooler stop/restart, cache-only HTTP latency, no printed.log | USB hardware: the USB-looking test port is **a local port named USB986** |
| Backend compatibility | Separate draft restora-backend PR #97 | New metadata fields accepted and older string[] reports continue to work | Actual downstream web settings UI |

## Independent acceptance scenarios

| Scenario | Synthetic coverage | Physical coverage before release |
| --- | --- | --- |
| USB queue installed, no verified hardware response | Yes: Unknown | Required |
| Unplugged USB interface with **verified** queue binding | Yes: Offline | Required |
| USB connected but powered off while still PnP-present | Yes: Unknown (no false Ready) | Required |
| Paper out/cover open via Winspool status | Yes: PaperOut/Error | Required |
| Bidirectional DLE EOT n=1..4 | **Not implemented** | Required before enabling |
| Network printer unreachable | Spooler source-limited; never promised physical confirmation | Required |
| WPP on/off on Windows 11 24H2+ | Official WinRT API compile; supported OS-specific runtime test required | Required |
| Concurrent print job plus probe | No probe currently enabled, so no new collision | Required before enabling probe |

## Release gate

1. Linux and Windows CI jobs **all green on the latest commit**, including new `windows-spooler-integration`.
2. `printer-integration-report.json` (artifact `restora-windows-spooler-evidence`) records each real Windows queue test, not a pretend USB fault. Missing driver, permission, nonresponsive spooler, or missed status changes must fail.
3. No regression to pairing, 2-second remote polling, retry timing, `printed.log` dedupe, or local request security.
4. Hardware-specific claims remain **unverified** until an actual Windows host and identified USB/ESC-POS printer have been tested. Even then, ready is point-in-time, not a guarantee of future paper output.

Never claim 100% accuracy for hardware that doesn't expose trustworthy sensors. Do not merge or release this draft solely because the simulated tests pass.

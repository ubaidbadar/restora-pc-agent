# Printer status implementation plan (Phase 2; approval gate before Phase 3)

Date: 2026-11-22. Based on [printer-status-research.md](printer-status-research.md) and reviewed code in src/printers.rs, backend.rs, status.rs, local.rs, agent.rs, config.rs, Cargo.toml, Cargo.lock, and .github/workflows/ci.yml.

**STOP HERE.** This file describes the proposed Phase 3 implementation. No runtime code, dependency version, CI, REST route, printing behavior, or polling configuration is changed by this documentation branch.

## Non-negotiable invariants

1. No change to pairing, remote job polling cadence (2s), retry intervals/15-minute give-up, receipt rendering, printed.log format/copy dedupe, offline authorization, or concurrent printing behavior.
2. GET /status remains cache-only. Never await driver/SetupAPI/spooler calls in a request handler.
3. All Windows entry points are under cfg(windows); all pure classification/merge/serialization code and tests compile on Linux.
4. Backend PUT /printers still sends printers as **array of printer-name strings**; extra fields are additive only. Existing local JSON fields are not removed.
5. Never infer USB printer health merely from an installed queue. No evidence / ambiguous queue mapping / stale data => unknown. Physical paper output is never guaranteed by Winspool success.
6. DLE EOT probes default OFF and never race an active print job. A probing failure must never suppress or reorder real jobs.
7. Explicit test print is user initiated; the periodic status check itself never prints.

## Data model and layer-specific diagnostics

In new pure src/printer_state.rs:
- enum PrinterState: Ready, Offline, PaperOut, Error, NotFound, Unknown. Serialize as snake_case (not_found, paper_out).
- enum Source: Spooler, Pnp, Printer. Use the source of the decisive observation; keep per-layer observations internally and optionally in an additive diagnostics object.
- WinPrinter { name: String, port: String, status: u32, attributes: u32 } plus internal optional driver_name, c_jobs and checked timestamp if useful.
- separate SpoolerObservation { state, raw flags, checked_at, error }, PnpObservation { matched identity, present, checked_at, error }, PrinterObservation { protocol result, checked_at, error }. Distinguish unsupported/unmapped/not-run/timeout from negative hardware evidence.
- aggregate CachedPrinter { name, port, state, source, checked_at, per_layer_results, last_print_result }; cache has last successful scan time and revision.
- Pure precedence: missing configured queue -> not_found; authoritative explicit spooler paper/door/error or explicit probe fault -> corresponding failure; verified mapped USB device absent -> offline; verified USB interface present with no blocking spooler flags -> Unknown/unverified with source pnp (PnP presence proves a connected interface, NOT physical readiness); only a fresh, validated device-protocol response can support Ready from the printer layer (still point-in-time only); USB spooler-only ready -> unknown/unverified; known non-USB spooler-only ready -> ready with source spooler as requested, but label it unconfirmed physical readiness; ambiguous transport or stale observation -> unknown. Confirm precise precedence for simultaneous conflicting flags with tests. No generic heuristic may mark a printer non-USB merely because port name does not start with USB.
- A checked_at timestamp records the **actual completed observation**, not the timestamp of every /status GET. Expire old ready claims (proposal: 90s without a successful refresh) to unknown; never overwrite last known diagnostic with an untruthful successful check.

## Layer 1 — spooler discovery (src/printers.rs + pure src/printer_state.rs)

- Replace list implementation with list() -> Result<Vec<WinPrinter>, String>, with a compatible helper names(&[WinPrinter]) -> Vec<String> for every existing pairing/reporting caller. Keep non-Windows stubs compiling.
- Use EnumPrintersW(PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS, level 2), copy pPrinterName and pPortName before freeing the underlying allocation, maintain proper alignment. Grow/retry the second call once on ERROR_INSUFFICIENT_BUFFER; preserve errors rather than cache empty names.
- Test pure map_spooler_flags(status: u32, attributes: u32) with flags for offline including PRINTER_ATTRIBUTE_WORK_OFFLINE, paper_out, jam, problem, door_open, error, not_available, user_intervention, no_toner, paused/unknown; zero flags maps only to spooler-ready candidate, never automatically hardware-ready.
- Call sites: src/agent.rs pair() still supplies name strings, src/backend.rs PUT still supplies name strings, src/local.rs GET /printers still returns its previous response shape. Maintain 5-second async timeout and spawn_blocking; limit concurrently active discovery tasks so timeout does not accumulate stuck threads.
- Unit tests for growth retry decision, zero/bad status, combined flags, empty list, port splitting, and clean name projection.

## Layer 2 — USB PnP (new src/usb_pnp.rs, cfg Windows)

- Build GUID_DEVINTERFACE_USBPRINT exactly from its documented value; enumerate **present** interfaces via SetupDiGetClassDevsW(DIGCF_PRESENT | DIGCF_DEVICEINTERFACE), SetupDiEnumDeviceInterfaces, SetupDiGetDeviceInterfaceDetailW; always destroy the device info set. May use CM_Get_Device_Interface_List_SizeW / CM_Get_Device_Interface_ListW (PRESENT) for a fast validation pass; do not treat stale/all-device listings as present.
- Record interface path, stable available PnP device instance ID, optional hardware IDs / serial; never use VID/PID or a numeric USB001 suffix alone to auto-map.
- For a known, explicitly verified persisted queue-to-device binding, distinguish present / absent. On a new/unmapped queue report unknown, not ready and not confidently absent. A local binding experience must be implemented only after the association can be demonstrated safely on Windows; ambiguous multiple printers stay unknown.
- Separately classify non-usb.sys/serial/network/virtual printers instead of pretending they are USBPRINT. Provide a clear explanation of the limitation in status.
- Pure tests for one-to-one binding, missing binding, duplicate model/VID:PID, USB001 renumbering, previous mapping now absent, and stale scan.

## Layer 3 — notifications and refresh (new src/printer_events.rs, src/backend.rs, src/agent.rs)

- One refresh coordinator owns the printer cache, deduplicates concurrent requests, debounces burst events, and wakes from USB PnP arrival/removal, local spooler queue changes, post-failed-print, startup, and a **30-second fallback timer**. No event may run enumeration on its callback or main async thread.
- Register CM_Register_Notification with CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, class USBPRINT. Callback enqueues a small refresh signal; always unregister before freeing callback context.
- Open a local print-server handle; FindFirstPrinterChangeNotification with PRINTER_CHANGE_PRINTER | PRINTER_CHANGE_PORT | PRINTER_CHANGE_JOB. Wait with a finite timeout on a dedicated thread, FindNextPrinterChangeNotification to reset, and close notification then printer handle. Remote printer connections can miss the event; 30-second poll remains mandatory. Recover/reenroll if the spooler restarts or registration fails.
- Preserve existing 2-second job polling and retry scheduler **unchanged**; dispatch is not blocked by status refresh.
- Backend PUT /printers: keep { version, printers: ["Name"] }; add e.g. printerStatuses: [{ systemName, state, source, checkedAt }] and wpp as separate top-level fields **only after verifying backend acceptance**. Compare semantic name/status/flags, not checkedAt-only time changes; report immediately when changed, and heartbeat every 5 minutes. Do not lose a changed state if send fails; retry without altering job polling. On no pairing, keep local cache without authenticated network posting.
- Record print failure -> queue immediate refresh, including local POST /print and remote attempted jobs; do not change retry delays.
- Tests for event storm/coalescing, repeated no-change, heartbeat, changed-state retry, timeout, and pairing changes.

## Layer 4 — optional USB ESC/POS (new src/escpos_status.rs, src/agent.rs, src/backend.rs)

- Config flag: escpos_status_probe_enabled, default false (stored separately from credentials, no automatic probing after upgrade). If no reliable queue binding or bidirectional usbprint interface, don't probe.
- Serialize a probe with Agent's existing per-printer lock **and** the backend active_printers set; inspect queue cJobs and reject the probe if jobs exist. Recheck before touching the device. Third-party spooler operations remain a race: fail closed if exclusive access cannot be established. Avoid blocking independent printers.
- Use CreateFileW on the exact verified device interface path, then WriteFile/ReadFile on a blocking OS worker. Prime/discard first byte if required; one 10 04 n request and one validated read per status (n=1..4), no batch. Recognize unsupported/vendor variations as unknown; never treat arbitrary bytes as a valid ready result. No print text/cut from a probe.
- Enforce actual **OS-I/O cancellation** and a maximum single in-flight probe, not only a detached tokio timeout. Validate overlapped IO + CancelIoEx support for usbprint.sys on the test printer; if it cannot be bounded safely, ship probing disabled for that model/driver. Cancel and close handles on shutdown/re-pair/print start. Record per-layer error without claiming physical evidence.
- Unit tests with a fake transport for bytes, first junk response, malformed/short reply, timeouts, job-in-progress and lock/queue rejection.

## Layer 5 — WPP environment (new src/windows_environment.rs; src/status.rs)

- Prefer Microsoft's effective-mode Windows.Graphics.Printing.ProtectedPrint.WindowsProtectedPrintInfo.IsProtectedPrintEnabled API, available on Windows 11 24H2+. This binding is **not in windows 0.61.3**. Proposed implementation route: assess a tightly scoped upgrade to a windows crate version that exposes Graphics_Printing_ProtectedPrint (0.62.x feature exists), test rust-toolchain/lockfile compatibility and WinRT initialization, then use the documented property. If the bump is not acceptable or the runtime API is unavailable, expose wpp: "unknown" without guessing.
- Do NOT use undocumented keys. Microsoft's documented WindowsProtectedPrintGroupPolicyState key only indicates group policy, not every possible effective-state/UI setting; never interpret absence as false.
- Status schema: wpp: true | false | "unknown". WPP true -> Headline Tone::Bad, with a specific message that legacy RAW spooler/ESC-POS printing is not supported in protected print mode and how to disable the mode using Windows Settings. Do not alter Windows policy automatically.
- Tests for true, false, unknown and headline precedence; real WPP-on Windows 11 testing required.

## Layer 6 — cached status and durable last result (src/status.rs, src/agent.rs, src/config.rs)

- PrinterRow adds state, source, checkedAt, optional diagnosis / layerDetails; preserves name, systemName, prints, found, lastOk, unused.
- Snapshot adds wpp; headline depends on WPP first, then pairing/network, missing/configured, failures, unknown/unverified, healthy. A configured USB printer that is only present in the spooler is **never a green headline**; when uncertain use Warn/Neutral with a useful explanation (not Bad for lack of evidence).
- found remains an installed-queue indicator for backward compatibility; lastOk means result of last spooler **submission**, not guaranteed physical delivery. Make these semantics explicit. Don't turn an empty recent list into a green status.
- Add %APPDATA%/restora/printer-last-results.json (Linux config equivalent), with per-printer last submission success/failure and time; load at startup. Write atomically using a temporary file and replace, bounded history and no secret/slip/customer details. Keep printed.log unchanged; clear or scope entries after unpair/re-pair so another branch's results do not leak. Persist across normal restart; log file corruption safely without affecting actual printing.
- GET /status only clones cache/last results. Tests: restart recovery, stale/nonmatching queue, names unchanged, no last print, corrupted file, unknown vs ready, JSON backward-compatibility.

## Layer 7 — explicit test print (src/local.rs, src/printers.rs, src/agent.rs; separate web UI follow-up)

- Add POST /test-print { printer } on loopback, through the existing host+Origin middleware, requiring pairing, authorized/configured systemName, and a deliberately clicked user action. Reject invalid/missing/unsafe input, WPP true, concurrent real jobs, and prohibit arbitrary bytes from the request. Rate-limit clicks and add an accessible result/message; don't let a test print block the job poller.
- Exact fixed RAW bytes: ESC @ (1B 40), ASCII "Restora test print" + LF, modest feed, GS V 0 (1D 56 00) for a cut-capable ESC/POS printer. Provide clear notice that not every printer supports cut. Dispatch through existing printers::print_raw on a blocking worker and same per-printer mutual exclusion; avoid Agent::print's order-key dedupe so the test does not write printed.log. Capture its result in recent as kind=test and in durable per-printer last result with a clear "submitted to spooler" message.
- Repository has **no local setup page**. The button belongs in the separate Restora web Settings > Printers UI, which must be independently scoped and implemented; a backend-only API cannot make a button appear by itself.
- Tests: CORS/Origin/Host security, unpaired/unconfigured/restricted printer, duplicate rapid clicks, WPP on, fake print success/failure, no changes to printed.log, and recent entry.

## File-by-file proposed diff

| File | Proposed change |
| --- | --- |
| Cargo.toml, Cargo.lock | Add Windows-specific verified features; conditional narrowly scoped windows 0.62 bump for WPP |
| src/printers.rs | WinPrinter enumeration, size retry, safe aligned buffer, names adapter, print_raw unchanged |
| src/printer_state.rs (new) | Pure flag/state/source merge and freshness logic + Linux tests |
| src/usb_pnp.rs (new) | Windows USBPRINT SetupAPI/CM presence and binding logic |
| src/printer_events.rs (new) | CM callbacks and spooler change waiters; event queue |
| src/escpos_status.rs (new) | Optional bidirectional DLE EOT probing with cancellable IO |
| src/windows_environment.rs (new) | WPP effective-state detection or explicit unknown |
| src/agent.rs | Structured cache, printer locks coordination, persistent last result, keep existing journal |
| src/backend.rs | Refresh coordinator and change/heartbeat PUT; share job-active state, preserve polling/retries |
| src/status.rs | Additive cache-only status schema, headline rules, tests |
| src/local.rs | Preserve GET schemas; add hardened POST /test-print |
| src/config.rs | Optional separate probe settings and local results file location |
| src/main.rs or module root | Register new modules and watcher lifecycle, cfg gating |
| .github/workflows/ci.yml | Explicit Windows target cargo check, Linux pure tests, existing smoke tests retained |
| README.md | Document unknown vs ready, local test API, setup UI requirement, WPP and hardware caveats |
| Separate restora-web repository | Test button in Settings > Printers, only when authorized; **not** included automatically |

## CI / verification gates

Before delivery of Phase 3:
1. cargo fmt --all -- --check and cargo clippy --all-targets --locked -- -D warnings (where current project policy permits).
2. cargo test --locked and cargo build --locked on Linux, exercising all pure functions and mock transports.
3. On windows-latest explicitly run cargo check --locked --target x86_64-pc-windows-msvc, cargo test --locked and existing scripts/verify-local.ps1 -Release. Ensure .github/workflows/ci.yml exercises Windows release/smokes (already present), not just a cross-target declaration on Linux. Confirm windows feature names with a real compiler.
4. HTTP contract/regression tests for GET /status, GET /printers, POST /print, new POST /test-print and unchanged PUT name-array serialization; run existing scripts/smoke-local.py on Linux and Windows.
5. Deliberate event, cancellation, replug, crash/restart and slow-driver tests with fakes; assert no stuck worker proliferation or async-runtime blocking.
6. No production deployment or merging before human acceptance of Windows behavior.

**No tests, target check, hardware checks, or implementation have been performed in Phases 1–2**; this is an executable work plan, not a verification claim.

## Required Windows hardware acceptance (cannot be proven here)

| Setup / condition | Required observation | Risk |
| --- | --- | --- |
| USB thermal printer powered off | Explicit spooler fault or previously verified PnP absence => offline; otherwise unknown, never false ready | USB logic may remain enumerated while printer power is off |
| Same USB printer unplugged | PnP absent for verified binding => offline promptly; unmapped => unknown | USB00x port persists; notifications may be missed |
| Paper removed | If DLE EOT or driver reports it, paper_out; otherwise unknown/limited | Many devices/drivers don't expose sensors |
| Cover open | Device-specific DLE EOT / driver error only; otherwise unknown | Cover sensor semantics vary |
| Network printer | At minimum spooler flag and status with source spooler; no claim of live reachability | Spooler status can be stale, remote connections differ |
| Windows Protected Print Mode enabled | Effective API returns true, headline Bad; verify real RAW behavior | API exists only on supported Windows builds and suitable binding |
| Two identical USB printers | Never cross-associate queues or status; unknown if ambiguous | VID/PID/model may be the same |
| Real job while DLE EOT enabled | No probe interleaving, no duplicate or lost slips | External applications and driver races |

## Stop point / decision needed before Phase 3

Review and approve this plan before making code changes. Two scope decisions are especially important:

1. **WPP binding:** allow a carefully checked Windows crate upgrade to obtain the official effective-state WinRT API, or retain 0.61 and return unknown where there is no reliable query. Recommendation: prefer official API with narrowly tested upgrade.
2. **USB mapping:** because automatic USB001-to-USBPRINT mapping is unverified, approve a fail-closed unknown state and later explicit binding flow rather than presenting false green status. Recommendation: fail closed. Device-specific automatic mapping only after proven on real hardware.

Until these constraints are resolved, a software-only implementation cannot guarantee a green READY state for arbitrary USB printers.

## Deep-verification safety addendum (2026-11-22)

**Correction to the earlier plan:** a present USBPRINT interface is not proof that a USB printer is powered on, has paper, has its cover closed or can accept a job. Even with a verified queue-to-interface binding, PnP-present-only results must remain **Unknown/unverified**, not Ready. A validated fresh ESC/POS response from the specific mapped printer may support an affirmative USB Ready at that instant; a disconnected, unmapped, unsupported or timed-out probe may not. An explicitly confirmed PnP absence for a bound device supports Offline. Avoid claiming 100% physical readiness for the next print.

Epson's DLE EOT reference details n=1 online/offline, n=2 cover/stop, n=3 error, and n=4 roll-paper status. It specifically notes model differences and missing sensors: https://download4.epson.biz/sec_pubs/pos/reference_en/escpos/dle_eot.html . Never apply bit masks to unverified ESC/POS clones simply because they respond.

Microsoft says EnumPrinters level 2 can block opening stale remote print connections (OpenPrinter/RPC). A 5-second Tokio timeout **does not cancel** the synchronous Win32 worker. Use bounded concurrency, isolate problematic remote connections if possible, retain the last known observation while expiring stale readiness to Unknown, and never accumulate unlimited blocked tasks: https://learn.microsoft.com/en-us/windows/win32/printdocs/enumprinters .

Microsoft documents IOCTL_USBPRINT_GET_LPT_STATUS as an optional USBPRINT status-byte retrieval. It is NOT an automatic complete paper/cover detector or universal readiness proof; device-specific validation required: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usbprint/ni-usbprint-ioctl_usbprint_get_lpt_status .

### Additional acceptance gates before release

1. **Exact binding and build evidence:** compile real proposed imports/features against locked windows 0.61.3 (or document the exact explicitly approved version upgrade), Windows MSVC target; Linux unit and integration tests. Document checks that were actually run, not only planned.
2. **Pure model, fuzz and concurrency:** exhaustive status flag combinations; fail-closed on ambiguous USB mapping; stale timestamps; unknown versus offline; changed/heartbeat PUT payloads; property tests for malformed ESC/POS responses; printer result JSON corruption and restart; no printed.log modification.
3. **Windows simulation:** empty queues; local and remote printers; unreachable remote print server; spooler service stopped/restarted; notification bursts and loss; slow blocking drivers; cancellation and teardown; pairing race; one active job and another printer remaining responsive; strict /status cached-only latency.
4. **Actual USB hardware:** powered off/on, unplug/replug, paper-out, cover-open, first-byte junk, no readback, two identical devices, USB001 reassignment, active spooler jobs, driver errors. Record the expected **Offline / PaperOut / Error / Unknown** per tested model and measure event-to-status latency. Never accept a false green from only an installed queue or PnP-present interface.
5. **Network hardware:** cable unplug, print server unreachable, interrupted Wi-Fi, slow RPC, connection queues; label spooler-only Ready as source-limited rather than claiming a physical guarantee.
6. **Protected Print Mode:** Windows 11 24H2+ effective API true/false, unsupported OS unknown; test RAW ESC/POS behavior in actual mode. Do not infer effective WPP from absent group-policy settings.
7. **Production regression:** unchanged remote polling (2s), retry schedule/15-minute give-up, pairing, printed.log byte-for-byte semantics and duplicate suppression, local Origin/branch protections, print ordering, eventual report after offline reconnect.

**Honest release claim:** we can require 100% pass for the defined checks, while never claiming universal 100% hardware-status accuracy. User's six real-hardware scenarios must be explicitly marked unverified until run on the actual devices. Phase 3 is still blocked.

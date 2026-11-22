# Printer status research (Phase 1; no production-code changes)

Date: 2026-11-22. Scope: restora-pc-agent, main at version 0.1.21; Cargo.toml requests windows 0.61, Cargo.lock resolves windows **0.61.3**. Findings were checked against the repository, Microsoft documentation, the windows-rs 0.61.0 generated source (the 0.61.3 per-symbol docs are a stub), and the current escpos USBPRINT example. Bindings still need a compiler confirmation against the locked 0.61.3 package before implementation. No Windows device was examined.

## Repository observations

- src/printers.rs enumerates PRINTER_INFO_4W using PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS. It exposes only Vec<String>, with no physical availability evidence.
- src/backend.rs refreshes discovery and sends PUT /print-agent/printers every five minutes; its active_printers set serializes job attempts in the backend dispatch loop, but is not currently shared with local printing.
- src/agent.rs caches Option<Vec<String>>, and recent print outcomes are in-memory only. Agent::print already has a separate per-printer mutex and durable printed.log dedupe. Its successful result means **Windows accepted a RAW job**, not necessarily that paper emerged. Do not rewrite the journal.
- src/status.rs computes found from name membership and last_ok from recent; it can show Tone::Good solely from queue presence. GET /status is cache-only.
- src/local.rs has GET /printers with its own bounded discovery, POST /print with Origin/branch/printer checks, and no POST /test-print. README states no local dashboard/tray UI; printer setup is in Restora web Settings > Printers.
- .github/workflows/ci.yml already runs Linux cargo test/build, and Windows release verification/smoke tests, but lacks an explicit named cargo check --target x86_64-pc-windows-msvc gate.

## A. Winspool queue metadata and exact Rust types

Microsoft PRINTER_INFO_2 has pPrinterName, pPortName, pDriverName, Status, Attributes, cJobs and other fields. pPortName can be **comma-separated multiple ports**, not necessarily one. Unlike level 4, level 2 supplies the status/attribute DWORDs. Microsoft: https://learn.microsoft.com/en-us/windows/win32/printdocs/printer-info-2

Inspection of microsoft/windows-rs tag 0.61.0, crates/libs/windows/src/Windows/Win32/Graphics/Printing/mod.rs, confirms generated PRINTER_INFO_2W with pPrinterName, pPortName and pDriverName of windows_core::PWSTR; Status: u32; Attributes: u32; cJobs: u32; pSecurityDescriptor refers to Win32::Security. Thus Status/Attributes here are **u32, not newtypes**. The same source declares EnumPrintersW(flags: u32, name: Param<PCWSTR>, level: u32, Option<&mut [u8]>, *mut u32, *mut u32) -> Result<()>. Other API flag families below really are newtypes. Recheck against locked 0.61.3 with cargo check.

Relevant flags: PRINTER_STATUS_OFFLINE, PAPER_OUT, PAPER_JAM, PAPER_PROBLEM, DOOR_OPEN, ERROR, NOT_AVAILABLE, USER_INTERVENTION, NO_TONER, and PRINTER_STATUS_SERVER_UNKNOWN. Crucially **WORK_OFFLINE is PRINTER_ATTRIBUTE_WORK_OFFLINE (0x400), not PRINTER_STATUS_WORK_OFFLINE**. Include PAUSED, PENDING_DELETION, SERVER_OFFLINE and other serious errors in the mapping policy as appropriate. Prioritize an actionable paper condition over generic ERROR, but a spooler flag does not prove the printer supports or reports that sensor. Microsoft status values: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rprn/1625e9d9-29e4-48f4-b83d-3bd0fdaea787

Implementation safety: two-call EnumPrintersW level 2; allocation is sized in bytes. A second call can fail with ERROR_INSUFFICIENT_BUFFER if the queue list changes; grow and retry the data call once. Do not turn a failed enumeration into an empty/successful cache. Align the PRINTER_INFO_2W records safely (a Vec<u8> alone does not promise alignment for that struct); use an aligned buffer or safe copy, never arbitrary unaligned typed slicing. Pointer-backed strings must be copied before the allocation is freed.

## B. SetupAPI / CfgMgr32 presence and bindings

Interface class GUID under investigation: {28d78fad-5a12-11d1-ae5b-0000f803a8c2}, matching the usbprint.sys example. It discovers usbprint-class exposed interfaces, not **all** USB-connected thermal printers (some use vendor, serial, WinUSB or network drivers).

Microsoft specifies SetupDiGetClassDevsW with DIGCF_PRESENT | DIGCF_DEVICEINTERFACE to discover present, enabled device interfaces. Obtain actual paths with SetupDiEnumDeviceInterfaces / SetupDiGetDeviceInterfaceDetailW, then SetupDiDestroyDeviceInfoList. Microsoft: https://learn.microsoft.com/en-us/windows/win32/api/setupapi/nf-setupapi-setupdigetclassdevsw and https://learn.microsoft.com/en-us/windows/win32/api/setupapi/nf-setupapi-setupdigetdeviceinterfacedetailw

Generated windows-rs 0.61.0 inspection:
- SetupDiGetClassDevsW(classguid: Option<*const GUID>, enumerator: Param<PCWSTR>, hwndparent: Option<HWND>, flags: SETUP_DI_GET_CLASS_DEVS_FLAGS) -> Result<HDEVINFO>.
- DIGCF_PRESENT and DIGCF_DEVICEINTERFACE are SETUP_DI_GET_CLASS_DEVS_FLAGS **newtype** values (support bitwise OR), not plain u32.
- CM_Register_Notification(pfilter: *const CM_NOTIFY_FILTER, pcontext: Option<*const c_void>, pcallback: PCM_NOTIFY_CALLBACK, pnotifycontext: *mut HCMNOTIFICATION) -> CONFIGRET.
- CM_NOTIFY_FILTER.FilterType is CM_NOTIFY_FILTER_TYPE (newtype), with CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE. CM_NOTIFY_FILTER.u.DeviceInterface.ClassGuid selects the class. Keep registration/callback context alive, and unregister on shutdown.
- CM_Get_Device_Interface_List_SizeW followed by CM_Get_Device_Interface_ListW(interfaceclassguid: *const GUID, pdeviceid: Param<PCWSTR>, buffer: &mut [u16], ulflags: CM_GET_DEVICE_INTERFACE_LIST_FLAGS) -> CONFIGRET. Use CM_GET_DEVICE_INTERFACE_LIST_PRESENT (newtype, value 0) and parse the returned MULTI_SZ; retry on size races.
- Treat CONFIGRET using its documented CR_SUCCESS/error handling, **not** as a Win32 BOOL or Result.

Microsoft: https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_register_notification and https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_get_device_interface_listw

Required windows Cargo features for the initial Windows code: existing Win32_Foundation and Win32_Graphics_Printing and Win32_Graphics_Gdi; **add Win32_Devices_DeviceAndDriverInstallation**, and **Win32_Security** if needed for PRINTER_INFO_2W dependency. For CreateFileW/ReadFile/WriteFile add **Win32_Storage_FileSystem**; for cancellation/overlapped IO add **Win32_System_IO** if the exact chosen APIs need it. These names exist in windows 0.61.3 feature list: https://docs.rs/crate/windows/0.61.3/features . Verify actual imports and dependency gates through the Windows target build.

## C. Queue -> USB interface mapping: NO VERIFIED GENERIC LINK

PRINTER_INFO_2W gives a logical spooler queue and a **port string** such as USB001. SetupAPI gives physical device-interface paths and device-instance identity. Microsoft does **not** document a universal lossless join from a queue name/USB00x port/driver name or VID:PID to a specific present USBPRINT interface. USB001 is a spooler port, **not** the CreateFile path or a stable USB device identity. VID:PID alone is non-unique for two identical printers; friendly names are mutable; composite devices and redirected printer connections complicate matching. Registry Device Parameters/Port Description heuristics are not a documented cross-version contract. See: https://learn.microsoft.com/en-us/windows/win32/printdocs/printer-info-2 and https://learn.microsoft.com/en-us/windows-hardware/drivers/install/identifiers-for-usb-devices

**Decision:** never infer presence of a particular print queue from the fact that some USBPRINT device is present. If there is no verified one-to-one association (e.g. explicit local binding to a stable PnP device instance plus revalidation), state must be **unknown/unverified**, not ready and not confidently absent. If a previously verified binding becomes absent, report offline. Do not automatically pair two same-model printers using VID/PID or USB001 ordering. Hardware acceptance is needed to determine if a reliable system-derived join can be implemented for any driver family.

## D. Spooler change notifications

OpenPrinterW with a NULL printer name obtains the **local print server** handle. FindFirstPrinterChangeNotification(handle, PRINTER_CHANGE_PRINTER | PRINTER_CHANGE_PORT | PRINTER_CHANGE_JOB, 0, None) returns a waitable change handle, or invalid on failure. Use a dedicated blocking thread; WaitForSingleObject with a finite timeout or WaitForMultipleObjects including a shutdown event; when signaled call FindNextPrinterChangeNotification to read/reset, then debounce and enqueue refresh. Call FindClosePrinterChangeNotification **before** ClosePrinter; never share a handle across threads without synchronization. Watch PRINTER_CHANGE_SET_PRINTER plus adds/deletes, job changes and port changes. PRINTER_CHANGE_SERVER is a distinct flag and not included in PRINTER_CHANGE_ALL. If monitoring status fields needs finer notifications, PRINTER_NOTIFY_OPTIONS can be used after Windows testing.

**Scope:** a NULL-name handle watches the local print server's queues and changes; it does **not** certify that per-user remote printer connections are monitored. Per-connection behavior is not assumed; the fallback poll catches misses. Notifications may themselves be delayed, and they are not hardware health evidence.

Microsoft: https://learn.microsoft.com/en-us/windows/win32/printdocs/findfirstprinterchangenotification , https://learn.microsoft.com/en-us/windows/win32/printdocs/findnextprinterchangenotification , https://learn.microsoft.com/en-us/windows/win32/printdocs/openprinter

## E. ESC/POS readback via usbprint.sys (opt-in)

For *compatible bidirectional* usbprint.sys interfaces, the current Rust escpos project shows a WindowsUsbPrintDriver implementation using SetupAPI -> CreateFileW on the **interface path** -> WriteFile/ReadFile and a working examples/usbprint.rs. Its example explicitly documents that the **first bulk-IN byte can be junk** (often 0x2B), so it primes with one throwaway DLE EOT request/read, then makes **one DLE EOT request per write/read**; batching requests can misalign bulk-IN. Invalid bytes or zero-length/no response are treated as unsupported/unverified, not success. That first-byte behavior is an observed quirk, **not a universal Microsoft guarantee**.

Sources: https://github.com/fabienbellanger/escpos-rs/blob/master/examples/usbprint.rs and https://github.com/fabienbellanger/escpos-rs/blob/master/src/io/driver.rs ; usbprint driver background: https://learn.microsoft.com/en-us/windows-hardware/drivers/print/programming-considerations-for-usbprint

ESC/POS DLE EOT n=1..4 asks for real-time printer, offline, error, and paper status. Do not assume all vendors implement the same status semantics. Map paper/cover/error using validated bit masks from the selected printer's documentation (not guessed by generic code). No probe may run while the agent is actively submitting that printer's job, **or while a Windows spooler job is queued/active**; cJobs and active_printers are safety inputs but cannot rule out every race with third-party spooler clients. Serialize against the existing Agent::printer_locks as well as backend's active_printers and pre/post-check queue state. Start with this feature OFF. Do not print a test line during a probe.

**Timeout caveat:** tokio::time::timeout around spawn_blocking does NOT cancel a blocked synchronous Win32 ReadFile. Use an IO mechanism proven cancellable with a real deadline (overlapped I/O plus CancelIoEx where supported, or an equivalent safely owned handle cancellation) and a single bounded worker; otherwise disable readback for that driver. A timeout must become unknown, not a fake paper-out/offline. Do not start infinite stuck worker threads.

## F. Windows Protected Print Mode (WPP)

Microsoft documents the **effective-state API** Windows.Graphics.Printing.ProtectedPrint.WindowsProtectedPrintInfo.IsProtectedPrintEnabled, Windows 11 **24H2 / build 26100+**; query that instead of guessing from policy. Source: https://learn.microsoft.com/en-us/uwp/api/windows.graphics.printing.protectedprint.windowsprotectedprintinfo and https://learn.microsoft.com/en-us/uwp/api/windows.graphics.printing.protectedprint.windowsprotectedprintinfo.isprotectedprintenabled

Microsoft ALSO documents the policy registry location Software\\Policies\\Microsoft\\Windows NT\\Printers\\WPP, value WindowsProtectedPrintGroupPolicyState. This is a **group-policy setting**, not authoritative evidence of the effective mode set through Windows UI; absence means neither true nor false. Source: https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-printers#configurewindowsprotectedprint

**Binding gap:** windows **0.61.3** feature list has no Graphics_Printing_ProtectedPrint feature; the generated windows-rs 0.61.0 sources similarly do not include this WinRT namespace. A later windows 0.62 release exposes that feature, but any upgrade must be explicitly compiler-validated with Cargo.lock and the repo's toolchain. Until an officially supported effective-state call is available, report wpp: "unknown"; NEVER use a made-up registry key or turn policy absence into wpp: false. If effectively on, RAW legacy printing is incompatible with WPP's modern protected driver restrictions; signal Bad with a clear message and verify exact device behavior on Windows 11 hardware. Microsoft overview: https://learn.microsoft.com/en-us/windows/modern-print/windows-protected-print-mode/windows-protected-print-mode

## Open questions / unverified without hardware or build

1. Whether any installed USB001 queue offers a documented reliable association to the exact USBPRINT PnP instance; retain unknown if not.
2. Whether this USB thermal model exposes a usable bidirectional endpoint through usbprint.sys; whether DLE EOT / first-byte priming / per-request timing work.
3. How fast Windows changes PRINTER_INFO_2W status and which sensor flags the model/driver actually reports.
4. Whether Windows notification callbacks cover a particular connection and whether the spooler signals a power/unplug transition.
5. Whether cancellation of the selected USBPRINT ReadFile can meet a real timeout on target hardware.
6. Whether installed Windows build supports WPP effective-state API and whether upgrading the Rust binding is compatible.
7. Windows target compilation and real hardware behavior have **not** been run in Phase 1.

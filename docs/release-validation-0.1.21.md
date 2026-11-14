# 0.1.21 validation

## Confirmed code defects

- Get-StartApps accepted names matching `Restora (.+)`, including Restora Agent.
  Its AppID passed the old generic validator. With no protocol/PWA installed,
  a paired agent could launch itself through AppsFolder. Repeated launches hit
  the occupied port and enter the same launcher again.
- Synchronous PowerShell lookups had no deadline.
- EndPagePrinter and EndDocPrinter failures were ignored before journaling
  success. Failed/partial submissions now request AbortPrinter.
- HTTP 4xx/5xx job-result reports were treated as successful sends; malformed
  job arrays were silently dropped while status said online.
- Cargo reported 0.1.0 even though Store packaging was 0.1.20.0. The release
  publisher also pointed permanently at an older MSIX workflow run.

These are source-level findings. The affected customer's exact failure cannot
be confirmed without their logs and Windows reproduction.

## Automated gates

Run `scripts/verify-local.ps1 -Release` on Windows with Rust stable/MSVC.
It checks formatting, warning-free clippy and tests for portable and Store
configurations, then builds the portable release executable.
Linux CI also runs `python3 scripts/smoke-local.py target/debug/restora-pc-agent`
against a real subprocess with temporary paired/unpaired profiles, testing
status/version, credential secrecy, Host and Origin enforcement, CORS private
network preflight and cross-branch rejection without submitting a print. CI additionally
checks loopback readiness, originless write rejection, background startup and
embedded lifetime setup. The embedded registry assertion now reads the actual
RestoraPrintAgent key.

New tests exercise the actual PowerShell query with simulated Get-StartApps:
agent-only, agent plus a browser PWA, and ambiguous PWAs. A subprocess timeout
test exercises kill/reap behavior on both Linux and Windows. Pure ID tests
reject the Agent package and shell input. Existing receipt, width, retry,
branch and credential tests continue to run. MSIX CI builds Store code with
static CRT and validates packaging; this does not install the package.

## Required Windows acceptance before customer rollout

Record OS/browser versions, executable version/hash and the prior installed
version. Keep the previous customer installer available for rollback.

| Scenario | Expected evidence |
| --- | --- |
| Paired PC, Agent installed, no PWA/protocol | One agent remains; HTTPS orders page opens; no relaunch loop |
| Unpaired PC, no PWA | HTTPS printer settings open; explicit pairing succeeds |
| Installed Edge and Chrome PWA | Protocol consent/focus-existing behavior and correct route |
| Multiple PWAs, absent/stale protocol | Verify website fallback or repair browser protocol registration |
| Manual second launch while background instance runs | Existing printing service remains; web UI opens |
| Windows login and reboot, Store and portable | Listener becomes ready without opening a page; opt-out respected |
| Offline paired PC and restored network | Authorized local print works; remote queue resumes without duplicate accepted copies |
| USB/network printers, 58mm and 80mm | Kitchen/receipt layout and copies match settings |
| Paper-out, disconnected printer and spooler restart | Failures appear, retries preserve accepted copies; verify actual paper |
| Store 0.1.20.0 upgrade to 0.1.21.0 | Pairing/config retained; one listener; correct log/status version |
| Signed PWA provisioning | Requires real trusted PWA package; verify identity/hash, install, launch and uninstall separately |

A registered protocol or successful ShellExecute/Explorer dispatch cannot prove
a visible page opened. Stale browser registrations require Windows validation.
No physical printer or Windows desktop is available in the Linux workspace.
Do not infer physical printing or Store upgrade success from unit tests.

## Official resources reviewed

- [Rust Child process API](https://doc.rust-lang.org/std/process/struct.Child.html):
  try_wait is nonblocking; kill does not replace reaping with wait. The installed
  stable toolchain for local checks is Rust 1.100.0.
- [ShellExecuteW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecutew):
  success is a value greater than 32; dispatch does not establish browser UI state.
- [EndDocPrinter](https://learn.microsoft.com/en-us/windows/win32/printdocs/enddocprinter):
  a zero return is failure and spooler calls can block.
- [Edge WebAppInstallForceList](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-browser-policies/webappinstallforcelist):
  managed-policy route for background PWA provisioning. The agent must not
  overwrite enterprise browser policy to simulate a consumer install.
- [Desktop startup extensions](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions):
  packaged startup and portable registry startup have different activation rules.
- Live `https://app.restora.live/manifest.webmanifest` was retrieved during
  investigation: name Restora, standalone display, web+restora handler and
  focus-existing launch handler are present. This establishes manifest contents,
  not whether any particular Windows browser installed/registered it.

# Restora PC Agent

A headless Windows background service that prints [Restora](https://restora.live) kitchen tickets and receipts on the printers connected to a PC, with or without internet. Restora's installed PWA is the only staff-facing application. Open source under the [MIT licence](LICENSE).

**Install:** [Restora Agent on Microsoft Store](https://apps.microsoft.com/detail/9p6vnx5kvjl6?hl=en-US&gl=PK). This repository also builds a separate portable executable for testing and legacy installations.

## How it works
- **Open Restora:** launch Restora Agent from Windows. If the Restora PWA is installed in Edge or Chrome, the agent opens that PWA through the standard web+restora protocol and asks the browser/OS for permission on first use. If no PWA handler is registered, it opens the normal HTTPS app; the web app offers its supported Install action.
- **No agent window:** the agent runs silently in the background. It has no tray menu or local setup dashboard. Use Restora → Settings → Printers to connect, manage printers and see status. Windows startup runs the agent silently without opening a window.
- **Connect:** sign in to Restora, select the correct restaurant branch, and approve the browser's one-time permission to connect to apps on this computer. Restora passes a temporary backend-issued credential directly to the agent. Staff do **not** type a code in the normal setup.
- **Disconnect:** when the agent is removed in Restora, it detects the revoked credential, clears the local pairing, remembers that the disconnect was deliberate, and opens the PWA's Printers settings. It will not silently re-pair; a staff member must explicitly choose Connect this PC.
- **Sync:** the agent polls the authenticated backend for queued slips every two seconds and checks for changed printer configuration on the same connection. Windows printer discovery runs in a bounded background task every five minutes.
- **Print:** kitchen tickets and receipts are submitted through the Windows spooler. A journal remembers accepted copies so automatic retries do not repeat previous copies after partial failures. Jobs for separate physical printers process independently.
- **Offline:** on the same authorized branch and PC, Restora can submit slips through the protected http://127.0.0.1:17777/print endpoint when the internet is unavailable. Remote jobs remain queued on the backend until the agent reconnects.
- **Status:** the web app reads a fast cached status snapshot. Printer discovery is isolated from status requests, so a slow or unhealthy Windows printer driver cannot leave the app stuck on “Checking”. Windows spooler acceptance does not guarantee physical paper output.

## Security and compatibility
- The agent listens on **loopback only** (127.0.0.1:17777), requires the approved browser Origin for writes, and checks the paired restaurant branch and configured printer before offline printing. Chrome or Edge may ask for local network/loopback permission; this permission cannot be bypassed.
- The browser PWA registers the supported web+restora protocol in its manifest. The first protocol launch can show a browser permission prompt and, on Windows, an app-selection prompt if multiple browsers registered Restora. This is OS/browser consent, not an agent bypass.
- Agent bearer credentials are encrypted using Windows DPAPI in %APPDATA%\restora\agent.json; existing plaintext configs migrate when loaded.
- An old agent installation may not know its branch until reconnected. Avoid using the offline local-print path across branches until all PCs have upgraded and their branch identity is established.
- The printer-copy journal is %APPDATA%\restora\printed.log. A crash between spooler acceptance and journal write can leave an ambiguous print; **do not promise exactly-once physical printing**.
- Portable builds register a silent Windows startup entry unless the existing autostart.off opt-out marker is present. Store builds continue to use the package StartupTask. Users can disable startup in Windows Settings → Apps → Startup.
- Only release after testing real USB/network printers, 58/80mm, offline sync, paper-out, reboot, browser permission and Store MSIX upgrade. The Store app and portable build have different packaging and startup rules.

## Local service (127.0.0.1:17777)
GET / and GET /status return a JSON status snapshot; GET /printers lists Windows printers with a bounded timeout; GET /config returns this PC's configured printer settings; POST /print { branchId, printer, kind, orderKey, width?, copies?, order, header? } prints a validated slip. POST /setup/pair { code, api? } is called by the authorized Restora web app. There is no local HTML dashboard. Only trusted Restora origins and the loopback Host are allowed. RESTORA_API may override the default https://restora.live/api/v1 in development.

## Develop
```
cargo test                                     # slip tests
cargo run                                      # runs on Linux too; printing only works on Windows
cargo build --release --target x86_64-pc-windows-msvc
```
`test/index.html` calls the local server from a browser page. Public portable releases are manual through `.github/workflows/release.yml` until the Windows Agent passes its release verification. A maintainer triggers the workflow after validation; the next patch version is selected by default, or `#minor` / `#major` in the latest commit message changes versioning. With the R2 secrets set, the exe is also uploaded to the Cloudflare bucket (see the top of the workflow).

## Code signing policy
Release builds are made by GitHub Actions from this repository (`.github/workflows/release.yml`), so a signature confirms that the exe is an automated build of the source code here.

- **Status:** an application for free code signing from SignPath Foundation is pending. Until it is approved, releases are unsigned and Windows may show an "unknown publisher" warning. This section will be updated when signing starts.
- **Team roles:** author, reviewer and approver: [Ubaid Badar](https://github.com/ubaidbadar). Every release needs approval before it is signed.
- **Privacy:** this app does not collect or send personal data. It talks only to the Restora API (to fetch the slips for this PC and report them printed) and to the printers on this PC, plus the Restora web app on this PC over `127.0.0.1`. It keeps its settings and a log of printed slips in `%APPDATA%\restora` and nothing else. The slips themselves contain the order details Restora sends, which are printed and not stored beyond that log of order keys.
- **Uninstall:** quit it from the tray icon, turn off "Start with Windows" there, and delete the exe and the `%APPDATA%\restora` folder.


## Integrated Windows setup (signed PWA + headless printing — candidate, NOT released)

The proposed single-EXE setup is `packaging/RestoraSetup.iss` (Inno Setup).
It embeds **two separately built components**: the existing Rust print engine and
a **genuine Microsoft Edge-hosted PWA MSIX generated using PWABuilder** for
`https://app.restora.live/`. The PWA remains the only staff-facing window.
The print engine now runs without a tray or local diagnostics UI by default. Manual launch opens the installed Restora PWA when registered, otherwise the HTTPS app; --background stays silent.

**This cannot be published yet.** The repository has *no production-signed
Restora PWA MSIX, no verified PWA Partner Center identity, and no actual
Windows 10/11 installation or printer acceptance result for this combination*.
This branch changes the Store Agent and portable restora.exe to the headless behavior described above; the integrated signed-PWA installer remains a separate unreleased candidate.
Do not publish or offer the integrated installer until these are supplied and
validated. A customer must not install a self-signed package or a trust root.

### How to build after the real PWA package is available

1. Generate the **modern Microsoft Edge** PWA package from PWABuilder for
   `https://app.restora.live/`. Reserve a **separate PWA package identity** in
   Partner Center; do not reuse the Agent's `BadarDigital.RestoraPrint`
   package identity. Sign the MSIX for direct distribution using a
   publicly trusted signing certificate, or distribute through Microsoft
   Store signing. The Store's unsigned submission MSIX is **not** a signed
   direct-download installer.
2. Record the exact MSIX `Identity.Name`, `Identity.Publisher`,
   `Application.Id` and SHA-256 digest. They are verified **before** Windows
   installs the PWA. Never pass dummy identity strings.
3. Build the Rust agent using the existing verified Windows toolchain, and
   compile `packaging/RestoraSetup.iss` with a current supported Inno Setup:

```powershell
cargo build --release --locked
$version = ((cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages |
    Where-Object name -eq 'restora-pc-agent' | Select-Object -First 1).version
$msix = 'C:\signed\RestoraPwa.msix'
$sha = (Get-FileHash -LiteralPath $msix -Algorithm SHA256).Hash
iscc.exe /DAppVersion=$version /DPwaSource=$msix /DPwaSha256=$sha /DPwaPackageName=YOUR_VERIFIED_PWA_PACKAGE /DPwaPublisher="YOUR_VERIFIED_PUBLISHER" /DPwaApplicationId=YOUR_VERIFIED_APPLICATION_ID packaging/RestoraSetup.iss
```

The resulting `dist/Restora-Setup-<version>-x64.exe` **must also be signed**
for production distribution. `scripts/provision-pwa.ps1` verifies the
package hash and manifest identity, then uses Windows' supported
`Add-AppxPackage` to install the **trusted, signed** MSIX for the current
user. It writes the installed AppUserModelID to
`%APPDATA%\\restora\\pwa-aumid.txt`. On manual launch, the agent checks a provisioned signed PWA package first, then a browser-installed Edge/Chrome PWA registered for web+restora, and finally falls back to the normal HTTPS website.
Windows startup launches the print engine silently.

**Migration policy:** if a separately installed Edge/Chrome Restora PWA is
detected, the provisioning script stops rather than creating a duplicate.
The migration from a browser-installed PWA to the packaged PWA must be
tested and supported before release. Existing Store Agent installations
also need a tested migration to avoid simultaneous listeners on
`127.0.0.1:17777`. Do not overwrite browser enterprise policies or use
Chromium's undocumented PWA install command-line switches.

**Release gates:** Windows 10/11, clean PCs, upgrading live Agent versions,
Store/portable migration, existing PWAs, locked/running apps, failed
signature and hash, interrupted installs, silent startup, local network
permissions, branch isolation, offline queues, Electron's `--embedded`
mode, USB/network printers, 58/80 mm slips and paper-out/retries. Also
review installer uninstall behavior; the PWA remains installed unless the
user uninstalls it separately. Merging code does not publish the setup.

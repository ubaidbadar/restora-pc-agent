# Restora PC Agent

A small Windows tray app that prints [Restora](https://restora.live) slips (kitchen tickets and receipts) on the printers connected to a PC, with or without internet. Open source under the [MIT licence](LICENSE).

**Install:** [Restora Agent on Microsoft Store](https://apps.microsoft.com/detail/9p6vnx5kvjl6?hl=en-US&gl=PK). This repository also builds a separate portable executable for testing and legacy installations.

## How it works
- **Open:** start Restora Agent from Windows; it opens the [Restora web app](https://app.restora.live) or the Printers settings if the PC has not yet connected. The tray icon opens Restora; **Agent diagnostics** opens the local status page.
- **Connect:** sign in to Restora, select the correct restaurant branch, and approve the browser's one-time permission to connect to apps on this computer. Restora passes a temporary backend-issued credential directly to the agent. Staff do **not** type a code in the normal setup. Advanced diagnostics retain manual connection for recovery.
- **Sync:** the agent polls the authenticated backend for queued slips every two seconds and checks for changed printer configuration on the same connection. Windows printer discovery still runs every five minutes.
- **Print:** kitchen tickets and receipts are submitted through the Windows spooler. A journal remembers accepted copies so automatic retries do not repeat previous copies after partial failures. Jobs for separate physical printers process independently.
- **Offline:** on the same authorized branch and PC, Restora can submit slips through the protected `http://127.0.0.1:17777/print` endpoint when the internet is unavailable. Remote jobs remain queued on the backend until the agent reconnects.
- **Status:** the tray and diagnostics distinguish disconnected backend, missing Windows printers, and failures. Windows spooler acceptance does not guarantee physical paper output.

## Security and compatibility
- The agent listens on **loopback only** (`127.0.0.1:17777`), requires the approved browser Origin for writes, and checks the paired restaurant branch and configured printer before offline printing. Chrome or Edge may ask for local network/loopback permission; this permission cannot be bypassed.
- Agent bearer credentials are encrypted using Windows DPAPI in `%APPDATA%\\restora\\agent.json`; existing plaintext configs migrate when loaded.
- An old agent installation may not know its branch until reconnected. Avoid using the offline local-print path across branches until all PCs have upgraded and their branch identity is established.
- The printer-copy journal is `%APPDATA%\\restora\\printed.log`. A crash between spooler acceptance and journal write can leave an ambiguous print; **do not promise exactly-once physical printing**.
- On a Windows login auto-start, the app stays in the tray without opening a browser. Manual launch opens Restora.
- Only release after testing real USB/network printers, 58/80mm, offline sync, paper-out, reboot, browser permission and Store MSIX upgrade. The Store app and portable build have different packaging and startup rules.

## Local server (127.0.0.1:17777)
`GET /status`, `GET /printers`, `GET /config`, `POST /print { branchId, printer, kind, orderKey, width?, copies?, order, header? }`, and the advanced diagnostics page. `POST /setup/pair { code, api? }` is called by the authorized Restora web app, or manually as an advanced recovery step. Only trusted Restora origins and the loopback Host are allowed. `RESTORA_API` may override the default `https://restora.live/api/v1` in development.

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

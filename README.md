# Restora PC Agent

A small Windows tray app that prints [Restora](https://restora.live) slips (kitchen tickets and receipts) on the printers connected to a PC, with or without internet. Open source under the [MIT licence](LICENSE).

**Download:** the newest `restora-pc-agent.exe` is on the [Releases page](https://github.com/ubaidbadar/restora-pc-agent/releases/latest). Run it, and when its setup page opens, type the 6-digit code from Restora (Settings, then Printers).

## How it works
- **Pair:** in Restora, Settings, then Printers, add the PC and get a 6-digit code. Run the agent, open its setup page (tray icon, then Open setup page, or http://127.0.0.1:17777) and type the code. The agent keeps its key in `%APPDATA%\restora\agent.json`.
- **Online:** every 2 seconds the agent asks the backend for waiting slips, prints them on the printer each one names (RAW ESC/POS through the Windows spooler) and reports printed or failed. A slip that will not print (printer off, no paper) is tried again after 2, 5, 10 and 30 seconds, then every minute for 15 minutes, then reported failed. Slips wait at the backend while the PC is offline.
- **Offline, same PC:** the web app posts a slip to `http://127.0.0.1:17777/print`; the agent prints it and remembers it (`printed.log`), so the same order is not printed again when it syncs.
- **Printers:** the agent sends the Windows printer list on pairing and every 5 minutes. Staff name them in Settings.

## Local server (127.0.0.1:17777)
`GET /status`, `GET /printers`, `POST /print { printer, kind: "kitchen" | "receipt", orderKey, width?: 48 | 32, copies?, order, header? }`, and the setup page. `POST /setup/pair { code, api? }` pairs: the web app open on this PC sends it by itself, with the API it uses (only restora.live or dev.restora.live). It answers only the Restora origins (`RESTORA_ORIGINS` replaces them) and the host `127.0.0.1:17777`. `RESTORA_API` replaces the API address (default https://restora.live/api/v1).

## Develop
```
cargo test                                     # slip tests
cargo run                                      # runs on Linux too; printing only works on Windows
cargo build --release --target x86_64-pc-windows-msvc
```
`test/index.html` calls the local server from a browser page. Every push to main publishes a new release by itself (`.github/workflows/release.yml`): the next patch version, or add `#minor` / `#major` to the commit message to raise that part. With the R2 secrets set, the exe is also uploaded to the Cloudflare bucket (see the top of the workflow).

## Code signing policy
Release builds are made by GitHub Actions from this repository (`.github/workflows/release.yml`), so a signature confirms that the exe is an automated build of the source code here.

- **Status:** an application for free code signing from SignPath Foundation is pending. Until it is approved, releases are unsigned and Windows may show an "unknown publisher" warning. This section will be updated when signing starts.
- **Team roles:** author, reviewer and approver: [Ubaid Badar](https://github.com/ubaidbadar). Every release needs approval before it is signed.
- **Privacy:** this app does not collect or send personal data. It talks only to the Restora API (to fetch the slips for this PC and report them printed) and to the printers on this PC, plus the Restora web app on this PC over `127.0.0.1`. It keeps its settings and a log of printed slips in `%APPDATA%\restora` and nothing else. The slips themselves contain the order details Restora sends, which are printed and not stored beyond that log of order keys.
- **Uninstall:** quit it from the tray icon, turn off "Start with Windows" there, and delete the exe and the `%APPDATA%\restora` folder.

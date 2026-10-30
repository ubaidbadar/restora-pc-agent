# Restora PC Agent

A small Windows tray app that prints Restora slips on the printers connected to a PC. Design: https://claude.ai/artifact/7x5JFNgGgipwpZpAUc1BEV

## How it works
- **Pair:** in Restora, Settings, then Printers, add the PC and get a 6-digit code. Run the agent, open its setup page (tray icon, then Open setup page, or http://127.0.0.1:17777) and type the code. The agent keeps its key in `%APPDATA%\restora\agent.json`.
- **Online:** every 2 seconds the agent asks the backend for waiting slips, prints them on the printer each one names (RAW ESC/POS through the Windows spooler) and reports printed or failed. A slip that will not print (printer off, no paper) is tried again after 2, 5, 10 and 30 seconds, then every minute for 15 minutes, then reported failed. Slips wait at the backend while the PC is offline.
- **Offline, same PC:** the web app posts a slip to `http://127.0.0.1:17777/print`; the agent prints it and remembers it (`printed.log`), so the same order is not printed again when it syncs.
- **Printers:** the agent sends the Windows printer list on pairing and every 5 minutes. Staff name them in Settings.

## Local server (127.0.0.1:17777)
`GET /status`, `GET /printers`, `POST /print { printer, kind: "kitchen" | "receipt", orderKey, width?: 48 | 32, copies?, order, header? }`, and the setup page. It answers only the Restora origins (`RESTORA_ORIGINS` replaces them) and the host `127.0.0.1:17777`. `RESTORA_API` replaces the API address (default https://restora.live/api/v1).

## Develop
```
cargo test                                     # slip tests
cargo run                                      # runs on Linux too; printing only works on Windows
cargo build --release --target x86_64-pc-windows-msvc
```
`test/index.html` calls the local server from a browser page. Every push to main publishes a new release by itself (`.github/workflows/release.yml`): the next patch version, or add `#minor` / `#major` to the commit message to raise that part.

# Restora PC Agent

A small Windows tray app that prints Restora slips on local thermal printers. Design: https://claude.ai/artifact/7x5JFNgGgipwpZpAUc1BEV

This first slice has no tray, pairing, queue or installer yet. It has:

- `printers.rs` lists Windows printers (`EnumPrintersW`) and sends RAW bytes to one (`WritePrinter`).
- `escpos.rs` is the ESC/POS builder; `slips.rs` builds the kitchen slip and receipt from the web app's order JSON.
- `local.rs` is the server on `127.0.0.1:17777`: `GET /status`, `GET /printers`, `POST /print`. It only answers the Restora origins and the host `127.0.0.1:17777`.
- `test/index.html` calls it from a browser page.

`POST /print` body: `{ printer, kind: "kitchen" | "receipt", orderKey, width?: 48 | 32, order, header? }`. The same `orderKey + kind + printer` prints once per run.

```
cargo test                                   # slip tests
cargo build --release --target x86_64-pc-windows-msvc   # on Windows
RESTORA_ORIGINS=http://localhost:3000 restora-pc-agent
```

# Privacy Policy — Restora PC Agent

Restora PC Agent is a small Windows tray app that prints Restora restaurant slips on printers connected to a PC.

## What the app does

- It pairs with a Restora account via a 6-digit code.
- It polls the Restora backend for waiting slips.
- It prints order slips as RAW ESC/POS through the Windows spooler.
- It sends the Windows printer list to the Restora backend on pairing and every 5 minutes, so staff can name printers in Restora Settings.
- It runs a local server at http://127.0.0.1:17777 for status, printers, and printing. That local server answers only the Restora origins and the host 127.0.0.1:17777.

## What the app does not collect

- No payment card data.
- No passwords.

## Data sent to Restora

Data sent to the Restora backend is limited to what is needed for printing: the pairing key, the printer list, and order slip data for printing, together with printed/failed status reports.

## Local storage

The agent keeps its pairing key in %APPDATA%\restora\agent.json and remembers printed slips in printed.log, so the same order is not printed again when it syncs.

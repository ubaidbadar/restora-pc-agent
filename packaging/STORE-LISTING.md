# Store listing draft — Restora Agent

> Draft for the Partner Center submission. Nothing here has been submitted.
>
> NAME HOLD (2026-11-09): the app is now "Restora Agent", not "Restora
> Print". Partner Center is still checking whether the reserved draft
> product "Restora Print" (Store ID `9P6VNX5KVJL6`, In draft) can be
> renamed to "Restora Agent" or a new product must be created. The
> Identity Name below is EXPECTED but UNCONFIRMED — do NOT submit
> until the reserved identity is confirmed.
>
> The MSIX is built by `.github/workflows/msix.yml` (unsigned — Microsoft
> signs it after Store certification). Product identity must match the
> manifest exactly: Name `BadarDigital.RestoraPrint` (expected,
> unconfirmed), Publisher `CN=72D821D7-890E-4CCA-94D2-675D84F355AA`,
> Version `0.1.17.0`.
> Partner Center product: "Restora Print" (rename pending), Store ID
> `9P6VNX5KVJL6`, In draft —
> https://partner.microsoft.com/en-us/dashboard/products/9P6VNX5KVJL6/overview

## App name

Restora Agent

## Short description

Prints Restora kitchen tickets and receipts on the printers connected to a
Windows PC — online, or offline from the Restora web app on the same PC.

## Description

Restora Agent is the Windows print agent for Restora
([restora.live](https://restora.live)), the restaurant system for running a
restaurant from a phone.

- Pair in minutes: in Restora, go to Settings, then Printers, add the PC and
  type the 6-digit code on the agent's setup page.
- Online: the agent checks Restora every 2 seconds for waiting slips and
  prints them on the printer each one names (RAW ESC/POS through the Windows
  spooler), then reports printed or failed. A slip that will not print is
  retried, then reported failed.
- Offline, same PC: the Restora web app posts a slip to the agent on the
  same PC and it prints without internet. Printed slips are remembered, so
  the same order is never printed twice when it syncs.
- Printers: the agent sends the Windows printer list to Restora on pairing
  and every 5 minutes, so staff can name printers in Restora Settings.

The agent lives in the system tray. It needs a Restora account and a
restaurant set up in Restora to print anything — on its own it has nothing
to print. For a reviewer test account and pairing instructions, see the
submission notes in Partner Center.

## Category

Productivity

## Privacy policy URL

https://github.com/ubaidbadar/restora-pc-agent/blob/main/PRIVACY.md

## Support URL

https://restora.live

## runFullTrust justification (restricted capability)

Restora Print is a Win32 desktop app, not a UWP app, so it needs
`runFullTrust` to run at all. It uses full trust for:

- RAW ESC/POS printing through the Windows print spooler (kitchen tickets
  and receipts on the PC's printers);
- running as a system-tray app with no console window;
- keeping its pairing key and printed-slip log in `%APPDATA%\restora`;
- polling the Restora backend over HTTPS for waiting slips;
- running a loopback-only HTTP server on `127.0.0.1:17777` so the Restora
  web app on the same PC can post slips for offline printing.

It does not download or run other executables, does not inject into other
processes, and collects no payment-card data or passwords (see PRIVACY.md).

## Loopback note — still to verify on a real Windows PC

The agent listens on `127.0.0.1:17777` and the Restora web app
(`https://app.restora.live`) posts slips to it from the browser. The package
declares `privateNetworkClientServer`, but loopback behaviour of a packaged
app must be verified after install on a real Windows PC: install the Store
package, open the Restora web app, and confirm `GET /status` and
`POST /print` on `http://127.0.0.1:17777` still work. If the packaged app
needs a loopback exemption, test
`CheckNetIsolation LoopbackExempt -a -n=<package family name>` and record
the result — do not assume it is automatic or Store-approved. A build alone
cannot verify printer or browser connectivity.

## Screenshots — still needed

No Store screenshots exist in the repo yet. Before submission, capture on a
real Windows PC: the tray icon and menu, the setup/pairing page, and a
printed kitchen ticket / receipt. The PWA screenshots in restora-web do not
show this app.

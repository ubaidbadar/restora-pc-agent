# Microsoft Store trial

This path packages the Rust tray app as a full-trust desktop MSIX. Microsoft signs
the package after Store certification. It does not sign the standalone release exe.
This is a packaging candidate, not a certified or Windows-tested release.

## Reserve the app identity

1. Register at https://storedeveloper.microsoft.com/ and complete identity verification.
2. Create an MSIX/packaged app and reserve a name such as **Restora Print**.
3. Under **Product management > Product identity**, copy `Package/Identity/Name`,
   `Package/Identity/Publisher`, and `Package/Properties/PublisherDisplayName` exactly.
   These are public identity values, not passwords or API secrets.

## Build on Windows

Install Rust with the MSVC toolchain, Visual Studio C++ Build Tools, and a Windows
10/11 SDK. No GitHub Actions minutes are required. Before the first Store build,
set the package version in `Cargo.toml` to `1.0.0` and refresh `Cargo.lock` with
`cargo check`. Commit both together. Each later Store upload needs a higher version.
The script uses that version plus `.0`, as required for Store submissions.

From the repository root, run PowerShell using your real Partner Center values:

```powershell
.\scripts\build-store.ps1 -IdentityName 'YOUR_PACKAGE_NAME' -Publisher 'CN=YOUR_PUBLISHER_ID' -PublisherDisplayName 'YOUR_PUBLISHER_NAME'
```

Output: `target\msix\RestoraPrint_<version>_x64.msix`. MakeAppx validates the
manifest during packing. The script builds with the `store` feature and static CRT.
The MSIX is unsigned and intended for Partner Center upload, not customer installation.
Do not invent the publisher identity or install a development certificate on customer PCs.

## Verify before submission

On a Windows test PC, locally sign a copy with a development certificate and trust
its **public** certificate on that test PC only. Never share its private key.
Use Microsoft's test-signing guide below. Run the Windows App Certification Kit.

Check first-run pairing, tray visibility and Quit, enumeration of actual printers,
online receipt/kitchen printing, offline printing from the Restora browser app,
duplicate prevention, restart, package upgrade, and uninstall. Test on a clean PC
without C++ Build Tools to verify runtime dependencies. A build alone cannot verify
printer or browser connectivity.

For the Store build, startup is initially disabled. The tray menu opens Windows
Settings > Apps > Startup; enable Restora Print there and verify it after signing
out and back in. Store builds never write the portable app's registry startup entry.
Before migration, disable **Start with Windows** in the old portable app and quit
it: both versions use the same local port. Existing AppData settings may be redirected
by packaging; test pairing again rather than assuming automatic data migration.

## Prepare the submission

Upload the MSIX to Partner Center and supply the listing, screenshots, support and
public privacy-policy URLs. Disclose the Restora account/backend requirement and
provide a working reviewer test account plus pairing instructions. Explain the
`runFullTrust` capability: Windows printer spooler access, tray operation, outbound
Restora HTTPS requests, and a loopback-only HTTP server for offline browser printing.
Microsoft reviews restricted capabilities and can request more information.

Review pricing, markets and publication settings before submitting. No upload,
account creation, submission or publication is performed by the build script.

References:
- https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements
- https://learn.microsoft.com/en-us/windows/msix/package/sign-msix-package-guide
- https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions

# Publishing Argus Local Windows release (ARG-33)

Portable Windows x64 zip — no installer, no Rust required for end users.

## Package locally (this machine)

From the repo root on Windows:

```powershell
.\scripts\package-windows-release.ps1
```

What it does:

1. `cargo build --release`
2. Stages `argus-local.exe` + SHA256 under `proof/arg-33/` and `dist/`
3. Creates `argus-local-windows-x64.zip` + `argus-local-windows-x64.zip.sha256`

Binaries under `dist/` and `proof/arg-33/*.exe` / `*.zip` are gitignored — do not commit them.

## Create the GitHub Release (separate, needs approval)

**Do not run this until Gadi/user approval.** Auto-review may block `gh release create`.

Suggested commands (after packaging):

```powershell
$tag = "v0.1.0"   # bump as needed
$notes = @"
## Argus Local $tag (Windows x64)

Portable exe — unzip and run. No Rust toolchain required.

### Artifacts
- ``argus-local-windows-x64.zip``
- ``argus-local-windows-x64.zip.sha256``

### Verify
``````powershell
Get-FileHash .\argus-local-windows-x64.zip -Algorithm SHA256
``````

### Run
``````powershell
Expand-Archive .\argus-local-windows-x64.zip -DestinationPath .\argus-local
.\argus-local\argus-local.exe open
.\argus-local\argus-local.exe tui --fixtures
``````
"@

gh release create $tag `
  --title "Argus Local $tag (Windows x64)" `
  --notes $notes `
  dist/argus-local-windows-x64.zip `
  dist/argus-local-windows-x64.zip.sha256
```

After publish, confirm the README **Download** link (`/releases/latest`) resolves and update Linear ARG-33 with the Release URL.

## Proof without Rust in PATH

```powershell
$tmp = Join-Path $env:TEMP "argus-local-arg33-proof"
Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive .\dist\argus-local-windows-x64.zip -DestinationPath $tmp
# Ensure cargo is not required:
$env:Path = ($env:Path -split ';' | Where-Object { $_ -notmatch 'cargo|rustup|\\.cargo\\bin' }) -join ';'
& "$tmp\argus-local.exe" --help
& "$tmp\argus-local.exe" tui --fixtures --proof "$tmp\proof-out"
```
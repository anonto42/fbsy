<#
.SYNOPSIS
  fbsy installer for Windows.

.DESCRIPTION
  Downloads the matching release binary, verifies its checksum, installs it to
  %LOCALAPPDATA%\Programs\fbsy, adds that directory to the User PATH, and runs
  `fbsy install` to create data directories.

  Run with:
    irm https://raw.githubusercontent.com/anonto42/fbsy/main/scripts/install.ps1 | iex

  Environment overrides:
    $Env:FBSY_VERSION      install a specific version instead of latest
    $Env:FBSY_INSTALL_DIR  install dir (default: %LOCALAPPDATA%\Programs\fbsy)
    $Env:FBSY_NO_VERIFY=1  skip checksum verification
#>

$ErrorActionPreference = 'Stop'
$Repo = 'anonto42/fbsy'

function Die($msg) { Write-Error "error: $msg"; exit 1 }

# 1. Detect architecture -> asset (only x86_64 is built today)
$arch = $Env:PROCESSOR_ARCHITECTURE
if ($arch -ne 'AMD64') {
    Die "unsupported architecture '$arch' (only x86_64 Windows builds are published)"
}
$asset = 'fbsy-windows-x86_64.exe'

# 2. URL base
if ($Env:FBSY_VERSION) {
    $base = "https://github.com/$Repo/releases/download/v$($Env:FBSY_VERSION)"
} else {
    $base = "https://github.com/$Repo/releases/latest/download"
}

$tmp = Join-Path $Env:TEMP ("fbsy-install-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
$binTmp = Join-Path $tmp 'fbsy.exe'

try {
    # 3. Download the binary
    Write-Host "Downloading $asset ..."
    Invoke-WebRequest -Uri "$base/$asset" -OutFile $binTmp -UseBasicParsing

    # 4. Verify checksum (best effort)
    if ($Env:FBSY_NO_VERIFY -ne '1') {
        $sumsTmp = Join-Path $tmp 'checksums.txt'
        try {
            Invoke-WebRequest -Uri "$base/checksums.txt" -OutFile $sumsTmp -UseBasicParsing
            $line = Select-String -Path $sumsTmp -Pattern "\s$([regex]::Escape($asset))$" |
                Select-Object -First 1
            if ($line) {
                $expected = ($line.Line -split '\s+')[0].ToLower()
                $actual = (Get-FileHash -Path $binTmp -Algorithm SHA256).Hash.ToLower()
                if ($expected -ne $actual) {
                    Die "checksum mismatch for $asset (expected $expected, got $actual)"
                }
                Write-Host "Checksum verified." -ForegroundColor Green
            } else {
                Write-Host "No checksum entry for $asset; skipping verification."
            }
        } catch {
            Write-Host "checksums.txt not available; skipping verification."
        }
    }

    # 5. Install to bin dir
    if ($Env:FBSY_INSTALL_DIR) {
        $installDir = $Env:FBSY_INSTALL_DIR
    } else {
        $installDir = Join-Path $Env:LOCALAPPDATA 'Programs\fbsy'
    }
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $dst = Join-Path $installDir 'fbsy.exe'
    Move-Item -Path $binTmp -Destination $dst -Force
    Write-Host "Installed fbsy to $dst" -ForegroundColor Green

    # 6. Add to User PATH (idempotent)
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath) { $userPath = '' }
    $entries = $userPath -split ';' | Where-Object { $_ -ne '' }
    if ($entries -notcontains $installDir) {
        $newPath = if ($userPath) { "$userPath;$installDir" } else { $installDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host "Added $installDir to your User PATH." -ForegroundColor Green
    }

    # 7. Finish setup (data dirs)
    try { & $dst install | Out-Null } catch { }

    Write-Host ""
    Write-Host "Done. Open a new terminal, then run: fbsy --help" -ForegroundColor Green
}
finally {
    Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

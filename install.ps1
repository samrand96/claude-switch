<#
.SYNOPSIS
  Installs Claude Switch (cswitch) on Windows and starts the setup wizard.

.DESCRIPTION
  Prefer the setup .exe from the Releases page for a regular install (it adds an entry to
  Settings > Apps). This script is the scriptable alternative; both end up running
  `cswitch install`, which needs no administrator rights.

.EXAMPLE
  irm https://raw.githubusercontent.com/samrand96/claude-switch/HEAD/install.ps1 | iex

.EXAMPLE
  .\install.ps1 -Path .\cswitch.exe

.EXAMPLE
  .\install.ps1 -Version v1.2.3 -NoSetup
#>
param(
    # Use a local cswitch.exe instead of downloading a release.
    [string]$Path = "",
    # Release tag to install, e.g. v1.2.3 or nightly (default: the latest release).
    [string]$Version = "",
    # GitHub repository to download from.
    [string]$Repo = "samrand96/claude-switch",
    # Install only; don't start the setup wizard.
    [switch]$NoSetup
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Write-Step($text) { Write-Host "  > $text" -ForegroundColor DarkYellow }

function Get-OsArchitecture {
    # $env:PROCESSOR_ARCHITECTURE says AMD64 inside an emulated x64 PowerShell on ARM64 Windows.
    try {
        return [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
        if ($env:PROCESSOR_ARCHITEW6432) { return $env:PROCESSOR_ARCHITEW6432 }
        return $env:PROCESSOR_ARCHITECTURE
    }
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("cswitch-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null

try {
    if ($Path) {
        $exe = (Resolve-Path $Path).Path
    } else {
        $arch = if ((Get-OsArchitecture) -match "^arm64$") { "aarch64" } else { "x86_64" }
        $asset = "cswitch-$arch-pc-windows-msvc.zip"
        $base = if ($Version) { "https://github.com/$Repo/releases/download/$Version" } else { "https://github.com/$Repo/releases/latest/download" }
        Write-Step "Downloading $base/$asset"
        $zip = Join-Path $work $asset
        try {
            Invoke-WebRequest -Uri "$base/$asset" -OutFile $zip -UseBasicParsing
        } catch {
            if ($Version) { throw }
            # "latest" skips pre-releases: with no stable release yet, take the nightly.
            $base = "https://github.com/$Repo/releases/download/nightly"
            Write-Step "No stable release yet, downloading the nightly build"
            Invoke-WebRequest -Uri "$base/$asset" -OutFile $zip -UseBasicParsing
        }
        $sums = Join-Path $work "SHA256SUMS.txt"
        try {
            Invoke-WebRequest -Uri "$base/SHA256SUMS.txt" -OutFile $sums -UseBasicParsing
        } catch {
            $sums = $null # older releases have no SHA256SUMS.txt
        }
        if ($sums) {
            $line = Select-String -Path $sums -Pattern " $([regex]::Escape($asset))$" | Select-Object -First 1
            if ($line) {
                $expected = $line.Line.Split(" ")[0]
                $actual = (Get-FileHash $zip -Algorithm SHA256).Hash
                if ($expected -ne $actual) { throw "checksum mismatch for $asset - download again" }
                Write-Step "Checksum verified"
            }
        }
        Expand-Archive -Path $zip -DestinationPath $work -Force
        $exe = Join-Path $work "cswitch.exe"
    }

    if (-not (Test-Path $exe)) { throw "cswitch.exe not found at $exe" }
    Unblock-File -Path $exe -ErrorAction SilentlyContinue

    $home_ = if ($env:CSWITCH_HOME) { $env:CSWITCH_HOME } else { Join-Path $env:USERPROFILE ".cswitch" }
    Write-Step "Installing to $home_\bin"
    & $exe install
    if ($LASTEXITCODE -ne 0) { throw "cswitch install failed ($LASTEXITCODE)" }

    $installed = Join-Path $home_ "bin\cswitch.exe"
    $env:Path = "$home_\bin;$env:Path"

    if (-not $NoSetup) {
        Write-Step "Starting setup"
        & $installed setup
    }
    Write-Host ""
    Write-Host "  Done. Open a new terminal and run: cswitch" -ForegroundColor Green
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

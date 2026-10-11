$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $root 'Self-disk'
$pcBinary = Join-Path $root 'target\debug\audiorouter-desktop.exe'
$pcOutput = Join-Path $outputDirectory 'AudioRouter.exe'
$uiSource = Join-Path $root 'assets\ui\pc-ui-mock.html'
$uiCopy = Join-Path $outputDirectory 'pc-ui-mock.html'

New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
Copy-Item -LiteralPath $uiSource -Destination $uiCopy -Force

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Push-Location $root
try {
    cargo build --package audiorouter-desktop
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $pcBinary)) {
    throw "PC executable was not generated: $pcBinary"
}
Move-Item -LiteralPath $pcBinary -Destination $pcOutput -Force

& (Join-Path $PSScriptRoot 'build-android.ps1')
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Get-ChildItem -LiteralPath $outputDirectory -Filter 'AudioRouter.*' | Select-Object FullName, Length, LastWriteTime

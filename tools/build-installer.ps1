param(
    [string]$Version = '1.0.0'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$application = Join-Path $root 'target\release\audiorouter-desktop.exe'
$innoCompiler = Join-Path $root '..\..\..\..\Self-Editor\Self-InnoSetup\Inno Setup 7\ISCC.exe'
$script = Join-Path $root 'installer\AudioRouter.iss'

if (-not (Test-Path -LiteralPath $application)) {
    throw "请先构建 PC Release 程序：$application"
}
if (-not (Test-Path -LiteralPath $innoCompiler)) {
    throw "找不到 Inno Setup 编译器：$innoCompiler"
}

& $innoCompiler "/DMyAppVersion=$Version" $script
if ($LASTEXITCODE -ne 0) {
    throw "Inno Setup 编译失败，退出码：$LASTEXITCODE"
}

$output = Join-Path $root "Self-disk\AudioRouter-Setup-$Version.exe"
if (-not (Test-Path -LiteralPath $output)) {
    throw "未生成安装程序：$output"
}
Get-Item -LiteralPath $output | Select-Object FullName, Length, LastWriteTime

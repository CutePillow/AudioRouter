$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$jdk = 'C:\Program Files\Microsoft\jdk-17.0.20.101-hotspot'
$sdk = Join-Path $root '.tools\android-sdk'
$androidUserHome = Join-Path $root '.tools\android-user-home'
$gradleUserHome = Join-Path $root '.tools\gradle-user-home'
$gradle = Join-Path $root '.tools\gradle\gradle-8.9\bin\gradle.bat'
$apk = Join-Path $root 'android\app\build\outputs\apk\debug\app-debug.apk'

if (-not (Test-Path -LiteralPath (Join-Path $jdk 'bin\java.exe'))) {
    throw "JDK 17 not found: $jdk"
}
if (-not (Test-Path -LiteralPath (Join-Path $sdk 'platforms\android-36\android.jar'))) {
    throw "Android SDK platform 36 not found: $sdk"
}
if (-not (Test-Path -LiteralPath $gradle)) {
    throw "Gradle 8.9 not found: $gradle"
}

$env:JAVA_HOME = $jdk
$env:ANDROID_SDK_ROOT = $sdk
$env:ANDROID_HOME = $sdk
$env:ANDROID_USER_HOME = $androidUserHome
$env:GRADLE_USER_HOME = $gradleUserHome
$env:JAVA_TOOL_OPTIONS = "-Duser.home=$env:USERPROFILE"
$null = New-Item -ItemType Directory -Force -Path $androidUserHome
$null = New-Item -ItemType Directory -Force -Path $gradleUserHome
$env:Path = "$jdk\bin;$sdk\platform-tools;$env:Path"

Push-Location (Join-Path $root 'android')
try {
    & $gradle --no-daemon --rerun-tasks assembleDebug
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $apk)) {
    throw "APK was not generated: $apk"
}

$outputDirectory = Join-Path $root 'Self-disk'
$outputApk = Join-Path $outputDirectory 'AudioRouter-debug.apk'
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
Move-Item -LiteralPath $apk -Destination $outputApk -Force
Get-Item -LiteralPath $outputApk | Select-Object FullName, Length, LastWriteTime

# Builds the Android app: android\gen\android\app\build\outputs\apk\arm64\release\app-arm64-release.apk
#   .\build-apk.ps1            the APK to hand out
#   .\build-apk.ps1 -Inspect   test build: WebViews can be inspected over adb, the log is readable with adb
# Needs: Android SDK with NDK, JDK 21, rustup target aarch64-linux-android, npm install in web\.
# Signing: gen\android\keystore.properties (storeFile, keyAlias, password); without it the APK stays unsigned.
#
# Why not `tauri android build`: on Windows it links the built library into the Gradle project with a symbolic
# link, which needs developer mode. This script copies the library instead and runs the same two steps itself.
param([switch]$Inspect)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot

$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$ndk = Get-ChildItem (Join-Path $sdk 'ndk') -Directory | Sort-Object Name | Select-Object -Last 1
if (-not $ndk) { throw "No NDK below $sdk\ndk" }
$bin = Join-Path $ndk.FullName 'toolchains\llvm\prebuilt\windows-x86_64\bin'
$clang = Join-Path $bin 'aarch64-linux-android26-clang.cmd'

$env:ANDROID_HOME = $sdk
$env:NDK_HOME = $ndk.FullName
$env:ANDROID_NDK_HOME = $ndk.FullName
$env:CC_aarch64_linux_android = $clang
$env:CXX_aarch64_linux_android = Join-Path $bin 'aarch64-linux-android26-clang++.cmd'
$env:AR_aarch64_linux_android = Join-Path $bin 'llvm-ar.exe'
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $clang
# What the Tauri CLI tells wry: where the app's Kotlin glue goes and how the library is called.
$gen = Join-Path $PSScriptRoot 'gen\android'
$kotlin = Join-Path $gen 'app\src\main\java\local\flimmer\app\generated'
New-Item -ItemType Directory -Force $kotlin | Out-Null
$env:WRY_ANDROID_PACKAGE = 'local.flimmer.app'
$env:WRY_ANDROID_LIBRARY = 'flimmer_android'
$env:WRY_ANDROID_KOTLIN_FILES_OUT_DIR = $kotlin
# ... and tauri-build: it writes the Gradle glue (tauri.settings.gradle, tauri.build.gradle.kts) into the project.
$env:TAURI_ANDROID_PROJECT_PATH = $gen
$env:TAURI_ANDROID_PACKAGE_UNESCAPED = 'local.flimmer.app'
# Version of the APK, as the Tauri CLI derives it from tauri.conf.json (1.2.3 -> code 1002003).
$version = (Get-Content (Join-Path $PSScriptRoot 'tauri.conf.json') -Raw | ConvertFrom-Json).version
$v = $version.Split('.') | ForEach-Object { [int]$_ }
$code = $v[0] * 1000000 + $v[1] * 1000 + $v[2]
Set-Content (Join-Path $gen 'app\tauri.properties') "tauri.android.versionName=$version`ntauri.android.versionCode=$code" -Encoding ascii
# One heavy job at a time, the machine stays usable.
$env:CARGO_BUILD_JOBS = '4'

Push-Location (Join-Path $root 'web')
try {
    npm run build
    if ($LASTEXITCODE -ne 0) { throw 'web build failed' }
} finally { Pop-Location }

Push-Location $root
try {
    $features = if ($Inspect) { @('--features', 'inspect') } else { @() }
    cargo build --release -p flimmer-android --target aarch64-linux-android @features
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally { Pop-Location }

$libs = Join-Path $gen 'app\src\main\jniLibs\arm64-v8a'
New-Item -ItemType Directory -Force $libs | Out-Null
Copy-Item (Join-Path $root 'target\aarch64-linux-android\release\libflimmer_android.so') $libs -Force

Push-Location $gen
try {
    # The Gradle project's own Rust step would call the Tauri CLI again; the library is already in place.
    & .\gradlew.bat assembleArm64Release -x rustBuildArm64Release --console=plain
    if ($LASTEXITCODE -ne 0) { throw 'gradle build failed' }
} finally { Pop-Location }

$apk = Join-Path $gen 'app\build\outputs\apk\arm64\release\app-arm64-release.apk'
if (-not (Test-Path $apk)) { $apk = Join-Path $gen 'app\build\outputs\apk\arm64\release\app-arm64-release-unsigned.apk' }
'{0}  ({1:N1} MB)' -f $apk, ((Get-Item $apk).Length / 1MB)

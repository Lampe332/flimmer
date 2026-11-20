# Builds the per-user installer target\release\bundle\nsis\Flimmer_<version>_x64-setup.exe:
# web UI, then Flimmer.exe without build-machine paths (user name, cargo folder), then the NSIS bundle.
# Needs: tools\ffmpeg\bin and desktop\extensions\ubol (see THIRD_PARTY_NOTICES.md), npm install in web\.
# 'Continue': npm and cargo write progress to stderr; failures are caught by their exit codes below.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }

# Panic messages carry source paths; map the local ones to neutral names.
$env:RUSTFLAGS = "--remap-path-prefix=$cargoHome=/cargo --remap-path-prefix=$root=/flimmer"
# Same for __FILE__ in bundled C code (aws-lc, SQLite): MSVC trims these prefixes, long and 8.3 form.
$short = (New-Object -ComObject Scripting.FileSystemObject).GetFolder($cargoHome).ShortPath
$trim = "/d1trimfile:$cargoHome\ /d1trimfile:$short\"
$env:CFLAGS = $trim
$env:CXXFLAGS = $trim
# One heavy job at a time, the machine stays usable.
$env:CARGO_BUILD_JOBS = '4'

Push-Location (Join-Path $root 'web')
try {
    npm run build
    if ($LASTEXITCODE -ne 0) { throw 'web build failed' }
} finally { Pop-Location }

Push-Location $PSScriptRoot
try {
    & (Join-Path $root 'web\node_modules\.bin\tauri.cmd') build --ci
    if ($LASTEXITCODE -ne 0) { throw 'tauri build failed' }
} finally { Pop-Location }

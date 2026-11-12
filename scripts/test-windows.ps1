$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$native = Join-Path $repo 'native'
$nativeManifest = Join-Path $native 'src-tauri\Cargo.toml'
$nativeTarget = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $native 'src-tauri\target' }
$cargo = (Get-Command cargo.exe -ErrorAction SilentlyContinue).Source
if (!$cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }

$hostLine = & $cargo -vV | Where-Object { $_ -like 'host: *' }
if ($LASTEXITCODE -ne 0 -or !$hostLine) { throw 'Rust toolchain not found' }
$triple = $hostLine.Substring(6)
if ($triple -notlike '*-pc-windows-*') { throw "Expected a Windows Rust toolchain, got $triple" }

Push-Location $repo
try {
    & node --test --test-reporter=dot tests/smoke.mjs tests/*.test.mjs
    if ($LASTEXITCODE) { throw 'Island Lab web tests failed' }
    & node native/scripts/prepare.mjs
    if ($LASTEXITCODE) { throw 'Frontend preparation failed' }

    $nativeArgs = @('--release', '--target', $triple, '--target-dir', $nativeTarget,
        '--features', 'tauri/custom-protocol', '--manifest-path', $nativeManifest, '--locked')
    & $cargo test @nativeArgs
    if ($LASTEXITCODE) { throw 'Island Lab native tests failed' }
    & $cargo build @nativeArgs
    if ($LASTEXITCODE) { throw 'Island Lab build failed' }

    $island = Join-Path $nativeTarget "$triple\release\island-lab.exe"
    if (!(Test-Path -LiteralPath $island -PathType Leaf)) { throw 'Island Lab executable is missing' }
    Write-Host "Island Lab: $island"
} finally {
    Pop-Location
}

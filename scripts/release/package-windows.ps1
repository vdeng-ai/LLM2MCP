$ErrorActionPreference = "Stop"

$Target = if ($env:TARGET) { $env:TARGET } else { "x86_64-pc-windows-msvc" }
$Version = if ($env:VERSION) {
    $env:VERSION
} else {
    python -c "import tomllib; print(tomllib.load(open('Cargo.toml','rb'))['package']['version'])"
}
$OutDir = if ($env:OUT_DIR) { $env:OUT_DIR } else { "release-artifacts" }
$StageDir = "target/release-packages/windows"

python scripts/release/generate_icons.py
if (Test-Path $StageDir) { Remove-Item -Recurse -Force $StageDir }
New-Item -ItemType Directory -Force -Path $StageDir, $OutDir | Out-Null

cargo build --locked --release --target $Target
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo packager --release --target $Target --formats nsis `
    --out-dir $StageDir `
    --binaries-dir "target/$Target/release"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Artifact = Get-ChildItem -Path $StageDir -File -Filter "*-setup.exe" | Select-Object -First 1
if (-not $Artifact) { throw "NSIS installer was not generated" }

$Normalized = Join-Path $OutDir "LLM2MCP_${Version}_x86_64-setup.exe"
Copy-Item $Artifact.FullName $Normalized -Force
$Signature = "$($Artifact.FullName).sig"
if (Test-Path $Signature) {
    Copy-Item $Signature "$Normalized.sig" -Force
}

Write-Host "Windows package: $Normalized"

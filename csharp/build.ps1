# Сборка одного self-contained exe на этом ПК. Запускать в PowerShell от администратора.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $root
Write-Host "== dotnet =="
dotnet --version
Write-Host "== publish =="
dotnet publish src\PcBridge\PcBridge.csproj -c Release -r win-x64 -o "$root\out" --nologo
if ($LASTEXITCODE -ne 0) { throw "publish завершился с кодом $LASTEXITCODE" }
$exe = Join-Path $root 'out\pc-bridge.exe'
if (-not (Test-Path $exe)) { throw "exe не появился: $exe" }
$size = [math]::Round((Get-Item $exe).Length / 1MB, 1)
Write-Host "готово: $exe ($size МБ)"
Get-FileHash $exe -Algorithm SHA256 | Select-Object -ExpandProperty Hash

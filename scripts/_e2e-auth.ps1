$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
# 清理可能残留的 PEK_IP_* 覆盖项，确保本次验证走 Config/Ip.toml
Get-ChildItem Env:PEK_IP* -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'PEK_IP_BASE' } | ForEach-Object { Remove-Item ("Env:\" + $_.Name) -ErrorAction SilentlyContinue }
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$plat = Join-Path $root '..\Pek.RPanlServer'
$stamp = [DateTime]::Now.ToString('yyyyMMddHHmmss')
$tmp = Join-Path $env:TEMP "pek-ip-auth-e2e-$stamp"
$platBase = Join-Path $tmp 'platform'
$clientBase = Join-Path $tmp 'client'
New-Item -ItemType Directory -Force -Path (Join-Path $platBase 'Config'), (Join-Path $clientBase 'Config'), (Join-Path $tmp 'pkg') | Out-Null

Write-Host "== build platform =="
Push-Location $plat
$old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
cargo build *> (Join-Path $tmp 'plat-build.log')
$code = $LASTEXITCODE
$ErrorActionPreference = $old
Pop-Location
if ($code -ne 0) { Get-Content (Join-Path $tmp 'plat-build.log') -Tail 60; throw 'platform build failed' }

Write-Host "== build client example =="
Push-Location $root
$old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
cargo build --features manager --example auth *> (Join-Path $tmp 'client-build.log')
$code = $LASTEXITCODE
$ErrorActionPreference = $old
Pop-Location
if ($code -ne 0) { Get-Content (Join-Path $tmp 'client-build.log') -Tail 60; throw 'client build failed' }

# fake ip2region xdb package
$data = Join-Path $tmp 'pkg\ip2region_v4.xdb'
[IO.File]::WriteAllText($data, 'fake-xdb-for-e2e')
$zip = Join-Path $tmp 'ip2region-v4.zip'
[IO.Compression.ZipFile]::CreateFromDirectory((Join-Path $tmp 'pkg'), $zip, [IO.Compression.CompressionLevel]::Optimal, $false)

$platExe = Join-Path $plat 'target\debug\pek-rpanlserver.exe'
$clientExe = Join-Path $root 'target\debug\examples\auth.exe'

$platCfg = @"
LocalPort = 5599
LocalOnly = true
DataSource = "Data Source=Data/panl.db;Provider=SQLite;ShowSql=false"
AdminUser = "admin"
AdminPassword = "admin"
SignKeyFile = "Config/plugin-store.key"
StoreBaseUrl = ""
ComponentStoreUrl = ""
ComponentStorePubKey = ""
ComponentStoreCaFile = ""
AgentToken = ""
AgentAllowGlobalToken = true
AgentAutoIssueToken = true
AgentAutoPushStore = true
AgentPairingEnabled = true
AdminOpsAllow = ""
AutoApproveCidrs = "127.0.0.1/32"
IpdbTrustProxy = false
SelfExe = ""
Debug = false
"@
[IO.File]::WriteAllText((Join-Path $platBase 'Config\Platform.toml'), $platCfg, (New-Object Text.UTF8Encoding($false)))

$env:PEK_RPANL_BASE = $platBase
$p = Start-Process -FilePath $platExe -WorkingDirectory $platBase -PassThru -RedirectStandardOutput (Join-Path $tmp 'platform.out.log') -RedirectStandardError (Join-Path $tmp 'platform.err.log')
Remove-Item Env:\PEK_RPANL_BASE

try {
  $ready = $false
  for ($i = 0; $i -lt 40; $i++) {
    try { $r = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ping' -TimeoutSec 2; $ready = $true; break } catch { Start-Sleep -Milliseconds 250 }
  }
  if (-not $ready) { throw 'platform did not start' }

  $login = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/login' -Method Post -ContentType 'application/json' -Body (@{ user='admin'; password='admin' } | ConvertTo-Json)
  $token = $login.data.token
  $hdr = @{ Authorization = "Bearer $token" }

  $q = 'componentId=ip2region-v4&title=ip2region%20IPv4&category=ip-database&version=2026.10&target=any&notes=e2e'
  $up = Invoke-RestMethod -Uri "http://127.0.0.1:5599/api/componentUpload?$q" -Method Post -Headers $hdr -ContentType 'application/zip' -InFile $zip
  if ($up.code -ne 0) { throw "upload failed: $($up.message)" }
  $info = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/componentInfo' -Headers $hdr
  $pub = $info.data.pubkey
  Write-Host "component uploaded, pubkey=$pub"

  $clientCfg = @"
StoreUrl = "http://127.0.0.1:5599"
PubKey = "$pub"
DataDir = "data"
CacheDir = "Data/IpCache"
StateDir = "Data/IpClient"
AutoEnroll = true
DownloadV6 = false
CaFile = ""
"@
  [IO.File]::WriteAllText((Join-Path $clientBase 'Config\Ip.toml'), $clientCfg, (New-Object Text.UTF8Encoding($false)))

  $env:PEK_IP_BASE = $clientBase
  $out = & $clientExe 2>&1
  $code = $LASTEXITCODE
  Remove-Item Env:\PEK_IP_BASE
  $out | ForEach-Object { Write-Host $_ }
  if ($code -ne 0) { throw "client failed exit=$code" }

  $clients = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ipdbClients' -Headers $hdr
  Write-Host "clients: $($clients.data.clients | ConvertTo-Json -Compress)"
  $license = Join-Path $clientBase 'Data\IpClient\license.json'
  if (!(Test-Path $license)) { throw 'license.json missing' }
  Write-Host "license: $(Get-Content $license -Raw -Encoding UTF8)"
  Write-Host "installed: $((Get-ChildItem (Join-Path $clientBase 'Data\IpCache\ip2region-v4') -Recurse -File | Select-Object -ExpandProperty FullName) -join ', ')"
  Write-Host 'E2E OK'
} finally {
  if ($p -and !$p.HasExited) { Stop-Process -Id $p.Id -Force }
  Write-Host "tmp=$tmp"
}

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
Get-ChildItem Env:PEK_IP* -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'PEK_IP_BASE' } | ForEach-Object { Remove-Item ("Env:\" + $_.Name) -ErrorAction SilentlyContinue }
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$plat = Join-Path $root '..\Pek.RPanlServer'
$stamp = [DateTime]::Now.ToString('yyyyMMddHHmmss')
$tmp = Join-Path $env:TEMP "pek-ip-auth-manual-$stamp"
$platBase = Join-Path $tmp 'platform'
$clientBase = Join-Path $tmp 'client'
New-Item -ItemType Directory -Force -Path (Join-Path $platBase 'Config'), (Join-Path $clientBase 'Config'), (Join-Path $tmp 'pkg-ip'), (Join-Path $tmp 'pkg-driver') | Out-Null

$platExe = Join-Path $plat 'target\debug\pek-rpanlserver.exe'
$clientExe = Join-Path $root 'target\debug\examples\auth.exe'
if (!(Test-Path $platExe)) { throw "missing $platExe" }
if (!(Test-Path $clientExe)) { throw "missing $clientExe" }

# 两个包：IP 数据库（需票据）+ 数据库驱动（公开）
[IO.File]::WriteAllText((Join-Path $tmp 'pkg-ip\ip2region_v4.xdb'), 'fake-xdb-for-e2e')
$ipZip = Join-Path $tmp 'ip2region-v4.zip'
[IO.Compression.ZipFile]::CreateFromDirectory((Join-Path $tmp 'pkg-ip'), $ipZip, [IO.Compression.CompressionLevel]::Optimal, $false)
[IO.File]::WriteAllText((Join-Path $tmp 'pkg-driver\dbserver.exe'), 'fake-driver')
$drvZip = Join-Path $tmp 'driver-demo.zip'
[IO.Compression.ZipFile]::CreateFromDirectory((Join-Path $tmp 'pkg-driver'), $drvZip, [IO.Compression.CompressionLevel]::Optimal, $false)

$platCfg = @"
LocalPort = 5599
LocalOnly = false
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
AutoApproveCidrs = ""
IpdbTrustProxy = false
SelfExe = ""
Debug = false
"@
[IO.File]::WriteAllText((Join-Path $platBase 'Config\Platform.toml'), $platCfg, (New-Object Text.UTF8Encoding($false)))

$env:PEK_RPANL_BASE = $platBase
$p = Start-Process -FilePath $platExe -WorkingDirectory $platBase -PassThru -RedirectStandardOutput (Join-Path $tmp 'platform.out.log') -RedirectStandardError (Join-Path $tmp 'platform.err.log')
Remove-Item Env:\PEK_RPANL_BASE

function Wait-Ping {
  for ($i = 0; $i -lt 40; $i++) {
    try { Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ping' -TimeoutSec 2 | Out-Null; return } catch { Start-Sleep -Milliseconds 250 }
  }
  throw 'platform did not start'
}
function Invoke-Auth {
  $env:PEK_IP_BASE = $clientBase
  $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  $out = & $clientExe 2>&1
  $code = $LASTEXITCODE
  $ErrorActionPreference = $old
  Remove-Item Env:\PEK_IP_BASE
  return @{ code = $code; text = ($out | Out-String) }
}

try {
  Wait-Ping
  $login = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/login' -Method Post -ContentType 'application/json' -Body (@{ user='admin'; password='admin' } | ConvertTo-Json)
  $token = $login.data.token
  $hdr = @{ Authorization = "Bearer $token" }

  $q = 'componentId=ip2region-v4&title=ip2region%20IPv4&category=ip-database&version=2026.10&target=any&notes=e2e'
  $up = Invoke-RestMethod -Uri "http://127.0.0.1:5599/api/componentUpload?$q" -Method Post -Headers $hdr -ContentType 'application/zip' -InFile $ipZip
  if ($up.code -ne 0) { throw "ip upload failed: $($up.message)" }
  $q2 = 'componentId=driver-demo&title=Driver%20demo&category=database-driver&version=1.0.0&target=any&notes=e2e'
  $up2 = Invoke-RestMethod -Uri "http://127.0.0.1:5599/api/componentUpload?$q2" -Method Post -Headers $hdr -ContentType 'application/zip' -InFile $drvZip
  if ($up2.code -ne 0) { throw "driver upload failed: $($up2.message)" }
  $info = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/componentInfo' -Headers $hdr
  $pub = $info.data.pubkey

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

  Write-Host '== phase 1: pending =='
  $r1 = Invoke-Auth
  Write-Host "exit=$($r1.code)"
  Write-Host $r1.text
  if ($r1.code -eq 0 -or $r1.text -notmatch '等待平台管理员批准') { throw 'expected pending rejection' }

  $clients = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ipdbClients' -Headers $hdr
  $pending = @($clients.data.clients | Where-Object { $_.status -eq 'pending' })
  if ($pending.Count -ne 1) { throw "expected 1 pending client, got $($pending.Count)" }
  $id = $pending[0].id

  Write-Host '== phase 2: approve with quota=1 =='
  $ap = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ipdbApprove' -Method Post -Headers $hdr -ContentType 'application/json' -Body (@{ id = $id; dailyQuota = 1 } | ConvertTo-Json)
  if ($ap.code -ne 0) { throw "approve failed: $($ap.message)" }
  $r2 = Invoke-Auth
  Write-Host "exit=$($r2.code)"
  Write-Host $r2.text
  if ($r2.code -ne 0) { throw 'approved download failed' }

  Write-Host '== phase 3: quota exhausted =='
  Remove-Item (Join-Path $clientBase 'Data\IpCache\ip2region-v4') -Recurse -Force
  $r3 = Invoke-Auth
  Write-Host "exit=$($r3.code)"
  Write-Host $r3.text
  if ($r3.code -eq 0 -or $r3.text -notmatch '配额') { throw 'expected quota rejection' }

  Write-Host '== phase 4: revoked =='
  $rv = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ipdbRevoke' -Method Post -Headers $hdr -ContentType 'application/json' -Body (@{ id = $id } | ConvertTo-Json)
  if ($rv.code -ne 0) { throw "revoke failed: $($rv.message)" }
  Remove-Item (Join-Path $clientBase 'Data\IpCache\ip2region-v4') -Recurse -Force -ErrorAction SilentlyContinue
  $r4 = Invoke-Auth
  Write-Host "exit=$($r4.code)"
  Write-Host $r4.text
  if ($r4.code -eq 0 -or ($r4.text -notmatch '吊销' -and $r4.text -notmatch '未批准')) { throw 'expected revoked rejection' }

  Write-Host '== phase 5: package access policy =='
  try {
    Invoke-WebRequest -Uri 'http://127.0.0.1:5599/components/ip2region-v4-2026.10-any.zip' -UseBasicParsing | Out-Null
    throw 'IP package should require ticket'
  } catch {
    $status = $_.Exception.Response.StatusCode.value__
    Write-Host "IP package without ticket -> HTTP $status"
    if ($status -ne 403) { throw "expected 403, got $status" }
  }
  $drv = Invoke-WebRequest -Uri 'http://127.0.0.1:5599/components/driver-demo-1.0.0-any.zip' -UseBasicParsing
  Write-Host "driver package without ticket -> HTTP $($drv.StatusCode), $($drv.RawContentLength) bytes"
  if ($drv.StatusCode -ne 200) { throw 'driver package should be public' }

  $pk = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ipdb/pubkey'
  Write-Host "pubkey endpoint -> $($pk.code) $($pk.data.pubkey)"
  Write-Host 'MANUAL E2E OK'
} finally {
  if ($p -and !$p.HasExited) { Stop-Process -Id $p.Id -Force }
  Write-Host "tmp=$tmp"
}

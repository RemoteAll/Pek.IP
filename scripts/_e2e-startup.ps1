$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
Get-ChildItem Env:PEK_IP* -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'PEK_IP_BASE' } | ForEach-Object { Remove-Item ("Env:\" + $_.Name) -ErrorAction SilentlyContinue }
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$plat = Join-Path $root '..\Pek.RPanlServer'
$stamp = [DateTime]::Now.ToString('yyyyMMddHHmmss')
$tmp = Join-Path $env:TEMP "pek-ip-startup-e2e-$stamp"
$platBase = Join-Path $tmp 'platform'
$clientBase = Join-Path $tmp 'client'
New-Item -ItemType Directory -Force -Path (Join-Path $platBase 'Config'), (Join-Path $clientBase 'Config') | Out-Null

$platExe = Join-Path $plat 'target\debug\pek-rpanlserver.exe'
if (!(Test-Path $platExe)) { throw "missing $platExe" }

function New-Zip($src, $name, $out) {
  $dir = Join-Path $tmp ("z-" + [IO.Path]::GetFileNameWithoutExtension($out))
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  Copy-Item $src (Join-Path $dir $name) -Force
  [IO.Compression.ZipFile]::CreateFromDirectory($dir, $out, [IO.Compression.CompressionLevel]::Optimal, $false)
}

$zipV4 = Join-Path $tmp 'ip2region-v4.zip'
New-Zip (Join-Path $root 'data\ip2region_v4.xdb') 'ip2region_v4.xdb' $zipV4
$zipCity = Join-Path $tmp 'geolite2-city.zip'
New-Zip (Join-Path $root 'data\dbip-country-lite-2026-10.mmdb') 'GeoLite2-City.mmdb' $zipCity
$zipAsn = Join-Path $tmp 'geolite2-asn.zip'
New-Zip (Join-Path $root 'data\dbip-asn-lite-2026-10.mmdb') 'GeoLite2-ASN.mmdb' $zipAsn

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
    try { Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/ping' -TimeoutSec 2 | Out-Null; $ready = $true; break } catch { Start-Sleep -Milliseconds 250 }
  }
  if (-not $ready) { throw 'platform did not start' }
  $login = Invoke-RestMethod -Uri 'http://127.0.0.1:5599/api/login' -Method Post -ContentType 'application/json' -Body (@{ user='admin'; password='admin' } | ConvertTo-Json)
  $hdr = @{ Authorization = "Bearer $($login.data.token)" }
  $uploads = @(
    @{ id='ip2region-v4'; file=$zipV4 },
    @{ id='geolite2-city'; file=$zipCity },
    @{ id='geolite2-asn'; file=$zipAsn }
  )
  foreach ($u in $uploads) {
    $q = "componentId=$($u.id)&title=$($u.id)&category=ip-database&version=2026.10&target=any&notes=e2e"
    $r = Invoke-RestMethod -Uri "http://127.0.0.1:5599/api/componentUpload?$q" -Method Post -Headers $hdr -ContentType 'application/zip' -InFile $u.file
    if ($r.code -ne 0) { throw "upload $($u.id) failed: $($r.message)" }
    Write-Host "uploaded $($u.id)"
  }
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

  Push-Location $root
  $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  cargo build --features manager --example startup *> (Join-Path $tmp 'build.log')
  $code = $LASTEXITCODE
  $ErrorActionPreference = $old
  Pop-Location
  if ($code -ne 0) { Get-Content (Join-Path $tmp 'build.log') -Tail 40; throw 'startup build failed' }

  $env:PEK_IP_BASE = $clientBase
  $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  $out = & (Join-Path $root 'target\debug\examples\startup.exe') '114.114.114.114' '8.8.8.8' 2>&1
  $code = $LASTEXITCODE
  $ErrorActionPreference = $old
  Remove-Item Env:\PEK_IP_BASE
  $out | ForEach-Object { Write-Host $_ }
  if ($code -ne 0) { throw "startup failed exit=$code" }
  $text = $out | Out-String
  if ($text -notmatch '国内数据：已加载' -or $text -notmatch '国外数据：已加载') { throw 'expected both china/world data loaded' }
  if ($text -notmatch '8\.8\.8\.8 =>') { throw 'expected lookup output for 8.8.8.8' }
  Write-Host 'STARTUP E2E OK'
} finally {
  if ($p -and !$p.HasExited) { Stop-Process -Id $p.Id -Force }
  Write-Host "tmp=$tmp"
}

#requires -Version 5.1
<#
.SYNOPSIS
    将 xdb / mmdb 数据文件打包为 Pek.RPanlServer 组件包。

.DESCRIPTION
    输出 dist/{id}-{version}-any.zip，并生成 dataset.json 说明文件。
    上传平台时：分类填 ip-database，目标平台填 any。

.EXAMPLE
    .\scripts\pack-ipdb.ps1 -File .\data\ip2region_v4.xdb

.EXAMPLE
    .\scripts\pack-ipdb.ps1 -File .\data\GeoLite2-City.mmdb -Id geolite2-city -Version 2026.10 -Notes "GeoLite2 City 2026-10"
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$File,

    [string]$Id = '',
    [string]$Version = '',
    [string]$Title = '',
    [string]$Notes = '',
    [string]$OutDir = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Fail([string]$message) {
    Write-Error $message
    exit 1
}

function Infer-Id([string]$name) {
    $lower = $name.ToLowerInvariant()
    if ($lower -match 'ip2region.*v6') { return 'ip2region-v6' }
    if ($lower -match 'ip2region.*v4') { return 'ip2region-v4' }
    if ($lower -match 'ip2region') { return 'ip2region-v4' }
    if ($lower -match 'geolite2.*city') { return 'geolite2-city' }
    if ($lower -match 'geolite2.*asn') { return 'geolite2-asn' }
    if ($lower -match 'geolite2.*country') { return 'geolite2-country' }
    if ($lower -match 'dbip.*city') { return 'dbip-city' }
    if ($lower -match 'dbip.*asn') { return 'dbip-asn' }
    return ''
}

function Infer-Format([string]$name) {
    if ($name.ToLowerInvariant().EndsWith('.xdb')) { return 'xdb' }
    if ($name.ToLowerInvariant().EndsWith('.mmdb')) { return 'mmdb' }
    return 'unknown'
}

function Infer-IpVersion([string]$name, [string]$format) {
    $lower = $name.ToLowerInvariant()
    if ($format -eq 'xdb') {
        if ($lower -match 'v6') { return 'v6' }
        if ($lower -match 'v4') { return 'v4' }
        return 'v4'
    }
    return 'v4-v6'
}

function Infer-Kind([string]$name) {
    $lower = $name.ToLowerInvariant()
    if ($lower -match 'asn') { return 'asn' }
    if ($lower -match 'isp') { return 'isp' }
    if ($lower -match 'country') { return 'country' }
    if ($lower -match 'city') { return 'city' }
    if ($lower -match 'ip2region') { return 'city' }
    return 'unknown'
}

function Infer-Source([string]$name) {
    $lower = $name.ToLowerInvariant()
    if ($lower -match 'ip2region') { return 'ip2region' }
    if ($lower -match 'geolite2' -or $lower -match 'geoip2') { return 'maxmind' }
    if ($lower -match 'dbip') { return 'dbip' }
    return 'custom'
}

function Infer-License([string]$source) {
    switch ($source) {
        'ip2region' { return 'Apache-2.0' }
        'maxmind' { return 'GeoLite2 EULA / GeoIP2 EULA' }
        'dbip' { return 'CC BY 4.0' }
        default { return 'unknown' }
    }
}

if (-not (Test-Path -LiteralPath $File -PathType Leaf)) {
    Fail "文件不存在：$File"
}
$sourceFile = (Resolve-Path -LiteralPath $File).Path
$fileName = [IO.Path]::GetFileName($sourceFile)
$format = Infer-Format $fileName
if ($format -eq 'unknown') {
    Fail "仅支持 .xdb / .mmdb 文件：$fileName"
}

if ([string]::IsNullOrWhiteSpace($Id)) {
    $Id = Infer-Id $fileName
}
if ([string]::IsNullOrWhiteSpace($Id)) {
    Fail "无法从文件名推断组件 id，请用 -Id 指定（如 ip2region-v4 / geolite2-city）"
}
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = Get-Date -Format 'yyyy.MM.dd'
}
if ($Id -notmatch '^[A-Za-z0-9._-]{1,64}$') {
    Fail "组件 id 不合法（仅 [A-Za-z0-9._-]，≤64）：$Id"
}
if ($Version -notmatch '^[A-Za-z0-9._+-]{1,50}$') {
    Fail "版本号不合法（仅 [A-Za-z0-9._+-]，≤50）：$Version"
}

$source = Infer-Source $fileName
$kind = Infer-Kind $fileName
$ipVersion = Infer-IpVersion $fileName $format
$license = Infer-License $source
if ([string]::IsNullOrWhiteSpace($Title)) {
    $Title = $Id
}
if ([string]::IsNullOrWhiteSpace($Notes)) {
    $Notes = "IP 数据库（$source / $kind / $ipVersion），$license"
}

if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'dist'
}
$OutDir = [IO.Path]::GetFullPath($OutDir)
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$stage = Join-Path $env:TEMP ("pek-ip-pack-{0}-{1}-{2}" -f $Id, $Version, $PID)
if (Test-Path -LiteralPath $stage) {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stage | Out-Null

try {
    Copy-Item -LiteralPath $sourceFile -Destination (Join-Path $stage $fileName) -Force
    $meta = [ordered]@{
        schema    = 1
        id        = $Id
        title     = $Title
        version   = $Version
        format    = $format
        ipVersion = $ipVersion
        kind      = $kind
        source    = $source
        license   = $license
        notes     = $Notes
        packedAt  = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
    }
    $metaJson = $meta | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText(
        (Join-Path $stage 'dataset.json'),
        $metaJson,
        (New-Object Text.UTF8Encoding($false))
    )

    $finalZip = Join-Path $OutDir ("{0}-{1}-any.zip" -f $Id, $Version)
    $newZip = "$finalZip.new"
    if (Test-Path -LiteralPath $newZip) {
        Remove-Item -LiteralPath $newZip -Force
    }
    [IO.Compression.ZipFile]::CreateFromDirectory(
        $stage,
        $newZip,
        [IO.Compression.CompressionLevel]::Optimal,
        $false
    )
    Move-Item -LiteralPath $newZip -Destination $finalZip -Force

    $item = Get-Item -LiteralPath $finalZip
    $hash = (Get-FileHash -LiteralPath $finalZip -Algorithm SHA256).Hash.ToLowerInvariant()
    $sizeMb = [Math]::Round($item.Length / 1MB, 2)

    Write-Host ''
    Write-Host '================ Pek.IP 组件包 ================'
    Write-Host ("组件 ID   : {0}" -f $Id)
    Write-Host ("版本      : {0}" -f $Version)
    Write-Host ("目标平台  : any")
    Write-Host ("分类      : ip-database")
    Write-Host ("标题      : {0}" -f $Title)
    Write-Host ("说明      : {0}" -f $Notes)
    Write-Host ("包文件    : {0}" -f $finalZip)
    Write-Host ("大小      : {0} MB" -f $sizeMb)
    Write-Host ("SHA-256   : {0}" -f $hash)
    Write-Host ''
    Write-Host '上传：Pek.RPanlServer → 下载管理/组件 → 上传组件，按上面的 id/版本/目标/分类填写。'
    Write-Host ("客户端：IpDbManager::ensure_updated(`"{0}`")" -f $Id)
    Write-Host '================================================'
}
finally {
    if (Test-Path -LiteralPath $stage) {
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
}

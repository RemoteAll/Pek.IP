#requires -Version 5.1
<#
.SYNOPSIS
    下载 Pek.IP 所需的 IP 数据文件到 data/。

.DESCRIPTION
    默认下载 ip2region v4/v6（Apache-2.0）与 DB-IP Country/ASN Lite（CC BY 4.0）。
    城市级 DB-IP City Lite 约 60MB 压缩 / 127MB 解压，需显式 -City。
    GeoLite2 需 MaxMind 账号，脚本只打印获取方式。

.EXAMPLE
    .\scripts\fetch-data.ps1

.EXAMPLE
    .\scripts\fetch-data.ps1 -City -Force
#>
[CmdletBinding()]
param(
    [switch]$City,
    [switch]$Force,
    [string]$DbIpMonth = (Get-Date -Format 'yyyy-MM')
)

$ErrorActionPreference = 'Stop'

function Download-File([string]$url, [string]$dest) {
    if ((Test-Path -LiteralPath $dest) -and -not $Force) {
        Write-Host "[跳过] 已存在：$dest"
        return
    }
    $part = "$dest.part"
    if (Test-Path -LiteralPath $part) {
        Remove-Item -LiteralPath $part -Force
    }
    Write-Host "[下载] $url"
    Invoke-WebRequest -Uri $url -OutFile $part -UseBasicParsing
    Move-Item -LiteralPath $part -Destination $dest -Force
    $len = (Get-Item -LiteralPath $dest).Length
    Write-Host ("[完成] {0}（{1:N1} MiB）" -f $dest, ($len / 1MB))
}

function Expand-Gzip([string]$gz, [string]$out) {
    if ((Test-Path -LiteralPath $out) -and -not $Force) {
        Write-Host "[跳过] 已存在：$out"
        return
    }
    Write-Host "[解压] $gz -> $out"
    $part = "$out.part"
    if (Test-Path -LiteralPath $part) {
        Remove-Item -LiteralPath $part -Force
    }
    $input = [IO.File]::OpenRead($gz)
    try {
        $gzip = New-Object IO.Compression.GZipStream($input, [IO.Compression.CompressionMode]::Decompress)
        try {
            $output = [IO.File]::Create($part)
            try { $gzip.CopyTo($output) } finally { $output.Close() }
        } finally { $gzip.Close() }
    } finally {
        $input.Close()
    }
    Move-Item -LiteralPath $part -Destination $out -Force
    $len = (Get-Item -LiteralPath $out).Length
    Write-Host ("[完成] {0}（{1:N1} MiB）" -f $out, ($len / 1MB))
}

$root = Split-Path -Parent $PSScriptRoot
$data = Join-Path $root 'data'
New-Item -ItemType Directory -Force -Path $data | Out-Null

# ip2region xdb v3（Apache-2.0）
Download-File 'https://raw.githubusercontent.com/lionsoul2014/ip2region/master/data/ip2region_v4.xdb' (Join-Path $data 'ip2region_v4.xdb')
Download-File 'https://raw.githubusercontent.com/lionsoul2014/ip2region/master/data/ip2region_v6.xdb' (Join-Path $data 'ip2region_v6.xdb')

# DB-IP Lite（CC BY 4.0；默认国家 + ASN，城市需 -City）
$dbip = @(
    @{ Name = 'dbip-country-lite'; Url = "https://download.db-ip.com/free/dbip-country-lite-$DbIpMonth.mmdb.gz" },
    @{ Name = 'dbip-asn-lite';     Url = "https://download.db-ip.com/free/dbip-asn-lite-$DbIpMonth.mmdb.gz" }
)
if ($City) {
    $dbip += @{ Name = 'dbip-city-lite'; Url = "https://download.db-ip.com/free/dbip-city-lite-$DbIpMonth.mmdb.gz" }
}
foreach ($item in $dbip) {
    $gz = Join-Path $data ("{0}-{1}.mmdb.gz" -f $item.Name, $DbIpMonth)
    $mmdb = Join-Path $data ("{0}-{1}.mmdb" -f $item.Name, $DbIpMonth)
    Download-File $item.Url $gz
    Expand-Gzip $gz $mmdb
    if ((Test-Path -LiteralPath $mmdb) -and (Test-Path -LiteralPath $gz)) {
        Remove-Item -LiteralPath $gz -Force
    }
}

Write-Host ''
Write-Host 'GeoLite2（可选，需 MaxMind 账号）：'
Write-Host '  https://www.maxmind.com/en/geolite2/signup'
Write-Host '  下载 GeoLite2-City.mmdb / GeoLite2-ASN.mmdb 后放入 data/ 即可。'
Write-Host ''
Write-Host '完成。上传 RPanel：'
Write-Host '  .\scripts\pack-ipdb.ps1 -File .\data\ip2region_v4.xdb -Version 2026.10'
Write-Host '  .\scripts\pack-ipdb.ps1 -File .\data\dbip-city-lite-2026-10.mmdb -Id geolite2-city -Version 2026.10'

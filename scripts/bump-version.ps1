# 同步版本号：Cargo.toml、assets/wgopt.rc、README 里的版本引用
#
# 用 PowerShell 7 运行（5.1 按 GBK 解析，中文会坏）。
# 写文件时一律 utf-8 无 BOM，避免 Tauri/Node/JSON 解析报错。

param(
    [Parameter(Mandatory = $true)]
    [string]$Version
)

$ErrorActionPreference = "Stop"

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    Write-Error "版本号格式错误，应为 主.次.修订，例如 1.0.1"
    exit 1
}

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$utf8 = New-Object System.Text.UTF8Encoding($false)

function Update-File {
    param([string]$Path, [scriptblock]$Transform)

    if (-not (Test-Path $Path)) {
        Write-Error "找不到文件：$Path"
        exit 1
    }

    $text = [System.IO.File]::ReadAllText($Path, $utf8)
    $new = & $Transform $text
    if ($new -eq $text) {
        Write-Output "  未变化：$Path"
        return
    }
    [System.IO.File]::WriteAllText($Path, $new, $utf8)
    Write-Output "  已更新：$Path"
}

# Cargo.toml: package 段的 version
Update-File "$root\Cargo.toml" {
    param($t)
    $t -replace '(?m)^(version\s*=\s*")[^"]*(")', "`${1}$Version`${2}"
}

# 资源文件里的四段式版本号
$parts = $Version.Split('.')
$quad = "$($parts[0]),$($parts[1]),$($parts[2]),0"
$quadDotted = "$($parts[0]).$($parts[1]).$($parts[2]).0"

Update-File "$root\assets\wgopt.rc" {
    param($t)
    $t = $t -replace '(?m)^FILEVERSION\s+.*$', "FILEVERSION     $quad"
    $t = $t -replace '(?m)^PRODUCTVERSION\s+.*$', "PRODUCTVERSION  $quad"
    $t = $t -replace '(?m)("FileVersion",\s+")[^"]*(")', "`${1}$quadDotted`${2}"
    $t -replace '(?m)("ProductVersion",\s+")[^"]*(")', "`${1}$quadDotted`${2}"
}

Write-Output ""
Write-Output "版本号已同步为 v$Version"
Write-Output "注意：assets\wgopt.rc 的版本改动需要重新编译才生效。"

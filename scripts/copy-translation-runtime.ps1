# Packages the local CPU translation runtime, never downloaded language models.
# Shared by the ZIP/Inno and MSIX paths so releases cannot omit this feature.
[CmdletBinding()]
param(
    [string]$SourceDirectory = 'dist/translation/windows-x64',
    [Parameter(Mandatory = $true)][string]$DestinationDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Translation runtime packaging requires Windows.' }
$repo = Split-Path $PSScriptRoot -Parent
$source = if ([System.IO.Path]::IsPathRooted($SourceDirectory)) {
    [System.IO.Path]::GetFullPath($SourceDirectory)
} else { [System.IO.Path]::GetFullPath((Join-Path $repo $SourceDirectory)) }
$destinationRoot = [System.IO.Path]::GetFullPath($DestinationDirectory)
$destination = Join-Path $destinationRoot 'translation'
if ($destination.StartsWith($source.TrimEnd([char[]]@('/', '\')) + [System.IO.Path]::DirectorySeparatorChar,
        [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Translation runtime destination must not be inside its source directory.'
}
if (-not (Test-Path $source -PathType Container)) {
    throw 'Offline translation runtime missing. Run scripts/build-translation-windows.ps1 before packaging.'
}
$required = @('mypdf_ct2.dll', 'mypdf_ct2.dll.sha256', 'licenses/CTranslate2.txt',
    'licenses/SentencePiece.txt', 'licenses/Argos-Translate.txt', 'licenses/languages.json')
foreach ($relative in $required) {
    $file = Join-Path $source $relative
    if (-not (Test-Path $file -PathType Leaf) -or (Get-Item $file).Length -eq 0) {
        throw "Offline translation runtime or license missing: $relative"
    }
}
foreach ($directory in @('CTranslate2-dependencies', 'SentencePiece-dependencies')) {
    $notices = Join-Path $source "licenses/$directory"
    if (-not (Test-Path $notices -PathType Container) -or
        @(Get-ChildItem $notices -File -Recurse).Count -eq 0) {
        throw "Offline translation dependency notices missing: $directory"
    }
}
foreach ($item in @(Get-ChildItem $source -Force)) {
    if ($item.Name -notin @('mypdf_ct2.dll', 'mypdf_ct2.dll.sha256', 'licenses')) {
        throw "Unexpected translation payload (language models must not be bundled): $($item.Name)"
    }
}
if (@(Get-ChildItem $source -Recurse -Force | Where-Object {
    $_.Attributes -band [System.IO.FileAttributes]::ReparsePoint
}).Count -ne 0) { throw 'Translation runtime payload must not contain filesystem links.' }
foreach ($file in @(Get-ChildItem (Join-Path $source 'licenses') -Recurse -File)) {
    if ($file.Name -notin @('CTranslate2.txt', 'SentencePiece.txt', 'Argos-Translate.txt', 'languages.json') -and
        $file.Name -notmatch '^(?i:LICENSE|COPYING|COPYRIGHT|NOTICE)') {
        throw "Unexpected translation license payload (language models must not be bundled): $($file.Name)"
    }
}
$checksum = (Get-Content (Join-Path $source 'mypdf_ct2.dll.sha256') -Raw).Trim()
if ($checksum -notmatch '^(?<digest>[a-fA-F0-9]{64})\s+\*?mypdf_ct2\.dll$') {
    throw 'Invalid translation runtime checksum file.'
}
$expected = $Matches.digest
if ((Get-FileHash (Join-Path $source 'mypdf_ct2.dll') -Algorithm SHA256).Hash -ine $expected) {
    throw 'Translation runtime checksum does not match the built DLL.'
}
# This checks the x64 PE header and loads the real exported ABI; merely finding
# a file would not detect a missing native dependency or the wrong architecture.
& (Join-Path $PSScriptRoot 'test-translation-runtime.ps1') -RuntimeDirectory $source
if (Test-Path $destination) { throw "Translation destination already exists: $destination" }
$null = New-Item -ItemType Directory -Path $destinationRoot -Force
try {
    Copy-Item $source $destination -Recurse
    foreach ($file in @(Get-ChildItem $source -Recurse -File)) {
        $relative = $file.FullName.Substring($source.Length).TrimStart([char[]]@('/', '\'))
        $copied = Join-Path $destination $relative
        if (-not (Test-Path $copied -PathType Leaf) -or
            (Get-FileHash $file.FullName -Algorithm SHA256).Hash -ne (Get-FileHash $copied -Algorithm SHA256).Hash) {
            throw "Translation runtime copy verification failed: $relative"
        }
    }
} catch {
    Remove-Item $destination -Recurse -Force -ErrorAction SilentlyContinue
    throw
}
Write-Host 'Packaged offline CPU translation runtime and dependency notices; language models remain user downloads.'

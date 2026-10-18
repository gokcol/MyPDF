# Exercises the real built runtime and failure paths on a disposable runner.
# No models are downloaded and no PDF content is used.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
$source = Join-Path $repo 'dist/translation/windows-x64'
$copy = Join-Path $PSScriptRoot 'copy-translation-runtime.ps1'
$scratch = Join-Path ([System.IO.Path]::GetTempPath()) ('mypdf-runtime-qa-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $scratch
try {
    $runtime = Join-Path $scratch 'source'
    Copy-Item $source $runtime -Recurse
    $destination = Join-Path $scratch 'package'
    function Expect-Rejection([string]$Pattern) {
        $rejected = $false
        try { & $copy -SourceDirectory $runtime -DestinationDirectory $destination } catch {
            if ($_.Exception.Message -notmatch $Pattern) { throw }
            $rejected = $true
        }
        if (-not $rejected) { throw "Expected runtime packaging rejection: $Pattern" }
        Write-Host "PASS runtime packaging rejection: $Pattern"
    }
    $dll = Join-Path $runtime 'mypdf_ct2.dll'
    Move-Item $dll "$dll.hidden"
    Expect-Rejection 'runtime or license missing: mypdf_ct2.dll'
    Move-Item "$dll.hidden" $dll
    $license = Join-Path $runtime 'licenses/CTranslate2.txt'
    Move-Item $license "$license.hidden"
    Expect-Rejection 'runtime or license missing: licenses/CTranslate2.txt'
    Move-Item "$license.hidden" $license
    $checksum = Get-Content "$dll.sha256" -Raw
    (('0' * 64) + '  mypdf_ct2.dll') | Set-Content "$dll.sha256" -Encoding ascii
    Expect-Rejection 'checksum does not match'
    $checksum | Set-Content "$dll.sha256" -Encoding ascii
    $model = Join-Path $runtime 'model.bin'
    Set-Content $model 'synthetic model sentinel' -Encoding ascii
    Expect-Rejection 'language models must not be bundled'
    Remove-Item $model
    & $copy -SourceDirectory $runtime -DestinationDirectory $destination
    if ((Get-FileHash $dll).Hash -ne (Get-FileHash (Join-Path $destination 'translation/mypdf_ct2.dll')).Hash) {
        throw 'Packaged native runtime changed.'
    }
    Expect-Rejection 'destination already exists'
    Write-Host 'PASS native runtime packaging, exact copies, mandatory notices, checksums, and model exclusion.'
} finally { Remove-Item $scratch -Recurse -Force }

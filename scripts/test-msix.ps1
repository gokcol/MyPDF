# Run on Windows after building mypdf.exe and msix_assets.exe. Checks input
# boundaries and XML escaping with the real SDK, without publishing anything.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$packager = Join-Path $PSScriptRoot 'package-msix.ps1'
function Expect-Rejection([hashtable]$Arguments, [string]$Pattern) {
    $Arguments.SkipBuild = $true
    $rejected = $false
    try { & $packager @Arguments } catch {
        if ($_.Exception.Message -notmatch $Pattern) { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw "Expected rejection matching: $Pattern" }
    Write-Host "PASS rejection: $Pattern"
}
Expect-Rejection @{} 'requires IdentityName'
Expect-Rejection @{ TestIdentity = $true; IdentityName = 'Example.MyPDF' } 'Do not combine'
Expect-Rejection @{ IdentityName = 'MyPDF.Placeholder'; Publisher = 'CN=Example'; PublisherDisplayName = 'Example' } 'placeholder'
Expect-Rejection @{ IdentityName = 'Invalid/Name'; Publisher = 'CN=Example'; PublisherDisplayName = 'Example' } 'Invalid package identity'
foreach ($version in @('1.0.0.1', '0.1.0.0', '65536.0.0.0', '1.0.0-beta.0')) {
    Expect-Rejection @{ TestIdentity = $true; PackageVersion = $version } 'four-part Store version'
}
foreach ($version in @('1.2.0.0', '1.11.1.0')) {
    Expect-Rejection @{ TestIdentity = $true; PackageVersion = $version } 'preserve Store upgrade ordering'
}
# Exercise XML-special characters as data, not manifest markup. This QA
# identity lives only in a disposable folder, never in published artifacts.
$scratch = Join-Path ([System.IO.Path]::GetTempPath()) ('mypdf-msix-qa-' + [guid]::NewGuid().ToString('N'))
try {
    & $packager -SkipBuild -IdentityName 'MyPDF.XmlEscapingQA' -Publisher 'CN=MyPDF QA, O=QA & Co' `
        -PublisherDisplayName 'QA Gökçöl & Co' -DisplayName 'MyPDF (QA) <Test>' -OutputDirectory $scratch
    $package = Get-ChildItem $scratch -Filter '*.msix' | Select-Object -First 1
    if (-not $package) { throw 'XML escaping validation did not produce a package.' }
    $info = Get-Content ($package.FullName + '.json') -Raw | ConvertFrom-Json
    if ($info.displayName -cne 'MyPDF (QA) <Test>' -or $info.publisherDisplayName -cne 'QA Gökçöl & Co') { throw 'Display-name escaping did not round-trip.' }
    $expectedVersion = (Get-Content (Join-Path $PSScriptRoot '../packaging/windows/msix/version.txt') -Raw).Trim()
    $expectedAppVersion = (Select-String -Path (Join-Path $PSScriptRoot '../Cargo.toml') -Pattern '^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Matches[0].Groups[1].Value
    if ($info.version -cne $expectedVersion -or $info.appVersion -cne $expectedAppVersion) { throw 'Public app version or Store package version does not match its source.' }
    Expect-Rejection @{ IdentityName = 'MyPDF.XmlEscapingQA'; Publisher = 'CN=MyPDF QA, O=QA & Co'; PublisherDisplayName = 'QA & Co'; OutputDirectory = $scratch } 'Output already exists'
    Write-Host 'PASS XML identity escaping, validated package round trip and safe output handling.'
} finally {
    if (Test-Path $scratch) { Remove-Item $scratch -Recurse -Force }
}

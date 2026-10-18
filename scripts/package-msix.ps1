# Builds an unsigned Microsoft Store submission package. A Store upload must
# use the exact public identity from Partner Center, never the test identity.
[CmdletBinding()]
param(
    [switch]$TestIdentity,
    [string]$IdentityName,
    [string]$Publisher,
    [string]$PublisherDisplayName,
    [string]$DisplayName = 'MyPDF',
    [string]$PackageVersion,
    [string]$OutputDirectory = 'dist/msix',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
try {
    if ($TestIdentity) {
        if ($IdentityName -or $Publisher -or $PublisherDisplayName) {
            throw 'Do not combine -TestIdentity with Store identity values.'
        }
        $IdentityName = 'MyPDF.PackagingTest'
        $Publisher = 'CN=MyPDF Packaging Test'
        $PublisherDisplayName = 'MyPDF packaging tests'
        $DisplayName = 'MyPDF Packaging Test'
    } else {
        if (-not $IdentityName -or -not $Publisher -or -not $PublisherDisplayName) {
            throw 'Store packaging requires IdentityName, Publisher and PublisherDisplayName from Partner Center. Use -TestIdentity only for validation.'
        }
        if ("$IdentityName $Publisher $PublisherDisplayName" -match '(?i)placeholder|packaging[ .-]?test|replace[_ -]?me') {
            throw 'A placeholder or test identity cannot be used for a Store package.'
        }
    }
    if ($IdentityName -notmatch '^[A-Za-z0-9.-]{3,50}$') { throw 'Invalid package identity name.' }
    if (-not $Publisher.StartsWith('CN=') -or $Publisher.Length -gt 8192) { throw 'Invalid publisher distinguished name.' }
    $null = [System.Security.Cryptography.X509Certificates.X500DistinguishedName]::new($Publisher)
    foreach ($value in @($PublisherDisplayName, $DisplayName)) {
        if ([string]::IsNullOrWhiteSpace($value) -or $value.Length -gt 256) { throw 'Display names must contain 1–256 characters.' }
    }
    $crateVersion = (Select-String -Path Cargo.toml -Pattern '^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Matches[0].Groups[1].Value
    # The public version reset from 1.11.x to 1.2.0. Keep Store update ordering
    # independent so existing 1.11.1.0 packages can receive this release.
    $defaultPackageVersion = (Get-Content 'packaging/windows/msix/version.txt' -Raw).Trim()
    if (-not $PackageVersion) { $PackageVersion = $defaultPackageVersion }
    foreach ($version in @($defaultPackageVersion, $PackageVersion)) {
        if ($version -notmatch '^([1-9][0-9]*)\.([0-9]+)\.([0-9]+)\.0$' -or
            @($version.Split('.') | Where-Object { [decimal]$_ -gt 65535 }).Count -ne 0) {
            throw 'Use a four-part Store version such as 1.0.0.0; major must be nonzero, each part <= 65535, and revision must be 0.'
        }
    }
    if ([version]$PackageVersion -lt [version]$defaultPackageVersion) {
        throw "Package version must be at least $defaultPackageVersion to preserve Store upgrade ordering."
    }
    if ($env:OS -ne 'Windows_NT') { throw 'MSIX packaging requires Windows and the Windows 10/11 SDK.' }
    $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
    $sdk = Get-ChildItem $sdkRoot -Directory | Where-Object {
        $_.Name -match '^10\.0\.\d+\.\d+$' -and
        (Test-Path (Join-Path $_.FullName 'x64/makeappx.exe')) -and
        (Test-Path (Join-Path $_.FullName 'x64/makepri.exe'))
    } | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    if (-not $sdk) { throw 'Install the Windows SDK with MakeAppx and MakePri.' }
    $makeappx = Join-Path $sdk.FullName 'x64/makeappx.exe'
    $makepri = Join-Path $sdk.FullName 'x64/makepri.exe'
    if (-not $SkipBuild) {
        & (Join-Path $PSScriptRoot 'build-translation-windows.ps1')
        & cargo build --locked --release --bin mypdf --example msix_assets
        if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
    }
    $exe = Join-Path $repo 'target/release/mypdf.exe'
    $assetsTool = Join-Path $repo 'target/release/examples/msix_assets.exe'
    if (-not (Test-Path $exe) -or -not (Test-Path $assetsTool)) { throw 'Build mypdf and the msix_assets example first.' }
    $reader = [System.IO.BinaryReader]::new([System.IO.File]::OpenRead($exe))
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'Not a Windows executable.' }
        $reader.BaseStream.Position = 0x3c
        $peOffset = $reader.ReadUInt32()
        $reader.BaseStream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) { throw 'The MSIX requires an x64 Windows executable.' }
    } finally { $reader.Dispose() }
    $out = [System.IO.Path]::GetFullPath($OutputDirectory)
    $null = New-Item -ItemType Directory -Path $out -Force
    $flavor = if ($TestIdentity) { 'TEST-ONLY' } else { 'store' }
    $name = "MyPDF-$PackageVersion-$flavor-x64.msix"
    $package = Join-Path $out $name
    if (Test-Path $package) { throw "Output already exists: $package. Choose a new folder or version." }
    $work = Join-Path $out ('.work-' + [guid]::NewGuid().ToString('N'))
    $layout = Join-Path $work 'layout'
    $null = New-Item -ItemType Directory -Path $layout -Force
    try {
        Copy-Item $exe (Join-Path $layout 'MyPDF.exe')
        Copy-Item -Path @('README.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'THIRD_PARTY_NOTICES.md') -Destination $layout
        & (Join-Path $PSScriptRoot 'copy-translation-runtime.ps1') -DestinationDirectory $layout
        & $assetsTool (Join-Path $layout 'Assets')
        if ($LASTEXITCODE -ne 0) { throw 'MSIX artwork generation failed.' }
        [xml]$manifest = Get-Content packaging/windows/msix/AppxManifest.xml -Raw
        $manifest.Package.Identity.SetAttribute('Name', $IdentityName)
        $manifest.Package.Identity.SetAttribute('Publisher', $Publisher)
        $manifest.Package.Identity.SetAttribute('Version', $PackageVersion)
        $manifest.Package.Properties.DisplayName = $DisplayName
        $manifest.Package.Properties.PublisherDisplayName = $PublisherDisplayName
        $visual = $manifest.GetElementsByTagName('VisualElements', 'http://schemas.microsoft.com/appx/manifest/uap/windows10')[0]
        $visual.SetAttribute('DisplayName', $DisplayName)
        $manifest.Save((Join-Path $layout 'AppxManifest.xml'))
        $config = Join-Path $work 'priconfig.xml'
        & $makepri createconfig /cf $config /dq en-US /o
        if ($LASTEXITCODE -ne 0) { throw 'MakePri configuration failed.' }
        & $makepri new /pr $layout /cf $config /of (Join-Path $layout 'resources.pri') /o
        if ($LASTEXITCODE -ne 0) { throw 'MakePri resource indexing failed.' }
        & $makeappx pack /d $layout /p $package /h SHA256 /o
        if ($LASTEXITCODE -ne 0) { throw 'MakeAppx packaging/manifest validation failed.' }
        $unpacked = Join-Path $work 'unpacked'
        & $makeappx unpack /p $package /d $unpacked /o
        if ($LASTEXITCODE -ne 0) { throw 'MSIX round-trip validation failed.' }
        if ((Get-FileHash $exe).Hash -ne (Get-FileHash (Join-Path $unpacked 'MyPDF.exe')).Hash) {
            throw 'Packaged executable differs from the release build.'
        }
        $runtime = Join-Path $layout 'translation'
        foreach ($file in @(Get-ChildItem $runtime -Recurse -File)) {
            $relative = $file.FullName.Substring($layout.Length).TrimStart([char[]]@('/', '\'))
            $roundTripFile = Join-Path $unpacked $relative
            if (-not (Test-Path $roundTripFile -PathType Leaf) -or
                (Get-FileHash $file.FullName).Hash -ne (Get-FileHash $roundTripFile).Hash) {
                throw "Packaged translation runtime differs from the release build: $relative"
            }
        }
        [xml]$roundTrip = Get-Content (Join-Path $unpacked 'AppxManifest.xml') -Raw
        if ($roundTrip.Package.Identity.Name -ne $IdentityName -or $roundTrip.Package.Identity.Publisher -ne $Publisher -or
            $roundTrip.Package.Identity.Version -ne $PackageVersion) { throw 'Packaged identity does not match the requested identity.' }
        $roundTripVisual = $roundTrip.GetElementsByTagName('VisualElements', 'http://schemas.microsoft.com/appx/manifest/uap/windows10')[0]
        if ($roundTrip.Package.Properties.DisplayName -cne $DisplayName -or
            $roundTrip.Package.Properties.PublisherDisplayName -cne $PublisherDisplayName -or
            $roundTripVisual.GetAttribute('DisplayName') -cne $DisplayName) {
            throw 'Packaged display names do not match the requested Store names.'
        }
        if (Test-Path (Join-Path $unpacked 'AppxSignature.p7x')) { throw 'The Store submission artifact should be unsigned.' }
        $digest = (Get-FileHash $package -Algorithm SHA256).Hash.ToLowerInvariant()
        "$digest  $name" | Set-Content "$package.sha256" -Encoding ascii
        $commit = (& git rev-parse HEAD)
        if ($LASTEXITCODE -ne 0) { throw 'Cannot record the source commit.' }
        [ordered]@{
            identityMode = $(if ($TestIdentity) { 'test-only' } else { 'store' })
            name = $IdentityName; publisher = $Publisher; publisherDisplayName = $PublisherDisplayName
            displayName = $DisplayName; appVersion = $crateVersion; version = $PackageVersion; architecture = 'x64'
            minimumWindowsVersion = '10.0.19041.0'; sdkVersion = $sdk.Name
            sourceCommit = $commit; sha256 = $digest; signed = $false
            offlineTranslation = 'CTranslate2 CPU'; languageModelsBundled = $false
            translationRuntimeSha256 = (Get-FileHash (Join-Path $runtime 'mypdf_ct2.dll') -Algorithm SHA256).Hash.ToLowerInvariant()
        } | ConvertTo-Json | Set-Content "$package.json" -Encoding utf8
        Write-Host "Built $package"
        Write-Host $(if ($TestIdentity) { 'TEST ONLY: do not submit this identity to the Store or publish it as an installer.' } else { 'Unsigned Partner Center upload: Store certification and signing have not yet occurred.' })
    } catch {
        # Do not leave a failed package looking like a successful artifact.
        Remove-Item $package, "$package.sha256", "$package.json" -Force -ErrorAction SilentlyContinue
        throw
    } finally { Remove-Item $work -Recurse -Force }
} finally { Pop-Location }

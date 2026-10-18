# Disposable Windows runner only. Signs a COPY with a one-day test identity,
# installs/uninstalls it, and removes the certificate. Never uploads that copy.
# Run in elevated Windows PowerShell 5.1 (the Appx module's native host).
param([string]$PackageDirectory = 'dist/msix')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$identity = 'MyPDF.PackagingTest'
if (Get-AppxPackage -Name $identity) { throw 'A packaging-test app already exists; refusing to change it.' }
$packages = @(Get-ChildItem $PackageDirectory -Filter '*-TEST-ONLY-x64.msix')
if ($packages.Count -ne 1) { throw 'Expected exactly one TEST-ONLY MSIX.' }
$package = $packages[0]
$info = Get-Content ($package.FullName + '.json') -Raw -Encoding utf8 | ConvertFrom-Json
if ($info.identityMode -ne 'test-only' -or $info.name -ne $identity -or $info.publisher -ne 'CN=MyPDF Packaging Test') {
    throw 'Only the isolated packaging-test identity may be installed by this script.'
}
$originalHash = (Get-FileHash $package.FullName -Algorithm SHA256).Hash
$sdk = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Directory | Where-Object {
    $_.Name -match '^10\.0\.\d+\.\d+$' -and (Test-Path (Join-Path $_.FullName 'x64/signtool.exe'))
} | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $sdk) { throw 'Windows SDK SignTool is required.' }
$scratch = Join-Path $env:TEMP ('mypdf-msix-install-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $scratch
$data = Join-Path $env:LOCALAPPDATA 'MyPDF'
$dataExisted = Test-Path $data
$null = New-Item -ItemType Directory -Path $data -Force
$sentinel = Join-Path $data ('.msix-test-' + [guid]::NewGuid().ToString('N'))
$token = [guid]::NewGuid().ToString('N')
Set-Content $sentinel $token -Encoding ascii
$cert = $null
$installed = $null
try {
    $cert = New-SelfSignedCertificate -Type Custom -Subject $info.publisher -KeyUsage DigitalSignature `
        -CertStoreLocation 'Cert:\CurrentUser\My' -HashAlgorithm SHA256 -NotAfter (Get-Date).AddDays(1) `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
    $publicCert = Join-Path $scratch 'test.cer'
    $null = Export-Certificate -Cert $cert -FilePath $publicCert
    $null = Import-Certificate -FilePath $publicCert -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople'
    $signed = Join-Path $scratch 'test-signed.msix'
    Copy-Item $package.FullName $signed
    $signtool = Join-Path $sdk.FullName 'x64/signtool.exe'
    & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $signed
    if ($LASTEXITCODE -ne 0) { throw 'Test-copy signing failed.' }
    & $signtool verify /pa $signed
    if ($LASTEXITCODE -ne 0) { throw 'Test-copy signature verification failed.' }
    Add-AppxPackage -Path $signed
    $installed = Get-AppxPackage -Name $identity
    if (-not $installed -or $installed.Version -ne $info.version) { throw 'Installed package/version not found.' }
    $manifest = Get-AppxPackageManifest -Package $installed.PackageFullName
    if ($manifest.Package.Applications.Application.Id -ne 'MyPDF') { throw 'Missing desktop application registration.' }
    if (-not (Test-Path (Join-Path $installed.InstallLocation 'MyPDF.exe'))) { throw 'Packaged executable missing.' }
    & (Join-Path $PSScriptRoot 'test-translation-runtime.ps1') -RuntimeDirectory (Join-Path $installed.InstallLocation 'translation')
    Write-Host "PASS package installation: $($installed.PackageFullName)"
    Remove-AppxPackage -Package $installed.PackageFullName
    $installed = $null
    if (Get-AppxPackage -Name $identity) { throw 'Test package was not uninstalled.' }
    if ((Get-Content $sentinel -Raw).Trim() -ne $token) { throw 'Existing MyPDF data was changed by package installation/removal.' }
    if ((Get-FileHash $package.FullName -Algorithm SHA256).Hash -ne $originalHash) { throw 'Unsigned upload artifact was modified.' }
    Write-Host 'PASS uninstall, existing-data preservation, and unchanged unsigned submission artifact.'
} finally {
    if ($installed) { Remove-AppxPackage -Package $installed.PackageFullName }
    if ($cert) {
        Remove-Item "Cert:\LocalMachine\TrustedPeople\$($cert.Thumbprint)" -ErrorAction SilentlyContinue
        Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -DeleteKey -ErrorAction SilentlyContinue
    }
    Remove-Item $sentinel -Force -ErrorAction SilentlyContinue
    if (-not $dataExisted -and @(Get-ChildItem $data -Force).Count -eq 0) { Remove-Item $data }
    Remove-Item $scratch -Recurse -Force
}

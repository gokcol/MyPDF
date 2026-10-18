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
$runtimeHost = $null
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
    # Exercise WindowsApps code with the installed app's package identity;
    # an unrelated PowerShell host may lack permission to map the DLL for execution.
    $runtimeResult = Join-Path $scratch 'runtime-result.json'
    $runtimeStarted = Join-Path $scratch 'runtime-started.json'
    $runtimeConfig = [ordered]@{
        packageFullName = $installed.PackageFullName
        runtimeDirectory = Join-Path $installed.InstallLocation 'translation'
        smokeScript = Join-Path $PSScriptRoot 'test-translation-runtime.ps1'
        resultPath = $runtimeResult
        startedPath = $runtimeStarted
    } | ConvertTo-Json -Compress
    $runtimeConfig64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($runtimeConfig))
    $runtimeCommand = @'
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$config = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__CONFIG__')) | ConvertFrom-Json
$process = [Diagnostics.Process]::GetCurrentProcess()
@{ pid = $PID; startTicks = $process.StartTime.ToUniversalTime().Ticks } |
    ConvertTo-Json -Compress | Set-Content ($config.startedPath + '.tmp') -Encoding utf8
Move-Item ($config.startedPath + '.tmp') $config.startedPath
$result = [ordered]@{ success = $false; packageFullName = $null; error = $null }
try {
    Add-Type @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class MyPDFPackageIdentity {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true)]
    static extern int GetCurrentPackageFullName(ref uint length, StringBuilder name);
    public static string Read() {
        uint length = 0;
        int code = GetCurrentPackageFullName(ref length, null);
        if (code != 122) throw new Win32Exception(code);
        var name = new StringBuilder(checked((int)length));
        code = GetCurrentPackageFullName(ref length, name);
        if (code != 0) throw new Win32Exception(code);
        return name.ToString();
    }
}
"@
    $result.packageFullName = [MyPDFPackageIdentity]::Read()
    if ($result.packageFullName -cne $config.packageFullName) {
        throw 'Runtime smoke host does not have the expected installed package identity.'
    }
    & $config.smokeScript -RuntimeDirectory $config.runtimeDirectory
    $result.success = $true
} catch {
    $result.error = $_.Exception.Message
} finally {
    $result | ConvertTo-Json -Compress | Set-Content ($config.resultPath + '.tmp') -Encoding utf8
    Move-Item ($config.resultPath + '.tmp') $config.resultPath
}
'@
    $runtimeCommand = $runtimeCommand.Replace('__CONFIG__', $runtimeConfig64)
    $runtimeCommand64 = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($runtimeCommand))
    Invoke-CommandInDesktopPackage -PackageFamilyName $installed.PackageFamilyName -AppId 'MyPDF' `
        -Command "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" `
        -Args "-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $runtimeCommand64" `
        -PreventBreakaway
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (-not (Test-Path $runtimeResult) -and [DateTime]::UtcNow -lt $deadline) {
        if (-not $runtimeHost -and (Test-Path $runtimeStarted)) {
            $started = Get-Content $runtimeStarted -Raw -Encoding utf8 | ConvertFrom-Json
            $candidate = Get-Process -Id $started.pid -ErrorAction SilentlyContinue
            if ($candidate) {
                try {
                    if ($candidate.StartTime.ToUniversalTime().Ticks -eq $started.startTicks) {
                        $runtimeHost = $candidate
                    }
                } catch {
                    # The short-lived host may exit between lookup and inspection.
                    # Its atomic result file remains the authoritative outcome.
                }
            }
        }
        Start-Sleep -Milliseconds 200
    }
    if (-not (Test-Path $runtimeResult)) { throw 'Installed package runtime smoke check timed out.' }
    $runtimeCheck = Get-Content $runtimeResult -Raw -Encoding utf8 | ConvertFrom-Json
    if (-not $runtimeCheck.success -or $runtimeCheck.packageFullName -cne $installed.PackageFullName) {
        throw "Installed package runtime smoke check failed: $($runtimeCheck.error)"
    }
    if ($runtimeHost -and -not $runtimeHost.WaitForExit(10000)) {
        throw 'Installed package runtime smoke host did not exit.'
    }
    Write-Host 'PASS installed translation runtime: package identity, checksum, dependency load, exports, and ABI.'
    Write-Host "PASS package installation: $($installed.PackageFullName)"
    Remove-AppxPackage -Package $installed.PackageFullName
    $installed = $null
    if (Get-AppxPackage -Name $identity) { throw 'Test package was not uninstalled.' }
    if ((Get-Content $sentinel -Raw).Trim() -ne $token) { throw 'Existing MyPDF data was changed by package installation/removal.' }
    if ((Get-FileHash $package.FullName -Algorithm SHA256).Hash -ne $originalHash) { throw 'Unsigned upload artifact was modified.' }
    Write-Host 'PASS uninstall, existing-data preservation, and unchanged unsigned submission artifact.'
} finally {
    if ($runtimeHost -and -not $runtimeHost.HasExited) { $runtimeHost.Kill(); $runtimeHost.WaitForExit(10000) | Out-Null }
    if ($installed) { Remove-AppxPackage -Package $installed.PackageFullName }
    if ($cert) {
        Remove-Item "Cert:\LocalMachine\TrustedPeople\$($cert.Thumbprint)" -ErrorAction SilentlyContinue
        Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -DeleteKey -ErrorAction SilentlyContinue
    }
    Remove-Item $sentinel -Force -ErrorAction SilentlyContinue
    if (-not $dataExisted -and @(Get-ChildItem $data -Force).Count -eq 0) { Remove-Item $data }
    Remove-Item $scratch -Recurse -Force
}

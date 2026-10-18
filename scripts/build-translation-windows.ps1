# Produces the native offline runtime. Models are downloaded separately only when
# the user requests a language; none are bundled into the installer.
[CmdletBinding()]
param(
    [string]$OutputDirectory = 'dist/translation/windows-x64',
    [string]$BuildDirectory = 'target/translation-windows',
    [string]$Generator = 'Visual Studio 17 2022'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Build the Windows native runtime on Windows with Visual Studio C++ tools.' }
$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
try {
    $output = [System.IO.Path]::GetFullPath($OutputDirectory)
    & cmake -S native/translation-ct2 -B $BuildDirectory -G $Generator -A x64 "-DCMAKE_INSTALL_PREFIX=$output"
    if ($LASTEXITCODE -ne 0) { throw 'Translation CMake configuration failed.' }
    & cmake --build $BuildDirectory --config Release --target mypdf_ct2 --parallel 4
    if ($LASTEXITCODE -ne 0) { throw 'Translation native build failed.' }
    & cmake --install $BuildDirectory --config Release --component MyPDFTranslation
    if ($LASTEXITCODE -ne 0) { throw 'Translation runtime installation failed.' }
    $dll = Join-Path $output 'mypdf_ct2.dll'
    if (-not (Test-Path $dll)) { throw 'The translation runtime DLL is missing.' }
    $hash = (Get-FileHash $dll -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  mypdf_ct2.dll" | Out-File -Encoding ascii (Join-Path $output 'mypdf_ct2.dll.sha256')
    Write-Host "Native offline translation runtime: $output"
} finally { Pop-Location }

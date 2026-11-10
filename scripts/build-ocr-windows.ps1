# Build the offline OCR executable from a pinned official vcpkg revision.
[CmdletBinding()]
param([string]$OutputDirectory = 'dist/ocr/windows-x64')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
try {
    $revision = 'dee92af2943e2216d7ba21ec4516a2c538c34c53'
    $root = Join-Path $repo 'target/ocr-vcpkg'
    if (-not (Test-Path (Join-Path $root '.git'))) {
        & git init $root
        if ($LASTEXITCODE -ne 0) { throw 'Cannot initialize OCR build sources.' }
        & git -C $root remote add origin https://github.com/microsoft/vcpkg.git
    }
    & git -C $root fetch --depth 1 origin $revision
    if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch pinned OCR build sources.' }
    & git -C $root checkout --detach $revision
    if ($LASTEXITCODE -ne 0) { throw 'Cannot check out pinned OCR build sources.' }
    & (Join-Path $root 'bootstrap-vcpkg.bat') -disableMetrics
    if ($LASTEXITCODE -ne 0) { throw 'Cannot bootstrap vcpkg.' }
    # No curl/archive features: OCR cannot fetch remote images or language models.
    & (Join-Path $root 'vcpkg.exe') install 'tesseract[core]:x64-windows-mypdf' --classic "--overlay-triplets=$repo/packaging/windows/ocr" --disable-metrics
    if ($LASTEXITCODE -ne 0) { throw 'Cannot build the offline OCR engine.' }
    $installed = Join-Path $root 'installed/x64-windows-mypdf'
    $output = [IO.Path]::GetFullPath($OutputDirectory)
    $null = New-Item -ItemType Directory -Path "$output/tessdata", "$output/licenses" -Force
    Copy-Item "$installed/tools/tesseract/tesseract.exe" "$output/tesseract.exe" -Force
    # Static linking must not hide a redistributable dependency available only on CI.
    $vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $dumpbin = Get-ChildItem "$vs/VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe" | Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $dumpbin) { throw 'dumpbin is required to verify the standalone OCR runtime.' }
    $imports = & $dumpbin.FullName /dependents "$output/tesseract.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect OCR runtime dependencies.' }
    foreach ($line in $imports) {
        if ($line -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$' -and $Matches[1] -notmatch '^(?i:KERNEL32|ADVAPI32|USER32|SHELL32|OLE32|OLEAUT32|GDI32|WS2_32|BCRYPT|CRYPT32|NTDLL|MSVCRT|api-ms-win-.*)\.dll$') {
            throw "Unexpected non-system OCR dependency: $($Matches[1])"
        }
    }
    foreach ($license in Get-ChildItem "$installed/share/*/copyright") {
        Copy-Item $license.FullName "$output/licenses/$($license.Directory.Name).txt" -Force
    }
    $models = Get-Content packaging/windows/ocr/models.json -Raw | ConvertFrom-Json
    foreach ($model in $models.models) {
        $file = "$output/tessdata/$($model.language).traineddata"
        if (-not (Test-Path $file) -or (Get-FileHash $file -Algorithm SHA256).Hash -ine $model.sha256) {
            Invoke-WebRequest "https://raw.githubusercontent.com/$($models.repository)/$($models.commit)/$($model.language).traineddata" -OutFile $file
        }
        if ((Get-FileHash $file -Algorithm SHA256).Hash -ine $model.sha256) { throw "OCR model checksum mismatch: $($model.language)" }
    }
    Invoke-WebRequest "https://raw.githubusercontent.com/$($models.repository)/$($models.commit)/LICENSE" -OutFile "$output/licenses/tessdata_fast.txt"
    ($models.models.language -join '+') | Set-Content "$output/languages.txt" -Encoding ascii
    Copy-Item packaging/windows/ocr/models.json "$output/models.json" -Force
    $hashes = [ordered]@{}
    foreach ($file in Get-ChildItem $output -Recurse -File | Where-Object Name -ne 'sha256.json') {
        $relative = $file.FullName.Substring($output.Length + 1).Replace('\', '/')
        $hashes[$relative] = (Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $hashes | ConvertTo-Json | Set-Content "$output/sha256.json" -Encoding utf8
    & "$output/tesseract.exe" --version
    if ($LASTEXITCODE -ne 0) { throw 'OCR runtime cannot launch.' }
    Write-Host "Offline OCR runtime and languages: $output"
} finally { Pop-Location }

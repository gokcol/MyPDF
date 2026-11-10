[CmdletBinding()]
param([string]$SourceDirectory = 'dist/ocr/windows-x64', [Parameter(Mandatory = $true)][string]$DestinationDirectory)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$source = [IO.Path]::GetFullPath($SourceDirectory)
$destination = Join-Path ([IO.Path]::GetFullPath($DestinationDirectory)) 'ocr'
if (Test-Path $destination) { throw 'OCR destination already exists.' }
$manifest = Get-Content "$source/sha256.json" -Raw | ConvertFrom-Json
foreach ($required in @('tesseract.exe', 'languages.txt', 'models.json', 'licenses/tesseract.txt', 'licenses/leptonica.txt', 'licenses/tessdata_fast.txt')) {
    if ($required -notin $manifest.PSObject.Properties.Name) { throw "Required OCR payload missing: $required" }
}
if (@(Get-ChildItem $source -Recurse -Force | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) { throw 'OCR payload contains filesystem links.' }
foreach ($entry in $manifest.PSObject.Properties) {
    if ($entry.Name -match '(^/|^\\|:|\.\.)' -or (Get-FileHash (Join-Path $source $entry.Name) -Algorithm SHA256).Hash -ine $entry.Value) {
        throw "OCR payload verification failed: $($entry.Name)"
    }
}
$models = Get-Content "$source/models.json" -Raw | ConvertFrom-Json
foreach ($model in $models.models) {
    if ((Get-FileHash "$source/tessdata/$($model.language).traineddata" -Algorithm SHA256).Hash -ine $model.sha256) { throw 'OCR model validation failed.' }
}
$null = New-Item -ItemType Directory -Path $destination
foreach ($entry in $manifest.PSObject.Properties) {
    $target = Join-Path $destination $entry.Name
    $null = New-Item -ItemType Directory -Path (Split-Path $target -Parent) -Force
    Copy-Item (Join-Path $source $entry.Name) $target
    if ((Get-FileHash $target -Algorithm SHA256).Hash -ine $entry.Value) { throw 'OCR copy checksum mismatch.' }
}
Copy-Item "$source/sha256.json" "$destination/sha256.json"

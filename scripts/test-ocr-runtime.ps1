# Runs under the installed MSIX identity too: launch a bundled child process and
# recognize an in-memory scan, without writing inside WindowsApps.
param([Parameter(Mandatory = $true)][string]$RuntimeDirectory)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.Drawing
$bitmap = New-Object Drawing.Bitmap 1000, 160
$graphics = [Drawing.Graphics]::FromImage($bitmap)
$font = New-Object Drawing.Font 'Arial', 34
$stream = New-Object IO.MemoryStream
$process = New-Object Diagnostics.Process
try {
    $graphics.Clear([Drawing.Color]::White)
    $graphics.DrawString('Sample invoice 1250', $font, [Drawing.Brushes]::Black, 20, 35)
    $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
    $process.StartInfo.FileName = Join-Path $RuntimeDirectory 'tesseract.exe'
    $process.StartInfo.WorkingDirectory = $RuntimeDirectory
    $process.StartInfo.Arguments = 'stdin stdout --tessdata-dir tessdata -l eng --dpi 216 -c tessedit_create_tsv=1'
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardInput = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    $null = $process.Start()
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    $bytes = $stream.ToArray()
    $process.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(60000)) { $process.Kill(); throw 'Installed OCR timed out.' }
    if ($process.ExitCode -ne 0 -or $output.Result -notmatch 'invoice' -or $output.Result -notmatch '1250') {
        throw "Installed OCR recognition failed: $($errors.Result) $($output.Result)"
    }
    Write-Host 'PASS installed offline OCR: rendered image, bundled models, child launch, and recognized words.'
} finally {
    $process.Dispose()
    $stream.Dispose()
    $font.Dispose()
    $graphics.Dispose()
    $bitmap.Dispose()
}

[CmdletBinding()]
param([string]$RuntimeDirectory = 'dist/translation/windows-x64')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$dll = [System.IO.Path]::GetFullPath((Join-Path $RuntimeDirectory 'mypdf_ct2.dll'))
if (-not (Test-Path $dll)) { throw 'Offline translation DLL is missing.' }
$stream = [System.IO.File]::OpenRead($dll)
$reader = [System.IO.BinaryReader]::new($stream)
try {
    if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'Translation runtime has no PE header.' }
    $stream.Position = 0x3c
    $peOffset = $reader.ReadUInt32()
    if ($peOffset -gt $stream.Length - 6) { throw 'Translation runtime PE header is invalid.' }
    $stream.Position = $peOffset
    if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
        throw 'Translation runtime must be a Windows x64 DLL.'
    }
} finally { $reader.Dispose(); $stream.Dispose() }
$expected = ((Get-Content "$dll.sha256" -Raw).Trim() -split '\s+')[0]
$actual = (Get-FileHash $dll -Algorithm SHA256).Hash.ToLowerInvariant()
if ($expected -notmatch '^[a-f0-9]{64}$' -or $actual -ne $expected) { throw 'Offline translation runtime checksum mismatch.' }
if (-not ('MyPDFOfflineTranslationSmoke' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MyPDFOfflineTranslationSmoke {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr LoadLibraryEx(string path, IntPtr reserved, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Ansi)]
    static extern IntPtr GetProcAddress(IntPtr module, string symbol);
    [DllImport("kernel32.dll")] static extern bool FreeLibrary(IntPtr module);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate uint Abi();
    public static uint Check(string path) {
        var module = LoadLibraryEx(path, IntPtr.Zero, 0x100 | 0x800);
        if (module == IntPtr.Zero) throw new Exception("Native runtime/dependency load failed: " + Marshal.GetLastWin32Error());
        try {
            foreach (var symbol in new[] {"mypdf_ct2_abi", "mypdf_ct2_create", "mypdf_ct2_destroy", "mypdf_ct2_translate", "mypdf_ct2_free"})
                if (GetProcAddress(module, symbol) == IntPtr.Zero) throw new Exception("Native translation export missing: " + symbol);
            var callback = (Abi)Marshal.GetDelegateForFunctionPointer(GetProcAddress(module, "mypdf_ct2_abi"), typeof(Abi));
            return callback();
        } finally { FreeLibrary(module); }
    }
}
'@
}
if ([MyPDFOfflineTranslationSmoke]::Check($dll) -ne 1) { throw 'Offline translation ABI mismatch.' }
Write-Host 'Offline translation native DLL: checksum, dependency load, and ABI passed.'

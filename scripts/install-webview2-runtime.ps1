[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'The WebView2 Runtime installer requires Windows' }

$client = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$paths = @(
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$client",
    "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$client",
    "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$client"
)
$runtime = $paths | ForEach-Object { Get-ItemProperty $_ -Name pv -ErrorAction SilentlyContinue } |
    Where-Object { $_.pv -and $_.pv -ne '0.0.0.0' }
if ($runtime) {
    Write-Output 'Microsoft WebView2 Runtime is already installed'
    return
}

# Official distribution procedure: https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution
$installerPath = Join-Path ([IO.Path]::GetTempPath()) "bruhs-webview2-$([guid]::NewGuid()).exe"
try {
    Invoke-WebRequest 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $installerPath
    $signature = Get-AuthenticodeSignature $installerPath
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw 'The WebView2 installer does not have a valid Microsoft signature'
    }
    $installer = Start-Process -FilePath $installerPath -ArgumentList '/silent','/install' -Wait -PassThru
    if ($installer.ExitCode -ne 0) { throw "WebView2 installer exit code $($installer.ExitCode)" }
} finally {
    Remove-Item -Force $installerPath -ErrorAction SilentlyContinue
}

<#
.SYNOPSIS
    Captures one VF frame on Windows into target\vf\ (the Windows twin of capture.sh).

.DESCRIPTION
    ref  : headless Chrome screenshot of the harman mock at 1440x900 x Scale.
    cand : starts spike-harman, waits for its window, and captures the window's client area
           (PrintWindow with PW_RENDERFULLCONTENT by default, or CopyFromScreen) through .NET.
    The PNG is written by System.Drawing as untagged sRGB (no iCCP chunk), which is what
    `cargo run -p xtask -- vf` expects. Run from any directory; paths resolve from the repo root.
    Run the candidate on a display whose Windows scale matches -Scale (150 % for 1.5).

.EXAMPLE
    pwsh design/vf/capture.ps1 -Kind ref  -Theme dark -Lang en -Scale 1.5
    pwsh design/vf/capture.ps1 -Kind cand -Theme dark -Lang en -Scale 1.5
    cargo run -p xtask -- vf --reference target/vf/ref-dark-en-1.5x.png `
        --candidate target/vf/cand-dark-en-1.5x.png --scale 1.5 --theme dark
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('ref', 'cand')]
    [string]$Kind,

    [ValidateSet('dark', 'light')]
    [string]$Theme = 'dark',

    [ValidateSet('en', 'tr')]
    [string]$Lang = 'en',

    [ValidateSet('1', '1.25', '1.5', '2')]
    [string]$Scale = '1.5',

    # Chrome (or Chromium) used for the reference.
    [string]$Chrome = 'C:\Program Files\Google\Chrome\Application\chrome.exe',

    # spike-harman.exe; defaults to <CARGO_TARGET_DIR or target>\debug\spike-harman.exe.
    [string]$Harman = '',

    # Index into spike-harman's display list (passed as --display).
    [int]$Display = -1,

    [ValidateSet('PrintWindow', 'CopyFromScreen')]
    [string]$Method = 'PrintWindow',

    # Seconds to let the first frames settle before capturing.
    [int]$SettleSeconds = 2
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Out = Join-Path $Root 'target\vf'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Tag = "$Theme-$Lang-${Scale}x"

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class VfWin32
{
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }

    [DllImport("user32.dll")]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr value);

    [DllImport("user32.dll")]
    public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll")]
    public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
}
'@

# Physical pixels everywhere: per-monitor DPI aware v2 (-4).
[void][VfWin32]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))

function Save-Png {
    param([System.Drawing.Bitmap]$Bitmap, [string]$Path)
    $Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
}

function Get-ClientBitmap {
    param([IntPtr]$Handle, [string]$CaptureMethod)
    $rect = New-Object VfWin32+RECT
    if (-not [VfWin32]::GetClientRect($Handle, [ref]$rect)) { throw 'GetClientRect failed' }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    $bitmap = New-Object System.Drawing.Bitmap($width, $height, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        if ($CaptureMethod -eq 'PrintWindow') {
            $hdc = $graphics.GetHdc()
            try {
                # PW_CLIENTONLY (1) | PW_RENDERFULLCONTENT (2): includes DirectX content.
                if (-not [VfWin32]::PrintWindow($Handle, $hdc, 3)) { throw 'PrintWindow failed' }
            }
            finally { $graphics.ReleaseHdc($hdc) }
        }
        else {
            $origin = New-Object VfWin32+POINT
            if (-not [VfWin32]::ClientToScreen($Handle, [ref]$origin)) { throw 'ClientToScreen failed' }
            [void][VfWin32]::SetForegroundWindow($Handle)
            Start-Sleep -Milliseconds 300
            $graphics.CopyFromScreen($origin.X, $origin.Y, 0, 0, $bitmap.Size)
        }
    }
    finally { $graphics.Dispose() }
    return $bitmap
}

switch ($Kind) {
    'ref' {
        if (-not (Test-Path -LiteralPath $Chrome)) { throw "Chrome not found at $Chrome; pass -Chrome" }
        $page = ([System.Uri](Join-Path $Root 'design\directions\harman\index.html')).AbsoluteUri
        $url = "${page}?theme=$Theme&lang=$Lang&view=workspace&result=1"
        $png = Join-Path $Out "ref-$Tag.png"
        $chromeArgs = @(
            '--headless=new', '--disable-gpu', '--hide-scrollbars', '--window-size=1440,900',
            "--force-device-scale-factor=$Scale", '--virtual-time-budget=4000', "--screenshot=$png", $url
        )
        $proc = Start-Process -FilePath $Chrome -ArgumentList $chromeArgs -Wait -PassThru -NoNewWindow
        if ($proc.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $png)) {
            throw "Chrome reference capture failed (exit $($proc.ExitCode))"
        }
    }
    'cand' {
        if (-not $Harman) {
            $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
            $Harman = Join-Path $targetDir 'debug\spike-harman.exe'
        }
        if (-not (Test-Path -LiteralPath $Harman)) { throw "spike-harman not found at $Harman; run cargo build -p spike-harman" }
        $log = Join-Path $Out "cand-$Tag.log"
        $errLog = Join-Path $Out "cand-$Tag.err.log"
        $harmanArgs = @('--theme', $Theme, '--lang', $Lang, '--scale', $Scale, '--print-window-id')
        if ($Display -ge 0) { $harmanArgs += @('--display', "$Display") }
        $proc = Start-Process -FilePath $Harman -ArgumentList $harmanArgs -PassThru `
            -RedirectStandardOutput $log -RedirectStandardError $errLog
        try {
            $handle = [IntPtr]::Zero
            for ($i = 0; $i -lt 50 -and $handle -eq [IntPtr]::Zero; $i++) {
                if ($proc.HasExited) {
                    Get-Content -LiteralPath $errLog | Write-Error
                    throw "spike-harman exited with $($proc.ExitCode)"
                }
                Start-Sleep -Milliseconds 200
                $proc.Refresh()
                $handle = $proc.MainWindowHandle
            }
            if ($handle -eq [IntPtr]::Zero) { throw 'spike-harman did not open a window' }
            Start-Sleep -Seconds $SettleSeconds
            $bitmap = Get-ClientBitmap -Handle $handle -CaptureMethod $Method
            try {
                $want = @([int][math]::Round(1440 * [double]$Scale), [int][math]::Round(900 * [double]$Scale))
                if ($bitmap.Width -ne $want[0] -or $bitmap.Height -ne $want[1]) {
                    Write-Warning "client area is $($bitmap.Width)x$($bitmap.Height), expected $($want[0])x$($want[1]); is the display at $([double]$Scale * 100) %?"
                }
                Save-Png -Bitmap $bitmap -Path (Join-Path $Out "cand-$Tag.png")
            }
            finally { $bitmap.Dispose() }
        }
        finally {
            if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
        }
    }
}
Write-Output "wrote $(Join-Path $Out "$Kind-$Tag.png")"

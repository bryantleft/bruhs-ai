[CmdletBinding()]
param(
    [string] $BinaryDirectory = (Join-Path $PSScriptRoot '..\target\debug'),
    [string] $OutputDirectory = (Join-Path $PSScriptRoot '..\out\windows-rich-smoke'),
    [switch] $SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Matches the successful manual Linux save/reopen observation window, 2026-11-13.
$ObservationMilliseconds = 120000
# ServerClient::REQUEST_TIMEOUT, in bruhs-desktop/src/server_client.rs.
$ApiTimeoutMilliseconds = 20000
# schedule_desktop_terminal_smoke's existing opening interval; used to probe readiness.
$ProbeMilliseconds = 800

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public sealed class RichSmokeProcess : IDisposable {
    public readonly Process Child;
    public readonly AutoResetEvent Changed = new AutoResetEvent(false);
    readonly StringBuilder log = new StringBuilder();
    readonly object gate = new object();
    readonly string path;
    bool disposed;
    public string Log { get { lock (gate) { return log.ToString(); } } }
    public RichSmokeProcess(string executable, string[] arguments, string workingDirectory,
                            IDictionary<string,string> environment, string logPath) {
        path = logPath;
        File.WriteAllText(path, "");
        var start = new ProcessStartInfo(executable) {
            WorkingDirectory = workingDirectory, UseShellExecute = false,
            CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8
        };
        foreach (var argument in arguments) start.ArgumentList.Add(argument);
        foreach (var pair in environment) start.Environment[pair.Key] = pair.Value;
        Child = new Process { StartInfo = start, EnableRaisingEvents = true };
        Child.OutputDataReceived += Receive;
        Child.ErrorDataReceived += Receive;
        Child.Exited += (sender, args) => { lock (gate) { if (!disposed) Changed.Set(); } };
        if (!Child.Start()) throw new InvalidOperationException("Could not start " + executable);
        Child.BeginOutputReadLine();
        Child.BeginErrorReadLine();
    }
    void Receive(object sender, DataReceivedEventArgs args) {
        if (args.Data == null) return;
        lock (gate) {
            if (disposed) return;
            log.AppendLine(args.Data);
            File.AppendAllText(path, args.Data + Environment.NewLine);
            Changed.Set();
        }
    }
    public void Dispose() {
        if (!Child.HasExited) Child.Kill(true);
        Child.WaitForExit();
        Child.Dispose();
        lock (gate) { disposed = true; Changed.Dispose(); }
    }
}

public static class RichSmokeWindows {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct MouseInput {
        public int X, Y; public uint Data, Flags, Time; public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint Type; public MouseInput Mouse; }
    delegate bool EnumWindow(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindow callback, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumWindow callback, IntPtr parameter);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out Rect bounds);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("dwmapi.dll")] static extern int DwmFlush();
    public static IntPtr MainWindow(int process) {
        IntPtr result = IntPtr.Zero; long largest = 0;
        EnumWindows((window, parameter) => {
            uint owner; Rect bounds;
            GetWindowThreadProcessId(window, out owner);
            if (owner == process && IsWindowVisible(window) && GetWindowRect(window, out bounds)) {
                long area = (long)(bounds.Right-bounds.Left)*(bounds.Bottom-bounds.Top);
                if (area > largest) { result = window; largest = area; }
            }
            return true;
        }, IntPtr.Zero);
        if (result == IntPtr.Zero) throw new InvalidOperationException("No visible native app window");
        return result;
    }
    public static Rect Bounds(int process) {
        Rect result;
        if (!GetWindowRect(MainWindow(process), out result)) throw new InvalidOperationException("No window bounds");
        return result;
    }
    public static void Prepare(int process) {
        var window = MainWindow(process); var bounds = Bounds(process);
        SetWindowPos(window, new IntPtr(-1), 0, 0,
                     Math.Min(bounds.Right-bounds.Left, GetSystemMetrics(0)),
                     Math.Min(bounds.Bottom-bounds.Top, GetSystemMetrics(1)), 0x40);
        SetForegroundWindow(window);
        DwmFlush();
    }
    public static Rect WebView(int process) {
        Rect result = new Rect(); long largest = 0;
        EnumChildWindows(MainWindow(process), (window, parameter) => {
            var name = new StringBuilder(256); Rect bounds;
            GetClassName(window, name, name.Capacity);
            if (name.ToString().StartsWith("Chrome_") && IsWindowVisible(window) && GetWindowRect(window, out bounds)) {
                long area = (long)(bounds.Right-bounds.Left)*(bounds.Bottom-bounds.Top);
                if (area > largest) { result = bounds; largest = area; }
            }
            return true;
        }, IntPtr.Zero);
        if (largest <= 0) throw new InvalidOperationException("No visible WebView2 child HWND");
        var parent = Bounds(process);
        if (result.Left < parent.Left || result.Top < parent.Top || result.Right > parent.Right || result.Bottom > parent.Bottom)
            throw new InvalidOperationException("WebView2 is outside its native parent bounds");
        return result;
    }
    public static void ClickCounter(int process) {
        var bounds = WebView(process);
        var point = new Point { X = bounds.Left+(bounds.Right-bounds.Left)/2, Y = bounds.Top+(bounds.Bottom-bounds.Top)/2 };
        if (GetAncestor(WindowFromPoint(point), 2) != MainWindow(process))
            throw new InvalidOperationException("The native widget is occluded");
        if (!SetCursorPos(point.X, point.Y)) throw new InvalidOperationException("Cannot move the Windows pointer");
        var events = new[] {
            new Input { Type=0, Mouse=new MouseInput { Flags=0x2 } },
            new Input { Type=0, Mouse=new MouseInput { Flags=0x4 } }
        };
        if (SendInput(2, events, Marshal.SizeOf<Input>()) != 2)
            throw new InvalidOperationException("Windows rejected native pointer input");
        DwmFlush();
    }
    public static void Close(int process) {
        if (!PostMessage(MainWindow(process), 0x10, IntPtr.Zero, IntPtr.Zero))
            throw new InvalidOperationException("Windows rejected WM_CLOSE");
    }
}
'@

function Read-NativeReport([string] $Text, [int] $Count) {
    $state = [regex]::Match($Text, 'state_key=([a-f0-9]{64}).*revision=(\d+) saved=(\d+)')
    $native = [regex]::Match($Text, '(?m)BRUHS_RICH_RESPONSE_SMOKE native=(.+)$')
    if (!$state.Success -or !$native.Success) { return $null }
    if ($Text -notmatch 'BRUHS_RICH_RESPONSE_SMOKE hosts=1 shell_ready=true') { throw 'Native host is not ready' }
    $packet = ConvertFrom-Json $native.Groups[1].Value
    if ($packet -is [string]) { $packet = ConvertFrom-Json $packet }
    if (!$packet.root -or $packet.sandbox -ne 'allow-scripts' -or !$packet.probe.parentBlocked) {
        throw 'Native sandbox isolation failed'
    }
    if ([int]$packet.probe.count -ne $Count) { throw "Expected restored counter $Count; got $($packet.probe.count)" }
    if ([int64]$state.Groups[2].Value -le 0 -or $state.Groups[2].Value -ne $state.Groups[3].Value) {
        throw 'Widget state has not been acknowledged as saved'
    }
    [pscustomobject]@{ Key=$state.Groups[1].Value; Native=$packet; Count=$Count }
}

if ($SelfTest) {
    $packet = @{ root=$true; sandbox='allow-scripts'; probe=@{parentBlocked=$true;count='1'} }
    $encoded = ConvertTo-Json (ConvertTo-Json $packet -Compress) -Compress
    $valid = "BRUHS_RICH_RESPONSE_SMOKE hosts=1 shell_ready=true`nstate_key=$('a'*64) revision=1 saved=1`nBRUHS_RICH_RESPONSE_SMOKE native=$encoded"
    if ((Read-NativeReport $valid 1).Count -ne 1) { throw 'Valid native report was rejected' }
    foreach ($bad in @($valid.Replace('saved=1','saved=0'), $valid.Replace('allow-scripts','allow-same-origin'))) {
        $rejected = $false
        try { $null = Read-NativeReport $bad 1 } catch { $rejected = $true }
        if (!$rejected) { throw 'Invalid native report was accepted' }
    }
    $rejected = $false
    try { $null = Read-NativeReport $valid 2 } catch { $rejected = $true }
    if (!$rejected) { throw 'Incorrect counter was accepted' }
    if ($null -ne (Read-NativeReport 'incomplete' 1)) { throw 'Incomplete report was accepted' }
    $packet.probe.parentBlocked = $false
    $unsafeEncoded = ConvertTo-Json (ConvertTo-Json $packet -Compress) -Compress
    $rejected = $false
    try { $null = Read-NativeReport $valid.Replace($encoded, $unsafeEncoded) 1 } catch { $rejected = $true }
    if (!$rejected) { throw 'Parent DOM access was accepted' }
    $temporary = Join-Path ([IO.Path]::GetTempPath()) "bruhs-smoke-self-test-$([guid]::NewGuid())"
    $null = New-Item -ItemType Directory $temporary
    $child = $null
    try {
        $executable = Join-Path $PSHOME $(if ($IsWindows) {'pwsh.exe'} else {'pwsh'})
        $environment = [Collections.Generic.Dictionary[string,string]]::new()
        $child = [RichSmokeProcess]::new($executable, @('-NoProfile','-NonInteractive','-Command',"Write-Output 'stdout-fixture'; [Console]::Error.WriteLine('stderr-fixture')"), $temporary, $environment, (Join-Path $temporary 'child.log'))
        if (!$child.Child.WaitForExit($ApiTimeoutMilliseconds)) { throw 'Capture-process self-test timed out' }
        $child.Child.WaitForExit()
        if ($child.Child.ExitCode -ne 0 -or $child.Log -notmatch 'stdout-fixture' -or $child.Log -notmatch 'stderr-fixture') {
            throw 'Capture-process self-test lost output or exit status'
        }
    } finally {
        if ($null -ne $child) { $child.Dispose() }
        Remove-Item -Recurse -Force $temporary
    }
    Write-Output 'Windows smoke self-test: 7 checks passed'
    return
}

if (!$IsWindows -or ![Environment]::UserInteractive) { throw 'A Windows interactive desktop session is required' }
Add-Type -AssemblyName System.Drawing
$BinaryDirectory = (Resolve-Path $BinaryDirectory).Path
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$null = New-Item -ItemType Directory -Force $OutputDirectory
$data = Join-Path $OutputDirectory 'data'
$null = New-Item -ItemType Directory -Force $data
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
$base = "http://127.0.0.1:$port"
$settings = [Collections.Generic.Dictionary[string,string]]::new()
$settings['BRUHS_SERVER_PORT'] = "$port"
$settings['BRUHS_SERVER_URL'] = $base
$settings['BRUHS_MEETING_DB'] = Join-Path $data 'meetings.sqlite3'
$settings['BRUHS_REACTIVE_DB'] = Join-Path $data 'reactive.sqlite3'
$settings['BRUHS_ATTACHMENTS_DIR'] = Join-Path $data 'attachments'
$settings['BRUHS_RICH_RESPONSE_DIR'] = Join-Path $data 'rich'
$settings['BRUHS_DESKTOP_STATE_FILE'] = Join-Path $data 'state.json'
$settings['BRUHS_CAPTURE_URL'] = 'http://127.0.0.1:9'
$settings['BRUHS_DESKTOP_LOG'] = 'warn'
$settings['RUST_BACKTRACE'] = 'full'

function Capture-Window([int] $ProcessId, [string] $Name) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        $bounds = [RichSmokeWindows]::Bounds($ProcessId)
        $widget = [RichSmokeWindows]::WebView($ProcessId)
        $size = [Drawing.Size]::new($bounds.Right-$bounds.Left, $bounds.Bottom-$bounds.Top)
        $bitmap = [Drawing.Bitmap]::new($size.Width, $size.Height)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $matches = 0
        try {
            $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $size)
            $centerX = [int](($widget.Left+$widget.Right)/2)-$bounds.Left
            $centerY = [int](($widget.Top+$widget.Bottom)/2)-$bounds.Top
            # The actual fixture button has #2f2922 padding around its digit.
            # Native HWND visibility and IPC alone also pass behind an opaque
            # DirectComposition target; require the button's rendered pixels.
            foreach ($offsetX in 12..20) {
                foreach ($offsetY in -2..2) {
                    $pixel = $bitmap.GetPixel($centerX+$offsetX, $centerY+$offsetY)
                    if ([Math]::Abs([int]$pixel.R-0x2f) -le 2 -and
                        [Math]::Abs([int]$pixel.G-0x29) -le 2 -and
                        [Math]::Abs([int]$pixel.B-0x22) -le 2) { $matches++ }
                }
            }
            $bitmap.Save((Join-Path $OutputDirectory "$Name.png"), [Drawing.Imaging.ImageFormat]::Png)
        } finally { $graphics.Dispose(); $bitmap.Dispose() }
        if ($matches -ge 8) { return }
        if ($clock.ElapsedMilliseconds -ge $ApiTimeoutMilliseconds) {
            throw 'The native widget is covered or has not rendered its counter on screen'
        }
        Start-Sleep -Milliseconds $ProbeMilliseconds
    }
}

function Wait-WidgetCount([RichSmokeProcess] $Gui, [string] $Key, [int] $Count) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.ElapsedMilliseconds -lt $ApiTimeoutMilliseconds) {
        if ($Gui.Child.HasExited) { throw 'The GUI exited before state was saved' }
        $value = Invoke-RestMethod "$base/api/agent/rich-response/state?key=$Key" -TimeoutSec ($ApiTimeoutMilliseconds/1000)
        if ($null -ne $value.custom -and [int]$value.custom.count -eq $Count) { return }
        $null = $Gui.Changed.WaitOne($ProbeMilliseconds)
    }
    throw "Pointer input did not durably save counter $Count"
}

$lifecycleErrors = [Collections.Generic.List[string]]::new()
function Invoke-Scene([string] $Scene, [int] $Count, [bool] $Click) {
    $settings['BRUHS_DESKTOP_TERMINAL_SMOKE'] = '1'
    $settings['BRUHS_DESKTOP_SMOKE_SKIP_TERMINAL'] = '1'
    $settings['BRUHS_DESKTOP_SMOKE_HOLD_MS'] = "$ObservationMilliseconds"
    $settings['BRUHS_DESKTOP_SMOKE_AFTER_REPORT_MS'] = "$ObservationMilliseconds"
    $settings['BRUHS_DESKTOP_SMOKE_SCENE'] = "transcript-rich-$Scene"
    $gui = [RichSmokeProcess]::new((Join-Path $BinaryDirectory 'bruhs-desktop.exe'), @(), $BinaryDirectory, $settings, (Join-Path $OutputDirectory "$Scene.log"))
    try {
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $budget = 2*$ObservationMilliseconds + 2*$ApiTimeoutMilliseconds
        $report = $null
        while ($clock.ElapsedMilliseconds -lt $budget) {
            $report = Read-NativeReport $gui.Log $Count
            if ($null -ne $report) { break }
            if ($gui.Child.HasExited) { $gui.Child.WaitForExit(); throw "GUI exited before its native report: $($gui.Log)" }
            $null = $gui.Changed.WaitOne($ProbeMilliseconds)
        }
        if ($null -eq $report) { throw 'The native widget did not become ready within the smoke observation window' }
        [RichSmokeWindows]::Prepare($gui.Child.Id)
        $null = [RichSmokeWindows]::WebView($gui.Child.Id)
        Capture-Window $gui.Child.Id "$Scene-before"
        if ($Click) {
            [RichSmokeWindows]::ClickCounter($gui.Child.Id)
            Wait-WidgetCount $gui $report.Key ($Count+1)
            Capture-Window $gui.Child.Id "$Scene-after-pointer"
        }
        [RichSmokeWindows]::Close($gui.Child.Id)
        $remaining = [int][Math]::Max(0, $budget-$clock.ElapsedMilliseconds)
        if (!$gui.Child.WaitForExit($remaining)) { throw 'The GUI did not exit after WM_CLOSE' }
        $gui.Child.WaitForExit()
        if ($gui.Child.ExitCode -ne 0) { throw "GUI exit code $($gui.Child.ExitCode): $($gui.Log)" }
        if ($gui.Log -match 'panicked at|app_will_quit timed out|Invalid window handle|Class not registered|window not found') { $lifecycleErrors.Add($gui.Log) }
        if ($gui.Log -notmatch 'teardown=(?:app_quit|parent_closed) hosts_disposed=\d+ hosts_remaining=0') {
            throw 'Native host teardown was not confirmed'
        }
        $report
    } finally { $gui.Dispose() }
}

$server = [RichSmokeProcess]::new((Join-Path $BinaryDirectory 'bruhs-server.exe'), @(), $BinaryDirectory, $settings, (Join-Path $OutputDirectory 'server.log'))
try {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $ready = $false
    while ($clock.ElapsedMilliseconds -lt $ApiTimeoutMilliseconds) {
        if ($server.Child.HasExited) { throw "Companion server exited: $($server.Log)" }
        try {
            $null = Invoke-RestMethod "$base/api/agent/rich-response/state?key=$('0'*64)" -TimeoutSec ($ApiTimeoutMilliseconds/1000)
            $ready = $true
            break
        } catch { $null = $server.Changed.WaitOne($ProbeMilliseconds) }
    }
    if (!$ready) { throw "Companion server did not become ready: $($server.Log)" }
    $first = Invoke-Scene 'save' 1 $true
    $second = Invoke-Scene 'reopen' 3 $false
    if ($first.Key -ne $second.Key) { throw 'Reopening did not retain the production widget identity' }
    $result = @{ passed=$true; sandbox=$second.Native.sandbox; parentBlocked=$second.Native.probe.parentBlocked; savedCounter=2; reopenedCounter=3; stateKey=$second.Key; nativePointerInput=$true; normalWindowClose=$true; renderedCounterPixels=$true }
    $result | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $OutputDirectory 'result.json')
    $result | ConvertTo-Json
    foreach ($errorLog in $lifecycleErrors) { Write-Output $errorLog }
    if ($lifecycleErrors.Count -gt 0) { throw 'Native lifecycle diagnostics recorded above' }
} finally { $server.Dispose() }

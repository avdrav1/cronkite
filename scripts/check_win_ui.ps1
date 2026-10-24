# WIN-3 acceptance on Windows: each app is a GUI-subsystem program whose --version still
# prints from a terminal, its windows have Windows' own title bar (minimize, maximize,
# close, snap), its menus are in a menu bar row under it (Help with Update Automatically and
# About, File with Exit), and it follows Windows' light and dark app mode, live. UPD-3: Help's
# Update Automatically is kept in the settings folder, and the first start after an update
# says so once.
#
#   powershell -File scripts\check_win_ui.ps1 [-Bin target\debug] [-Out out\win-ui]
#
# Windows PowerShell 5.1 (its Add-Type builds the helpers below against .NET Framework).
# Runs on GitHub's Windows runner (ci.yml's windows job) with MSYS2's GTK on PATH and
# GSK_RENDERER=cairo (the runner has no GPU). For each app:
#   - the exe's PE header says GUI subsystem (2), and no console process starts with it;
#   - `--version` prints `<app> <version>` through a pipe, into a file redirected by cmd,
#     and into a console window of its own (a PowerShell started in a new console runs it
#     and reads its console's text back);
#   - its window (on a sample file) has WS_CAPTION, WS_SYSMENU, WS_MINIMIZEBOX,
#     WS_MAXIMIZEBOX, and WS_THICKFRAME, a system title bar above its client area, and
#     snaps to the left half with Win+Left;
#   - F10 then Down opens File (a popup window more), Left moves to Help, whose first item,
#     Update Automatically, Enter turns off (the settings folder's `update` says
#     automatic=false); Help again, Down and Enter open About (a window more), and File's
#     last item (Up from the top) exits the app;
#   - UPD-3: with an `applied` record naming this version in its updates folder, the first
#     start logs "update: Updated to <App> X.Y.Z" (the window screenshot shows the notice)
#     and removes the record; the next start (dark) doesn't, and Help's Update
#     Automatically, unchecked there (screenshot), turns it back on;
#   - with AppsUseLightTheme switched to 0 while it runs, its title bar and menu bar row
#     turn dark within a few seconds, and light again with 1; started in dark mode, it's
#     dark from the start.
# Screenshots land in $Out\shots, logs in $Out\logs, the result lines in $Out\summary.txt;
# a failure exits 1 after every app has run.
param(
    [string]$Bin = "target\debug",
    [string]$Out = "out\win-ui"
)
$ErrorActionPreference = "Continue"
$root = (Resolve-Path "$PSScriptRoot\..").Path
$Bin = (Resolve-Path $Bin).Path
foreach ($d in "shots", "logs", "files") { New-Item -ItemType Directory -Force "$Out\$d" | Out-Null }
$Out = (Resolve-Path $Out).Path
$shots = "$Out\shots"; $logs = "$Out\logs"; $files = "$Out\files"
$summary = "$Out\summary.txt"
Set-Content -Encoding utf8 $summary ""
$script:failed = $false
function Note([string]$line) { Add-Content -Encoding utf8 $summary $line; Write-Host $line }
function Pass([string]$what) { Note "ok   $what" }
function Fail([string]$what) { Note "FAIL $what"; $script:failed = $true }
function Check([bool]$ok, [string]$what) { if ($ok) { Pass $what } else { Fail $what } }

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int index);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, System.Text.StringBuilder text, int max);

    // The visible windows of process `pid`.
    public static List<IntPtr> Windows(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows((h, p) => {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner == pid && IsWindowVisible(h)) found.Add(h);
            return true;
        }, IntPtr.Zero);
        return found;
    }

    // The title of window `h`.
    public static string Title(IntPtr h) {
        var text = new System.Text.StringBuilder(256);
        GetWindowText(h, text, text.Capacity);
        return text.ToString();
    }

    // The window's visible bounds (without the invisible resize borders).
    public static RECT Bounds(IntPtr h) {
        RECT r;
        DwmGetWindowAttribute(h, 9 /* DWMWA_EXTENDED_FRAME_BOUNDS */, out r, Marshal.SizeOf(typeof(RECT)));
        return r;
    }

    // The client area's top left corner on the screen.
    public static POINT ClientOrigin(IntPtr h) {
        var p = new POINT();
        ClientToScreen(h, ref p);
        return p;
    }

    // Key `vk` down and up, with `mods` held (virtual keys). Arrows are extended keys.
    public static void Keys(byte[] mods, byte vk) {
        uint ext = (vk >= 0x21 && vk <= 0x28) ? 1u : 0u;
        foreach (var m in mods) keybd_event(m, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, ext, UIntPtr.Zero);
        keybd_event(vk, 0, ext | 2 /* up */, UIntPtr.Zero);
        for (int i = mods.Length - 1; i >= 0; i--) keybd_event(mods[i], 0, 2, UIntPtr.Zero);
    }

    // A left click at screen point (x, y).
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        mouse_event(2 /* left down */, 0, 0, 0, UIntPtr.Zero);
        mouse_event(4 /* left up */, 0, 0, 0, UIntPtr.Zero);
    }

    // Bring `h` to the front: an Alt press first lets this process set the foreground.
    public static bool Front(IntPtr h) {
        ShowWindow(h, 9 /* SW_RESTORE */);
        keybd_event(0x12, 0, 0, UIntPtr.Zero);
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        SetForegroundWindow(h);
        return GetForegroundWindow() == h;
    }

    // The mean brightness (0-255) of the screen's rectangle (x, y, w, h).
    public static double Brightness(int x, int y, int w, int h) {
        using (var bmp = new Bitmap(w, h))
        using (var g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(x, y, 0, 0, new Size(w, h));
            double sum = 0;
            for (int j = 0; j < h; j++)
                for (int i = 0; i < w; i++) {
                    var c = bmp.GetPixel(i, j);
                    sum += 0.299 * c.R + 0.587 * c.G + 0.114 * c.B;
                }
            return sum / (w * h);
        }
    }

    // The screen's rectangle into a PNG.
    public static void Shot(int x, int y, int w, int h, string file) {
        using (var bmp = new Bitmap(w, h))
        using (var g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(x, y, 0, 0, new Size(w, h));
            bmp.Save(file, System.Drawing.Imaging.ImageFormat.Png);
        }
    }
}
"@

$VK_CONTROL = 0x11; $VK_LWIN = 0x5B
$VK = @{ F10 = 0x79; Down = 0x28; Up = 0x26; Left = 0x25; Return = 0x0D; Escape = 0x1B }
function Press([string]$key) { [Win]::Keys(@(), $VK[$key]); Start-Sleep -Milliseconds 700 }

function DesktopShot([string]$name) {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    [Win]::Shot($b.X, $b.Y, $b.Width, $b.Height, "$shots\$name.png")
}

function WindowShot([IntPtr]$h, [string]$name) {
    $r = [Win]::Bounds($h)
    if ($r.Right -gt $r.Left -and $r.Bottom -gt $r.Top) {
        [Win]::Shot($r.Left, $r.Top, $r.Right - $r.Left, $r.Bottom - $r.Top, "$shots\$name.png")
    }
}

# The brightness of the title bar (a strip in its middle) and of the menu bar row (right
# of the menus' names) of window $h.
function Looks([IntPtr]$h) {
    $r = [Win]::Bounds($h)
    $c = [Win]::ClientOrigin($h)
    $w = $r.Right - $r.Left
    $caption = $c.Y - $r.Top
    $title = [Win]::Brightness($r.Left + [int]($w * 0.55), $r.Top + 4, [int]($w * 0.15), [math]::Max(4, $caption - 8))
    $menu = [Win]::Brightness($c.X + [int]($w * 0.6), $c.Y + 4, [int]($w * 0.3), 14)
    return @{ title = [math]::Round($title); menu = [math]::Round($menu) }
}

function SetDark([bool]$dark) {
    $key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
    if (-not (Test-Path $key)) { New-Item -Force $key | Out-Null }
    New-ItemProperty -Force -Path $key -Name AppsUseLightTheme -PropertyType DWord -Value ([int](-not $dark)) | Out-Null
}

# Start $app on $file; its process once its window shows, else $null.
function Start-App([string]$app, [string]$file, [string]$tag) {
    $p = Start-Process -FilePath "$Bin\$app.exe" -ArgumentList "`"$file`"" -PassThru `
        -RedirectStandardOutput "$logs\$tag.out.txt" -RedirectStandardError "$logs\$tag.err.txt"
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline -and -not $p.HasExited) {
        Start-Sleep -Milliseconds 200
        $p.Refresh()
        if ($p.MainWindowHandle -ne 0) { break }
    }
    if ($p.HasExited -or $p.MainWindowHandle -eq 0) {
        Fail "${tag}: no window (exited: $($p.HasExited))"
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
        return $null
    }
    # Let it lay out and draw its file.
    Start-Sleep -Seconds 3
    return $p
}

function Stop-App($p) {
    if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force; Start-Sleep -Milliseconds 500 }
}

# Samples.
Set-Content -Encoding ascii "$files\people.csv" "id,name,city`n1,Ada,Paris`n2,Linus,Austin`n"
Copy-Item "$root\apps\slate\crates\docx\tests\data\fixture.docx" "$files\letter.docx"
Copy-Item "$root\apps\frame\crates\pptx\tests\data\fixture.pptx" "$files\deck.pptx"
$bmp = New-Object System.Drawing.Bitmap 640, 400
$g = [System.Drawing.Graphics]::FromImage($bmp)
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Point 0, 0), (New-Object System.Drawing.Point 640, 400), ([System.Drawing.Color]::Navy), ([System.Drawing.Color]::Gold)
$g.FillRectangle($brush, 0, 0, 640, 400)
$bmp.Save("$files\picture.png", [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
$samples = [ordered]@{
    brick = "$files\people.csv"
    slate = "$files\letter.docx"
    frame = "$files\deck.pptx"
    etch  = "$files\picture.png"
}

$s = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Note "screen $($s.Width)x$($s.Height); GSK_RENDERER=$env:GSK_RENDERER; GTK_CSD=$env:GTK_CSD"
# An app's version: its Cargo.toml's, else the workspace's.
function Version([string]$app) {
    foreach ($toml in "$root\apps\$app\Cargo.toml", "$root\Cargo.toml") {
        $m = Select-String -Path $toml -Pattern '^version = "(.+)"' | Select-Object -First 1
        if ($m) { return $m.Matches[0].Groups[1].Value }
    }
}

# The console helper: run the exe in this (new) console, then write the console's text.
$helper = "$Out\console_version.ps1"
Set-Content -Encoding utf8 $helper @'
param([string]$Exe, [string]$Into)
& $Exe --version
Start-Sleep -Seconds 3
$raw = $Host.UI.RawUI
$width = $raw.BufferSize.Width
$rows = $raw.CursorPosition.Y + 2
$cells = $raw.GetBufferContents((New-Object System.Management.Automation.Host.Rectangle 0, 0, ($width - 1), $rows))
$text = for ($y = 0; $y -lt $cells.GetLength(0); $y++) {
    (-join (0..($width - 1) | ForEach-Object { $cells[$y, $_].Character })).TrimEnd()
}
Set-Content -Encoding utf8 $Into ($text -join "`n")
'@

SetDark $false
foreach ($app in $samples.Keys) {
    $exe = "$Bin\$app.exe"
    if (-not (Test-Path $exe)) { Fail "${app}: no $exe"; continue }

    # GUI subsystem: the optional header's Subsystem field (offset 68) is 2.
    $stream = [IO.File]::OpenRead($exe)
    $head = New-Object byte[] 4096
    [void]$stream.Read($head, 0, $head.Length)
    $stream.Close()
    $pe = [BitConverter]::ToInt32($head, 0x3C)
    $subsystem = [BitConverter]::ToUInt16($head, $pe + 24 + 68)
    Check ($subsystem -eq 2) "${app}: GUI-subsystem exe (PE subsystem $subsystem)"

    # --version: a pipe, a file redirected by cmd, and a console of its own.
    $want = "$app $(Version $app)"
    $piped = (& $exe --version 2>&1 | Out-String).Trim()
    Check ($piped -eq $want) "${app}: --version through a pipe prints '$piped'"
    $redirected = "$logs\$app-version-cmd.txt"
    cmd /c "`"$exe`" --version > `"$redirected`""
    for ($i = 0; $i -lt 20 -and -not ((Test-Path $redirected) -and (Get-Item $redirected).Length -gt 0); $i++) { Start-Sleep -Milliseconds 250 }
    $text = if (Test-Path $redirected) { (Get-Content -Raw $redirected).Trim() } else { "" }
    Check ($text -eq $want) "${app}: --version redirected by cmd writes '$text'"
    $console = "$logs\$app-version-console.txt"
    Start-Process -Wait -FilePath powershell.exe -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$helper`"", "`"$exe`"", "`"$console`"")
    $text = if (Test-Path $console) { Get-Content -Raw $console } else { "" }
    Check ($text -match [regex]::Escape($want)) "${app}: --version in a console window shows '$want' ($logs\$app-version-console.txt)"

    # UPD-3: no setting yet, and the record an applied update leaves.
    $title = (Get-Culture).TextInfo.ToTitleCase($app)
    $settings = "$env:APPDATA\Cronkite\$title\update"
    $updates = "$env:LOCALAPPDATA\Cronkite\$title\updates"
    Remove-Item -Force -ErrorAction SilentlyContinue $settings
    New-Item -ItemType Directory -Force $updates | Out-Null
    Set-Content -Encoding ascii "$updates\applied" "version=$(Version $app)"

    # The window, light.
    $p = Start-App $app $samples[$app] "$app-light"
    if (-not $p) { continue }
    $notice = "update: Updated to $title $(Version $app)"
    $log = Get-Content -ErrorAction SilentlyContinue "$logs\$app-light.err.txt"
    Check ($log -contains $notice) "${app}: the first start after an update logs '$notice' ($shots\$app-1-window-light.png)"
    Check (-not (Test-Path "$updates\applied")) "${app}: the applied record is gone"
    $h = $p.MainWindowHandle
    $consoles = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($p.Id)" | Where-Object { $_.Name -in "conhost.exe", "OpenConsole.exe", "WindowsTerminal.exe" })
    Check ($consoles.Count -eq 0) "${app}: no console process with it ($($consoles.Name -join ','))"
    $front = [Win]::Front($h)
    Start-Sleep -Milliseconds 700
    $style = [Win]::GetWindowLongPtr($h, -16).ToInt64()
    $need = @{ WS_CAPTION = 0x00C00000; WS_SYSMENU = 0x00080000; WS_THICKFRAME = 0x00040000; WS_MINIMIZEBOX = 0x00020000; WS_MAXIMIZEBOX = 0x00010000 }
    $missing = @($need.Keys | Where-Object { ($style -band $need[$_]) -ne $need[$_] })
    Check ($missing.Count -eq 0) ("${app}: Windows' title bar: style 0x{0:X8}, missing [{1}]" -f $style, ($missing -join ","))
    $r = [Win]::Bounds($h); $c = [Win]::ClientOrigin($h)
    $caption = $c.Y - $r.Top
    Check ($caption -ge 20) "${app}: the system's title bar is $caption px above the client area"
    Note "     ${app}: foreground $front; title '$($p.MainWindowTitle)'"
    WindowShot $h "$app-1-window-light"
    $looks = Looks $h
    Check ($looks.title -gt 150 -and $looks.menu -gt 150) "${app}: light: title bar $($looks.title), menu bar row $($looks.menu)"

    # The menus: File, then Help (Left from File wraps to the last menu), then About.
    $before = ([Win]::Windows($p.Id)).Count
    [void][Win]::Front($h)
    Press F10; Press Down
    $open = ([Win]::Windows($p.Id)).Count
    Check ($open -gt $before) "${app}: F10, Down opens File ($before -> $open windows)"
    if ($open -le $before) {
        # The keys didn't reach it: click File, the menu bar row's first name.
        $c = [Win]::ClientOrigin($h)
        [Win]::Click($c.X + 18, $c.Y + 12)
        Start-Sleep -Milliseconds 700
        $open = ([Win]::Windows($p.Id)).Count
        Check ($open -gt $before) "${app}: a click opens File ($before -> $open windows)"
    }
    DesktopShot "$app-2-file-menu"
    Press Left
    DesktopShot "$app-3-help-menu"
    # Help's first item, Update Automatically, has the focus: Enter turns it off.
    Press Return
    Start-Sleep -Milliseconds 500
    $kept = if (Test-Path $settings) { (Get-Content -Raw $settings).Trim() } else { "" }
    Check ($kept -eq "automatic=false") "${app}: Help -> Update Automatically keeps automatic=false in $settings ('$kept')"
    # Help again; About is next.
    [void][Win]::Front($h)
    Press F10; Press Down; Press Left; Press Down
    Press Return
    $about = $null
    for ($i = 0; $i -lt 20 -and -not $about; $i++) {
        Start-Sleep -Milliseconds 250
        $about = [Win]::Windows($p.Id) | Where-Object { [Win]::Title($_) -eq "About $title" } | Select-Object -First 1
    }
    Start-Sleep -Milliseconds 500
    DesktopShot "$app-4-about"
    $titles = ([Win]::Windows($p.Id) | ForEach-Object { "'" + [Win]::Title($_) + "'" }) -join ", "
    Check ($null -ne $about) "${app}: Help -> About opens 'About $title' (windows: $titles)"
    if ($about) { [void][Win]::Front($about); Press Escape }

    # Live: dark, then light again.
    SetDark $true
    Start-Sleep -Seconds 3
    [void][Win]::Front($h); Start-Sleep -Milliseconds 700
    WindowShot $h "$app-5-window-dark-live"
    $looks = Looks $h
    Check ($looks.title -lt 100 -and $looks.menu -lt 100) "${app}: turns dark live: title bar $($looks.title), menu bar row $($looks.menu)"
    [void][Win]::Front($h); Press F10; Press Down
    DesktopShot "$app-6-file-menu-dark"
    Press Escape; Press Escape
    SetDark $false
    Start-Sleep -Seconds 3
    [void][Win]::Front($h); Start-Sleep -Milliseconds 700
    WindowShot $h "$app-7-window-light-again"
    $looks = Looks $h
    Check ($looks.title -gt 150 -and $looks.menu -gt 150) "${app}: turns light again: title bar $($looks.title), menu bar row $($looks.menu)"

    # Snap: Win+Left puts it on the left of the screen at the work area's full height, half
    # its width unless the window's narrowest is wider.
    [void][Win]::Front($h); Start-Sleep -Milliseconds 500
    [Win]::Keys(@($VK_LWIN), $VK.Left)
    Start-Sleep -Seconds 1
    Press Escape
    $r = [Win]::Bounds($h)
    $work = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    DesktopShot "$app-8-snapped"
    $left = [math]::Abs($r.Left - $work.Left) -le 8 -and [math]::Abs($r.Top - $work.Top) -le 8 -and [math]::Abs($r.Bottom - $work.Bottom) -le 8
    $half = [math]::Abs(($r.Right - $r.Left) - $work.Width / 2) -le 16
    Check $left "${app}: Win+Left snaps it to the left ($($r.Left),$($r.Top),$($r.Right),$($r.Bottom) in $($work.Width)x$($work.Height); half the width: $half)"

    # File's last item, Exit, ends the app (no changes to ask about).
    [void][Win]::Front($h)
    Press F10; Press Down; Press Up
    DesktopShot "$app-9-exit"
    Press Return
    $p.WaitForExit(10000) | Out-Null
    Check $p.HasExited "${app}: File -> Exit ends it"
    Stop-App $p

    # Started in dark mode.
    SetDark $true
    $p = Start-App $app $samples[$app] "$app-dark"
    if ($p) {
        [void][Win]::Front($p.MainWindowHandle); Start-Sleep -Milliseconds 700
        WindowShot $p.MainWindowHandle "$app-10-window-dark-at-start"
        $looks = Looks $p.MainWindowHandle
        Check ($looks.title -lt 100 -and $looks.menu -lt 100) "${app}: dark at start: title bar $($looks.title), menu bar row $($looks.menu)"
        # UPD-3: the next start shows no notice; Update Automatically is still off (the
        # screenshot shows it unchecked), and Enter turns it back on.
        $log = Get-Content -ErrorAction SilentlyContinue "$logs\$app-dark.err.txt"
        Check (-not ($log -match "^update: Updated to")) "${app}: the next start shows no notice"
        [void][Win]::Front($p.MainWindowHandle)
        Press F10; Press Down; Press Left
        DesktopShot "$app-11-help-menu-restarted"
        Press Return
        Start-Sleep -Milliseconds 500
        $kept = if (Test-Path $settings) { (Get-Content -Raw $settings).Trim() } else { "" }
        Check ($kept -eq "automatic=true") "${app}: after a restart, Help -> Update Automatically turns it back on ('$kept')"
        Stop-App $p
    }
    SetDark $false
}

if ($script:failed) { Note "check_win_ui: FAILED"; exit 1 }
Note "check_win_ui: OK"
exit 0

# UPD-7 on Windows: an installed copy of a live release updates itself from the live
# latest.json, in the wild (ADR 0012). On a clean Windows runner (update-live.yml's windows
# job, MSYS2 moved away):
#
#   python scripts\pin_release.py SITE brick:0.6.0 ...   # the site the installers read
#   powershell -File scripts\check_win_update_live.ps1 -Site SITE [-Apps "brick slate frame etch"] [-ExpectUpdate 1] [-Out out\update-live-win]
#
# Windows PowerShell 5.1 on an interactive desktop (GitHub's windows-latest). Per app:
#   1. the app's real installer (https://<app>.cronkite.cc/install.ps1, as SITE\<app> holds
#      it) installs the live release FROM named by SITE\<app>\latest.json, served on
#      127.0.0.1 (APP_DOWNLOAD_URL; the installer checks Setup.exe's sha256): Settings >
#      Apps and `--version` are recorded;
#   2. the installed program starts as Explorer starts it (PATH of Windows' own folders), its
#      stderr kept, with no CRONKITE_UPDATE_URL and no test key: it checks
#      https://dl.cronkite.cc's latest.json, signed with a release key;
#   with -ExpectUpdate 1 (the default), the live latest must be newer than FROM, and:
#   3. while FROM runs it logs "update: <App> LATEST downloaded for the next start" and
#      stages LATEST's Setup.exe (screenshot); closing its window (the title bar's button)
#      starts Setup ("update: installing <App> LATEST"), which runs on after it exits;
#   4. Settings > Apps shows LATEST once Setup has finished, `--version` is LATEST
#      (recorded), updates\applied names it;
#   5. the next start logs "update: Updated to <App> LATEST" as its window shows the banner
#      (screenshot);
#   with -ExpectUpdate 0 the live latest must be FROM itself (before the next release), and:
#   3. the check runs to its end (updates\.lock taken, then released) without a problem
#      logged and stages nothing: up to date (screenshot); after closing, Settings > Apps
#      and `--version` still say FROM.
# Screenshots land in $Out\shots, logs in $Out\logs, the results and recorded versions in
# $Out\summary.txt; it ends with "check_win_update_live: OK (N checks)" or exits 1 after
# every app ran.
param(
    [Parameter(Mandatory = $true)][string]$Site,
    [string]$Apps = "brick slate frame etch",
    [ValidateSet("0", "1")][string]$ExpectUpdate = "1",
    [string]$Out = "out\update-live-win"
)
$ErrorActionPreference = "Continue"
$pinned = (Resolve-Path $Site).Path
foreach ($d in "shots", "logs", "files") { New-Item -ItemType Directory -Force "$Out\$d" | Out-Null }
$Out = (Resolve-Path $Out).Path
$shots = "$Out\shots"; $logs = "$Out\logs"; $files = "$Out\files"
$summary = "$Out\summary.txt"
Set-Content -Encoding utf8 $summary ""
$script:failed = $false
$script:oks = 0
function Note([string]$line) { Add-Content -Encoding utf8 $summary $line; Write-Host $line }
function Pass([string]$what) { $script:oks++; Note "ok   $what" }
function Fail([string]$what) { Note "FAIL $what"; $script:failed = $true }
function Check([bool]$ok, [string]$what) { if ($ok) { Pass $what } else { Fail $what } }

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;

public static class Win {
    delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    // Visible top-level windows with a title, as "handle<TAB>pid<TAB>title"; of process
    // `pid`, or of every process when it's 0.
    public static string[] Windows(int pid) {
        var found = new List<string>();
        EnumWindows(delegate (IntPtr h, IntPtr p) {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if ((pid == 0 || owner == (uint)pid) && IsWindowVisible(h)) {
                var s = new StringBuilder(512);
                GetWindowTextW(h, s, 512);
                if (s.Length > 0) found.Add(h.ToInt64() + "\t" + owner + "\t" + s);
            }
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    public static void Front(long h) {
        ShowWindow(new IntPtr(h), 9);
        keybd_event(0x12, 0, 0, UIntPtr.Zero);
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        SetForegroundWindow(new IntPtr(h));
    }

    // WM_CLOSE, as the title bar's close button sends.
    public static void Close(long h) { PostMessageW(new IntPtr(h), 0x10, IntPtr.Zero, IntPtr.Zero); }

    public static void Shot(long h, string file) {
        RECT r;
        DwmGetWindowAttribute(new IntPtr(h), 9, out r, Marshal.SizeOf(typeof(RECT)));
        Rectangle screen = System.Windows.Forms.Screen.PrimaryScreen.Bounds;
        int x = Math.Max(r.Left, screen.Left), y = Math.Max(r.Top, screen.Top);
        int w = Math.Min(r.Right, screen.Right) - x, hh = Math.Min(r.Bottom, screen.Bottom) - y;
        if (w <= 0 || hh <= 0) return;
        using (var bmp = new Bitmap(w, hh))
        using (var g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(x, y, 0, 0, new Size(w, hh));
            bmp.Save(file, System.Drawing.Imaging.ImageFormat.Png);
        }
    }

    public static void Screen(string file) {
        Rectangle b = System.Windows.Forms.Screen.PrimaryScreen.Bounds;
        using (var bmp = new Bitmap(b.Width, b.Height))
        using (var g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(b.Location, Point.Empty, b.Size);
            bmp.Save(file, System.Drawing.Imaging.ImageFormat.Png);
        }
    }
}
"@

$Names = @{ brick = "Brick"; slate = "Slate"; frame = "Frame"; etch = "Etch" }

# --- Setup -----------------------------------------------------------------------------------
# As Explorer starts programs, with nothing of MSYS2 anywhere and no update overrides.
$env:PATH = "$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\Wbem;$env:SystemRoot\System32\WindowsPowerShell\v1.0\"
foreach ($v in "HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_DATA_DIRS",
    "GSK_RENDERER", "PANGOCAIRO_BACKEND", "GTK_DATA_PREFIX", "GDK_PIXBUF_MODULE_FILE", "FONTCONFIG_FILE", "MSYSTEM",
    "CRONKITE_NO_UPDATE", "CRONKITE_UPDATE_URL") {
    Remove-Item "env:$v" -ErrorAction SilentlyContinue
}
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
Note "UPD-7 in the wild, Windows $([Environment]::OSVersion.Version), $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mmZ')), ExpectUpdate $ExpectUpdate"
$msys = @(Get-Command gcc, pacman, cygpath, ntldd -ErrorAction SilentlyContinue)
Check (($msys.Count -eq 0) -and -not (Test-Path C:\msys64)) "no MSYS2 on PATH or in C:\msys64 ($($msys.Count) of its programs found)"

# The pinned site, for the installers only.
$port = 8765
$base = "http://127.0.0.1:$port"
$server = Start-Job -ArgumentList $pinned, $port -ScriptBlock {
    param($root, $port)
    $l = New-Object Net.HttpListener
    $l.Prefixes.Add("http://127.0.0.1:$port/")
    $l.Start()
    while ($l.IsListening) {
        $c = $l.GetContext()
        $rel = [Uri]::UnescapeDataString($c.Request.Url.AbsolutePath.TrimStart('/')).Replace('/', '\')
        $path = Join-Path $root $rel
        if (Test-Path -PathType Leaf $path) {
            $c.Response.ContentType = switch ([IO.Path]::GetExtension($path)) {
                ".json" { "application/json" } ".ps1" { "text/plain; charset=utf-8" } default { "application/octet-stream" }
            }
            $b = [IO.File]::ReadAllBytes($path)
            $c.Response.ContentLength64 = $b.Length
            try { $c.Response.OutputStream.Write($b, 0, $b.Length) } catch {}
        } else { $c.Response.StatusCode = 404 }
        try { $c.Response.Close() } catch {}
    }
}
$first = ($Apps -split '\s+' | Where-Object { $_ } | Select-Object -First 1)
$deadline = (Get-Date).AddSeconds(30)
do {
    try { $up = (Invoke-WebRequest -UseBasicParsing "$base/$first/latest.json").StatusCode -eq 200 } catch { $up = $false }
    if (-not $up) { Start-Sleep -Milliseconds 500 }
} while (-not $up -and (Get-Date) -lt $deadline)
Check $up "the pinned site answers on $base"

# --- Helpers ---------------------------------------------------------------------------------
function Live([string]$app) { if ($app -eq "brick") { "https://dl.cronkite.cc" } else { "https://dl.cronkite.cc/$app" } }
function Uninstall-Key([string]$name) { "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1" }
function Installed([string]$name) { (Get-ItemProperty (Uninstall-Key $name) -ErrorAction SilentlyContinue).DisplayVersion }
function Version([string]$exe) { if (Test-Path $exe) { (& $exe --version 2>&1 | Out-String).Trim() } else { "(no $exe)" } }

# A log file the app is still writing, read without disturbing it.
function Read-Log([string]$file) {
    try {
        $s = [IO.File]::Open($file, 'Open', 'Read', 'ReadWrite')
        try { return (New-Object IO.StreamReader($s)).ReadToEnd() } finally { $s.Dispose() }
    } catch { return "" }
}
function Wait-Log([string]$file, [string]$pattern, [int]$seconds = 120) {
    $deadline = (Get-Date).AddSeconds($seconds)
    do {
        if ((Read-Log $file) -match $pattern) { return $true }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    return $false
}
function Log-Lines([string]$file) {
    ((Read-Log $file) -split "`r?`n" | Where-Object { $_ -match 'update: ' } | ForEach-Object { ($_ -replace '^.*?update: ', 'update: ').Trim() }) -join ' | '
}

# The installed app, started as Explorer starts it, on $doc (none: Frame's new deck); its
# stderr in $logs\$tag.err.txt. Returns the process and its window (0 if none showed).
function Start-App([string]$exe, [string]$tag, [string]$doc) {
    $how = @{ FilePath = $exe; WorkingDirectory = $files; PassThru = $true
        RedirectStandardOutput = "$logs\$tag.out.txt"; RedirectStandardError = "$logs\$tag.err.txt" }
    if ($doc) { $how.ArgumentList = "`"$doc`"" }
    $p = Start-Process @how
    $h = 0
    $deadline = (Get-Date).AddSeconds(90)
    do {
        $w = @([Win]::Windows($p.Id)) | Select-Object -First 1
        if ($w) { $h = [long](($w -split "`t")[0]) } else { Start-Sleep -Milliseconds 250 }
    } while (-not $h -and -not $p.HasExited -and (Get-Date) -lt $deadline)
    if (-not $h) { [Win]::Screen("$shots\$tag-no-window.png") }
    return @{ p = $p; h = $h; err = "$logs\$tag.err.txt"; tag = $tag }
}

# Close its window as the title bar's button does; true once the process has exited.
function Quit($run, [int]$seconds = 60) {
    if ($run.h) { [Win]::Close($run.h) }
    return $run.p.WaitForExit($seconds * 1000)
}

# Inno Setup processes of this app's Setup.exe (the exe and its setup.tmp).
function Setups([string]$app) {
    @(Get-CimInstance Win32_Process -Filter "Name LIKE '$app-%-windows-setup.%'" -ErrorAction SilentlyContinue)
}

function Staged([string]$updates) {
    $record = "$updates\staged"
    if (-not (Test-Path $record)) { return $null }
    $f = @{}
    foreach ($l in Get-Content $record) { $k, $v = $l -split '=', 2; $f[$k] = $v }
    return $f
}

function Shot-Window([long]$h, [string]$name) {
    [Win]::Front($h)
    Start-Sleep -Milliseconds 800
    [Win]::Shot($h, "$shots\$name.png")
}

# The check ran to its end: it takes updates\.lock (made then; LockFileEx over the whole
# file) for as long as it runs. Locking a byte of it succeeds only once it's released.
function Finished([string]$updates) {
    $lock = "$updates\.lock"
    $deadline = (Get-Date).AddSeconds(60)
    while (-not (Test-Path $lock) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    if (-not (Test-Path $lock)) { return $false }
    Start-Sleep -Seconds 2
    $deadline = (Get-Date).AddSeconds(120)
    do {
        try {
            $s = [IO.File]::Open($lock, 'Open', 'ReadWrite', 'ReadWrite')
            try { $s.Lock(0, 1); $s.Unlock(0, 1); return $true } catch {} finally { $s.Dispose() }
        } catch {}
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    return $false
}

# Files to start each app on.
Set-Content -Encoding ascii "$files\people.csv" "id,name,city`r`n1,Ada,Paris`r`n2,Linus,Austin"
Set-Content -Encoding ascii "$files\readme.md" "# Read me`r`n`r`nUpdated by itself."
$wide = New-Object System.Drawing.Bitmap 64, 32
$g = [System.Drawing.Graphics]::FromImage($wide)
$g.FillRectangle([System.Drawing.Brushes]::Firebrick, 0, 0, 32, 32)
$g.FillRectangle([System.Drawing.Brushes]::RoyalBlue, 32, 0, 32, 32)
$g.Dispose()
$wide.Save("$files\wide.png", [System.Drawing.Imaging.ImageFormat]::Png)
$wide.Dispose()
$StartOn = @{ brick = "$files\people.csv"; slate = "$files\readme.md"; frame = ""; etch = "$files\wide.png" }

# --- Each app --------------------------------------------------------------------------------
foreach ($app in $Apps -split '\s+' | Where-Object { $_ }) {
    if (-not $Names.ContainsKey($app)) { Fail "unknown app $app"; continue }
    $name = $Names[$app]
    $dir = "$env:LOCALAPPDATA\Programs\$name"
    $exe = "$dir\bin\$app.exe"
    $updates = "$env:LOCALAPPDATA\Cronkite\$name\updates"
    try { $from = (Get-Content -Raw "$pinned\$app\latest.json" | ConvertFrom-Json).version } catch { $from = $null }
    try { $latest = (Invoke-RestMethod -UseBasicParsing "$(Live $app)/latest.json").version } catch { $latest = $null }
    if (-not $from -or -not $latest) { Fail "${app}: pinned FROM ($from) or live latest ($latest) missing"; continue }
    Note "== $name FROM $from, live latest $latest"
    if ($ExpectUpdate -eq "1" -and $latest -eq $from) { Fail "${app}: no newer release live: $(Live $app)/latest.json is $from (-ExpectUpdate 0 checks the up-to-date path)"; continue }
    if ($ExpectUpdate -eq "0" -and $latest -ne $from) { Fail "${app}: $(Live $app)/latest.json is $latest, not ${from}: -ExpectUpdate 0 expects FROM to be the live latest"; continue }
    $doc = $StartOn[$app]

    # 1. FROM, through the real install.ps1 reading the pinned latest.json.
    Remove-Item -Recurse -Force "$env:LOCALAPPDATA\Cronkite\$name" -ErrorAction SilentlyContinue
    Set-Item "env:$($app.ToUpperInvariant())_DOWNLOAD_URL" "$base/$app"
    $text = & powershell.exe -NoProfile -Command "irm $base/$app/install.ps1 | iex" 2>&1 | Out-String
    Remove-Item "env:$($app.ToUpperInvariant())_DOWNLOAD_URL"
    Set-Content -Encoding utf8 "$logs\$app-install.txt" $text
    $v = Version $exe
    Note "${app}: --version before: $v; Settings > Apps: $(Installed $name)"
    Check ($text -match "(?m)^(Installed|Updated) $name .*$([regex]::Escape($from))\r?$" -and (Installed $name) -eq $from -and $v -eq "$app $from") "${app}: https://$app.cronkite.cc/install.ps1 installs the live $name $from (Settings > Apps: $(Installed $name); --version: $v)"

    # 2. Started, it checks the live latest.json.
    $run = Start-App $exe "$app-1-$from" $doc
    Check ($run.h -ne 0) "${app}: $name $from starts with a window"
    if ($ExpectUpdate -eq "1") {
        $said = Wait-Log $run.err "update: $name $([regex]::Escape($latest)) downloaded for the next start" 300
        $s = Staged $updates
        Check ($said -and $s -and $s.version -eq $latest) "${app}: the running $from downloaded and staged the live $latest ($($s.file)): '$(Log-Lines $run.err)'"
        if ($run.h) { Shot-Window $run.h "$app-1-$from-updating" }
        $t0 = Get-Date
        $quit = Quit $run
        $installing = Wait-Log $run.err "update: installing $name $([regex]::Escape($latest))" 10
        Check ($quit -and $installing) "${app}: closing $name starts Setup: '$(Log-Lines $run.err)'"
        $deadline = (Get-Date).AddSeconds(300)
        while ((Installed $name) -ne $latest -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
        $deadline = (Get-Date).AddSeconds(60)
        while ((Setups $app).Count -gt 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
        $secs = [math]::Round(((Get-Date) - $t0).TotalSeconds)
        Copy-Item "$updates\setup.log" "$logs\$app-setup.log" -ErrorAction SilentlyContinue
        $v = Version $exe
        Note "${app}: --version after: $v; Settings > Apps: $(Installed $name)"
        Check ((Installed $name) -eq $latest -and $v -eq "$app $latest") "${app}: Settings > Apps shows $name $(Installed $name) $secs s after closing it; --version: $v"
        $applied = Get-Content "$updates\applied" -ErrorAction SilentlyContinue
        Check ($applied -contains "version=$latest") "${app}: updates\applied names $latest ($applied)"

        # The next start shows the notice.
        $run = Start-App $exe "$app-2-$latest" $doc
        $notice = Wait-Log $run.err "update: Updated to $name $([regex]::Escape($latest))" 30
        if ($run.h) { Start-Sleep -Seconds 1; Shot-Window $run.h "$app-2-$latest-updated" }
        Check ($run.h -and $notice) "${app}: the next start says '$(Log-Lines $run.err)' (its window: $app-2-$latest-updated.png)"
        $quit = Quit $run
        Check $quit "${app}: $name $latest quits"
    } else {
        $done = Finished $updates
        Check $done "${app}: the check of the live latest.json ran to its end (updates\.lock taken, released)"
        $lines = Log-Lines $run.err
        Check (-not $lines) "${app}: it logged no problem: latest.json and its signature verified, nothing newer$(if ($lines) { ': ' + $lines })"
        # Nothing in updates but the check's .lock and the running instances' owner-PID.lock.
        $left = @(Get-ChildItem -Name -Force $updates -ErrorAction SilentlyContinue | Where-Object { $_ -ne ".lock" -and $_ -notmatch '^owner-\d+\.lock$' })
        Check (-not (Staged $updates) -and $left.Count -eq 0) "${app}: nothing staged: $from is the live latest (updates: $($left -join ', '))"
        if ($run.h) { Shot-Window $run.h "$app-1-$from-up-to-date" }
        $quit = Quit $run
        Start-Sleep -Seconds 5
        $v = Version $exe
        Note "${app}: --version after: $v; Settings > Apps: $(Installed $name)"
        Check ($quit -and (Setups $app).Count -eq 0 -and (Installed $name) -eq $from -and $v -eq "$app $from") "${app}: closed, no Setup started; still $name $from (Settings > Apps: $(Installed $name); --version: $v)"
    }
}

# Settings > Apps with the installed versions.
Start-Process "ms-settings:appsfeatures"
Start-Sleep -Seconds 10
[Win]::Screen("$shots\settings-apps.png")
Get-Process -Name SystemSettings -ErrorAction SilentlyContinue | Stop-Process -Force

Stop-Job $server -ErrorAction SilentlyContinue
Remove-Job -Force $server -ErrorAction SilentlyContinue
if ($script:failed -or $script:oks -eq 0) { Note "check_win_update_live: FAILED"; exit 1 }
Note "check_win_update_live: OK ($script:oks checks)"

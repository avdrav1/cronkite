# UPD-6 acceptance on a clean Windows runner (update-windows.yml's verify job, MSYS2 moved
# away): an installed app updates itself through the staged Setup.exe when its last running
# instance quits.
#
#   powershell -File scripts\check_win_update.ps1 [-Old DIR] [-Site DIR] [-Apps "brick slate frame etch"] [-Out out\win-update] [-RequireNotice]
#
# Windows PowerShell 5.1 on an interactive desktop (GitHub's windows-latest). OLD holds each
# app's old Setup.exe (<app>-<old>-x86_64-windows-setup.exe); SITE holds each app's newer
# release as update-windows.yml's bundle job lays it out: <app>\latest.json and
# latest.json.sig, signed with suite-update's TEST key (release_json.py with
# CRONKITE_RELEASE_TEST_KEY=1), naming http://127.0.0.1:8765/<app>/releases/v<new>/<setup>.
# Both builds trust that key (`--features suite-update/test-key`); release builds never do.
# A throwaway site on 127.0.0.1:8765 (HttpListener) serves them, the old release for
# install.ps1, and the tampered copies. The apps run as Explorer starts them (PATH of
# Windows' own folders, nothing of MSYS2), with CRONKITE_UPDATE_URL naming the site. Per app:
#   1. install.ps1 installs the old version (Settings > Apps, --version);
#   2. a latest.json with a changed byte (its signature no longer verifies) is refused:
#      nothing staged, the old version starts and quits as before;
#   3. a Setup.exe whose bytes differ from the signed sha256 is refused the same way;
#   4. a staged Setup.exe changed after its download is refused at the exit that would run
#      it (the apply step's SHA-256 check): removed, no Setup started, the old version stays;
#   5. Setup closes no running instance: with the app running, the staged Setup.exe run with
#      the updater's options waits, then stops with exit code 7; the app still runs, its
#      window answers, its files and version are as they were;
#   6. two running instances: quitting the first starts no Setup (it says it waits) and the
#      second keeps running; quitting the second starts Setup detached, with
#      /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /NOCLOSEAPPLICATIONS, running
#      on after the app has exited; Settings > Apps then shows the new version, --version
#      prints it, and updates\applied names it;
#   7. the next start reports it: stderr's "update: Updated to <Name> <new>" (UPD-3's
#      notice; required with -RequireNotice), its window screenshotted; at its exit the
#      installed Setup.exe and its record leave the updates folder.
# Programs and Features and Settings > Apps are screenshotted with the new versions.
# Screenshots land in $Out\shots, logs in $Out\logs, the result lines in $Out\summary.txt;
# it ends with "check_win_update: OK" or exits 1 after every app ran.
param(
    [string]$Old = "upd\old",
    [string]$Site = "upd\site",
    [string]$Apps = "brick slate frame etch",
    [string]$Out = "out\win-update",
    [switch]$RequireNotice
)
$ErrorActionPreference = "Continue"
$root = (Resolve-Path "$PSScriptRoot\..").Path
# (PowerShell's names ignore case: $Old and $Site are read here once, never as $old or $site.)
$oldBuilds = (Resolve-Path $Old).Path
$newSite = (Resolve-Path $Site).Path
foreach ($d in "shots", "logs", "files", "site") { New-Item -ItemType Directory -Force "$Out\$d" | Out-Null }
$Out = (Resolve-Path $Out).Path
$shots = "$Out\shots"; $logs = "$Out\logs"; $files = "$Out\files"; $www = "$Out\site"
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
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
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

    // Whether the window's thread answers a message within 5 s (WM_NULL, SMTO_ABORTIFHUNG).
    public static bool Answers(long h) {
        IntPtr r;
        return SendMessageTimeoutW(new IntPtr(h), 0, IntPtr.Zero, IntPtr.Zero, 2, 5000, out r) != IntPtr.Zero;
    }

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
$UpdaterOptions = "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /NOCLOSEAPPLICATIONS"

# --- Setup -----------------------------------------------------------------------------------
# As Explorer starts programs, with nothing of MSYS2 anywhere.
$env:PATH = "$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\Wbem;$env:SystemRoot\System32\WindowsPowerShell\v1.0\"
foreach ($v in "HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_DATA_DIRS",
    "GSK_RENDERER", "PANGOCAIRO_BACKEND", "GTK_DATA_PREFIX", "GDK_PIXBUF_MODULE_FILE", "FONTCONFIG_FILE", "MSYSTEM",
    "CRONKITE_NO_UPDATE", "CRONKITE_UPDATE_URL") {
    Remove-Item "env:$v" -ErrorAction SilentlyContinue
}
$msys = @(Get-Command gcc, pacman, cygpath, ntldd -ErrorAction SilentlyContinue)
Check (($msys.Count -eq 0) -and -not (Test-Path C:\msys64)) "no MSYS2 on PATH or in C:\msys64 ($($msys.Count) of its programs found)"
Note "Windows $([Environment]::OSVersion.Version)"

# The site: the signed new release as the bundle job made it, the old one for install.ps1,
# and a latest.json whose signature no longer verifies.
$port = 8765
$base = "http://127.0.0.1:$port"
Copy-Item -Recurse -Force "$newSite\*" $www
$olds = @{}; $news = @{}; $setups = @{}
foreach ($app in $Apps -split '\s+' | Where-Object { $_ }) {
    $oldSetup = Get-ChildItem "$oldBuilds\$app-*-x86_64-windows-setup.exe" | Select-Object -First 1
    $latest = "$www\$app\latest.json"
    if (-not $oldSetup -or -not (Test-Path $latest) -or -not (Test-Path "$latest.sig")) { Fail "${app}: no old Setup.exe in $oldBuilds or no signed latest.json in $newSite\$app"; continue }
    $old = $oldSetup.Name -replace "^$app-(.+)-x86_64-windows-setup\.exe$", '$1'
    $text = [IO.File]::ReadAllText($latest)
    $release = $text | ConvertFrom-Json
    $new = $release.version
    $win = $release.platforms.'windows-x86_64'
    $setup = "$www\$app\releases\v$new\$([IO.Path]::GetFileName(([uri]$win.url).AbsolutePath))"
    $sha = if (Test-Path $setup) { (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLowerInvariant() } else { "" }
    $ok = $release.app -eq $app -and $win.url -like "$base/$app/releases/v$new/*" -and $sha -eq $win.sha256 -and $old -ne $new
    Check $ok "${app}: the site offers $app $new (signed latest.json naming $($win.url), sha256 $($win.sha256.Substring(0, 12))...) over the installed $old"
    if (-not $ok) { continue }
    $olds[$app] = $old; $news[$app] = $new; $setups[$app] = $setup

    $dir = "$www\$app-old"
    New-Item -ItemType Directory -Force "$dir\releases\v$old" | Out-Null
    Copy-Item "$root\sites\$app\install.ps1" "$dir\install.ps1"
    Copy-Item $oldSetup.FullName "$dir\releases\v$old\$($oldSetup.Name)"
    $oldRelease = [ordered]@{
        app       = $app
        platforms = [ordered]@{
            "windows-x86_64" = [ordered]@{ url = "$base/$app-old/releases/v$old/$($oldSetup.Name)"; sha256 = (Get-FileHash -Algorithm SHA256 $oldSetup.FullName).Hash.ToLowerInvariant(); bytes = $oldSetup.Length }
        }
        version   = $old
    }
    [IO.File]::WriteAllText("$dir\latest.json", ($oldRelease | ConvertTo-Json -Compress -Depth 4))

    # One byte of the signed manifest changed: a newer version than it was signed for.
    $dir = "$www\$app-badsig"
    New-Item -ItemType Directory -Force $dir | Out-Null
    $bad = $text.Replace("`"version`":`"$new`"", "`"version`":`"$new.1`"")
    [IO.File]::WriteAllText("$dir\latest.json", $bad)
    Copy-Item "$latest.sig" "$dir\latest.json.sig"
}

$server = Start-Job -ArgumentList $www, $port -ScriptBlock {
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
$deadline = (Get-Date).AddSeconds(30)
do {
    try { $up = (Invoke-WebRequest -UseBasicParsing "$base/$(($setups.Keys | Select-Object -First 1))/latest.json").StatusCode -eq 200 } catch { $up = $false }
    if (-not $up) { Start-Sleep -Milliseconds 500 }
} while (-not $up -and (Get-Date) -lt $deadline)
Check $up "the update site answers on $base"

# --- Helpers ---------------------------------------------------------------------------------
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
function Log-Line([string]$file, [string]$pattern) {
    $line = ((Read-Log $file) -split "`r?`n" | Where-Object { $_ -match $pattern } | Select-Object -First 1)
    if ($line) { return ($line -replace '^.*?(update: )', '$1').Trim() } else { return "(none)" }
}

# The installed app, started as a user would, with CRONKITE_UPDATE_URL naming $url; its
# stderr in $logs\$tag.err.txt. Returns the process and its window (0 if none showed).
function Start-App([string]$exe, [string]$tag, [string]$url) {
    $env:CRONKITE_UPDATE_URL = $url
    $p = Start-Process -FilePath $exe -WorkingDirectory $files -PassThru `
        -RedirectStandardOutput "$logs\$tag.out.txt" -RedirectStandardError "$logs\$tag.err.txt"
    Remove-Item env:CRONKITE_UPDATE_URL
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
function Exes([string]$updates) { @(Get-ChildItem "$updates\*.exe" -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }) }

# The bundle's files that an interrupted Setup would have removed.
function Intact([string]$dir, [string]$app) {
    @("bin\$app.exe", "bin\libgtk-4-1.dll", "share\glib-2.0\schemas\gschemas.compiled", "etc\fonts\fonts.conf",
        "lib\gdk-pixbuf-2.0\2.10.0\loaders.cache", "licenses\THIRD-PARTY.txt") | Where-Object { -not (Test-Path "$dir\$_") }
}

function Shot-Window([long]$h, [string]$name) {
    [Win]::Front($h)
    Start-Sleep -Milliseconds 800
    [Win]::Shot($h, "$shots\$name.png")
}

function Flip-Byte([string]$file) {
    $b = [IO.File]::ReadAllBytes($file)
    $i = [int]($b.Length / 2)
    $b[$i] = $b[$i] -bxor 0xFF
    [IO.File]::WriteAllBytes($file, $b)
}

# --- Each app --------------------------------------------------------------------------------
foreach ($app in $Apps -split '\s+' | Where-Object { $_ -and $setups.ContainsKey($_) }) {
    $name = $Names[$app]
    $old = $olds[$app]; $new = $news[$app]
    $dir = "$env:LOCALAPPDATA\Programs\$name"
    $exe = "$dir\bin\$app.exe"
    $updates = "$env:LOCALAPPDATA\Cronkite\$name\updates"
    $url = "$base/$app"
    Note "== $name $old -> $new"

    # 1. The old version, through install.ps1.
    Set-Item "env:$($app.ToUpperInvariant())_DOWNLOAD_URL" "$base/$app-old"
    $text = & powershell.exe -NoProfile -Command "irm $base/$app-old/install.ps1 | iex" 2>&1 | Out-String
    Remove-Item "env:$($app.ToUpperInvariant())_DOWNLOAD_URL"
    Set-Content -Encoding utf8 "$logs\$app-1-install.txt" $text
    $v = Version $exe
    Check ($text -match "(?m)^Installed $name $([regex]::Escape($old))\r?$" -and (Installed $name) -eq $old -and $v -eq "$app $old") "${app}: install.ps1 installs $name $old (Settings > Apps: $(Installed $name); --version: $v)"
    Remove-Item -Recurse -Force $updates -ErrorAction SilentlyContinue

    # 2. A manifest whose signature doesn't verify.
    $run = Start-App $exe "$app-2-badsig" "$base/$app-badsig"
    $said = Wait-Log $run.err "signature doesn't verify" 60
    $quit = Quit $run
    $s = Staged $updates
    Check ($run.h -and $said -and $quit -and -not $s -and (Exes $updates).Count -eq 0 -and (Installed $name) -eq $old) "${app}: a latest.json with a changed byte is refused ('$(Log-Line $run.err 'update:')'); nothing staged, $name $old ran and quit"

    # 3. A Setup.exe that isn't the file the signed manifest names.
    $good = "$Out\$app-good-setup.exe"
    Copy-Item $setups[$app] $good -Force
    Flip-Byte $setups[$app]
    $run = Start-App $exe "$app-3-badfile" $url
    $said = Wait-Log $run.err "sha256 is" 120
    $quit = Quit $run
    Copy-Item $good $setups[$app] -Force
    $s = Staged $updates
    Check ($run.h -and $said -and $quit -and -not $s -and (Exes $updates).Count -eq 0 -and (Installed $name) -eq $old) "${app}: a Setup.exe with a changed byte is refused ('$(Log-Line $run.err 'update:')'); nothing staged, $name $old ran and quit"

    # 4. A staged Setup.exe changed after its download: refused at the exit that would run it.
    $run = Start-App $exe "$app-4-staged-tampered" $url
    $said = Wait-Log $run.err "update: $name $([regex]::Escape($new)) downloaded" 120
    $s = Staged $updates
    $file = if ($s) { "$updates\$($s.file)" } else { "" }
    $sha = if ($file -and (Test-Path $file)) { (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant() } else { "" }
    Check ($said -and $s -and $s.version -eq $new -and $sha -eq $s.sha256) "${app}: $name $new downloaded and staged ($($s.file), sha256 as signed): '$(Log-Line $run.err 'downloaded')'"
    if ($file) { Flip-Byte $file }
    $quit = Quit $run
    $refused = Wait-Log $run.err "refused $name $([regex]::Escape($new))'s Setup.exe" 10
    Start-Sleep -Seconds 5
    $started = Setups $app
    Check ($quit -and $refused -and -not (Staged $updates) -and (Exes $updates).Count -eq 0 -and $started.Count -eq 0 -and -not (Test-Path "$updates\applied")) "${app}: the changed staged Setup.exe is refused at exit ('$(Log-Line $run.err 'refused')'), removed, no Setup started"
    $v = Version $exe
    Check ((Installed $name) -eq $old -and $v -eq "$app $old" -and @(Intact $dir $app).Count -eq 0) "${app}: $name $old still installed and working (--version: $v)"

    # 5. Setup closes no running instance.
    $a = Start-App $exe "$app-5-running" $url
    $said = Wait-Log $a.err "update: $name $([regex]::Escape($new)) downloaded" 120
    $s = Staged $updates
    Check ($a.h -and $said -and $s) "${app}: staged again by a running $name ($($s.file))"
    if ($s) {
        $t0 = Get-Date
        $p = Start-Process -FilePath "$updates\$($s.file)" -ArgumentList (($UpdaterOptions -split ' ') + "/LOG=`"$logs\$app-5-setup.log`"") -Wait -PassThru
        $secs = [math]::Round(((Get-Date) - $t0).TotalSeconds)
        $log = Get-Content -Raw "$logs\$app-5-setup.log" -ErrorAction SilentlyContinue
        $alive = -not $a.p.HasExited -and [Win]::Answers($a.h)
        $missing = @(Intact $dir $app)
        Check ($p.ExitCode -eq 7 -and $log -match "is running after" -and $alive -and (Installed $name) -eq $old -and $missing.Count -eq 0) "${app}: Setup.exe with the updater's options leaves the running $name alone: exit $($p.ExitCode) after $secs s ('$((($log -split "`r?`n") | Where-Object { $_ -match 'is running after' } | Select-Object -First 1) -replace '^\S+ \S+\s+', '')'), $name still running and answering, $(Installed $name) installed, files intact$(if ($missing) { '; missing: ' + ($missing -join ', ') })"
        Shot-Window $a.h "$app-5-still-running"
    }

    # 6. Two instances: the update waits for both to quit.
    $b = Start-App $exe "$app-6-second" $url
    Check ($b.h -ne 0) "${app}: a second $name runs beside the first"
    $quitA = Quit $a
    $waits = Wait-Log $a.err "waits until every running $name has quit" 10
    Start-Sleep -Seconds 8
    $started = Setups $app
    Check ($quitA -and $waits -and $started.Count -eq 0 -and -not $b.p.HasExited -and [Win]::Answers($b.h) -and (Installed $name) -eq $old) "${app}: quitting the first starts no Setup ('$(Log-Line $a.err 'waits')'); the second runs on, $(Installed $name) installed"
    $t0 = Get-Date
    [Win]::Close($b.h)
    $seen = $null; $line = ""
    $deadline = (Get-Date).AddSeconds(60)
    do {
        $found = Setups $app | Where-Object { $_.CommandLine -match [regex]::Escape($UpdaterOptions) } | Select-Object -First 1
        if ($found) { $seen = $found; $line = $found.CommandLine } else { Start-Sleep -Milliseconds 100 }
    } while (-not $seen -and (Get-Date) -lt $deadline)
    $quitB = $b.p.WaitForExit(60000)
    $after = if ($seen) { [bool](Get-Process -Id $seen.ProcessId -ErrorAction SilentlyContinue) -or (Setups $app).Count -gt 0 } else { $false }
    Check ($quitB -and $seen -and $line -notmatch '/CLOSEAPPLICATIONS' -and (Read-Log $b.err) -match "update: installing $name $([regex]::Escape($new))") "${app}: quitting the last $name starts Setup.exe ('$(Log-Line $b.err 'installing')'): $line"
    Check $after "${app}: Setup runs on after $name has exited (detached)"
    $deadline = (Get-Date).AddSeconds(180)
    while ((Installed $name) -ne $new -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    $deadline = (Get-Date).AddSeconds(60)
    while ((Setups $app).Count -gt 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    $secs = [math]::Round(((Get-Date) - $t0).TotalSeconds)
    Copy-Item "$updates\setup.log" "$logs\$app-6-setup.log" -ErrorAction SilentlyContinue
    $setupLog = Get-Content -Raw "$logs\$app-6-setup.log" -ErrorAction SilentlyContinue
    $free = (($setupLog -split "`r?`n") | Where-Object { $_ -match 'is free after' } | Select-Object -First 1) -replace '^\S+ \S+\s+', ''
    $v = Version $exe
    $missing = @(Intact $dir $app)
    Check ((Installed $name) -eq $new -and $v -eq "$app $new" -and $missing.Count -eq 0) "${app}: Settings > Apps shows $name $(Installed $name) $secs s after quitting; --version: $v; Setup's log: '$free'"
    $applied = Get-Content "$updates\applied" -ErrorAction SilentlyContinue
    Check ($applied -contains "version=$new") "${app}: updates\applied names $new for the next start's notice ($applied)"

    # 7. The next start reports it, and its exit clears the installed Setup.exe.
    $run = Start-App $exe "$app-7-next" $url
    $notice = Wait-Log $run.err "update: Updated to $name $([regex]::Escape($new))" 20
    if ($run.h) { Start-Sleep -Seconds 2; Shot-Window $run.h "$app-7-updated" }
    if ($notice) { Pass "${app}: the next start says '$(Log-Line $run.err 'Updated to')' (screenshot $app-7-updated)" }
    elseif ($RequireNotice) { Fail "${app}: the next start doesn't say 'update: Updated to $name $new'" }
    else { Note "note ${app}: no notice on stderr yet (UPD-3 shows it); the window started: $([bool]$run.h)" }
    $quit = Quit $run
    Check ($run.h -and $quit -and -not (Staged $updates) -and (Exes $updates).Count -eq 0) "${app}: $name $new ran and quit; its exit removed the installed Setup.exe and its record (left in updates: $(@(Get-ChildItem -Name $updates -ErrorAction SilentlyContinue) -join ', '))"
}

# Settings > Apps and Programs and Features with the new versions.
Start-Process "ms-settings:appsfeatures"
Start-Sleep -Seconds 10
[Win]::Screen("$shots\settings-apps.png")
Get-Process -Name SystemSettings -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Process control.exe -ArgumentList "appwiz.cpl"
Start-Sleep -Seconds 8
[Win]::Screen("$shots\programs-and-features.png")
foreach ($w in [Win]::Windows(0)) { if (($w -split "`t")[2] -eq "Programs and Features") { [Win]::Close([long](($w -split "`t")[0])) } }

Stop-Job $server -ErrorAction SilentlyContinue
Remove-Job -Force $server -ErrorAction SilentlyContinue
if ($script:failed) { Note "check_win_update: FAILED"; exit 1 }
Note "check_win_update: OK ($script:oks checks)"

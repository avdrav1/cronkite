# WIN-5 acceptance on a clean Windows runner (release-windows.yml's verify job, MSYS2 moved
# away): each app installs per user through its Setup.exe and through install.ps1, starts,
# opens and saves a file, and uninstalls with nothing left behind.
#
#   powershell -File scripts\check_win_install.ps1 [-Dist dist] [-Apps "brick slate frame etch"] [-Out out\win-install]
#
# Windows PowerShell 5.1 (Add-Type builds the helpers below against .NET Framework) on an
# interactive desktop (GitHub's windows-latest). DIST holds scripts/win_bundle.sh's
# <app>-<version>-x86_64-windows-setup.exe and .sha256. The apps run as Explorer starts
# them: PATH is Windows' own folders, no HOME, no XDG_*, no MSYS2 anywhere. A throwaway
# download site on 127.0.0.1 (HttpListener) serves each app's sites/<app>/install.ps1, a
# latest.json whose "platforms" "windows-x86_64" entry (url, sha256, bytes) names the
# Setup.exe, and a tampered copy. Per app:
#   1. install.ps1 refuses the tampered Setup.exe and installs nothing;
#   2. then twice, once through `Setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
#      /CURRENTUSER` and once through `irm .../install.ps1 | iex` (which prints "Installed
#      <Name> <version>"):
#      - per user, no admin: %LOCALAPPDATA%\Programs\<Name> with bin\<app>.exe (a GUI
#        program), GTK's DLLs and no gdbus.exe, loaders, schemas, icons, fontconfig's files, the
#        fonts (Brick, Slate, Frame), the .ico, the licence notices (THIRD-PARTY.txt naming
#        gtk4 and no lzo or jbig, THIRD-PARTY-RUST.txt for x86_64-pc-windows-gnu with gtk4),
#        and Inno Setup's log saying it installs in non-administrative mode under HKCU;
#      - a Start menu shortcut to it; Settings > Apps' entry (HKCU's Uninstall key: name,
#        version, publisher, icon, uninstaller) and none in HKLM;
#      - its file types under HKCU: a ProgID per extension in the extension's
#        OpenWithProgids, Applications\<app>.exe with SupportedTypes, Capabilities and
#        RegisteredApplications; the shell's Open with list (SHAssocEnumHandlers) offers
#        it; the extensions' defaults (their HKCU default value, Explorer's UserChoice, and
#        the program AssocQueryString opens them with) are as they were;
#      - started from the Start menu shortcut, it shows a window, with no console, every
#        DLL it loads from its folder or Windows;
#      - a file opened through its ProgID (what Open with runs) shows in its window;
#      - started on a file, it says it runs from the bundle, takes typing or a command and
#        Control+S, and the file on disk changes (Brick: zz into A2; Slate: Hello at the
#        start; Frame: a new slide; Etch: rotated); fontconfig's cache is in
#        %LOCALAPPDATA%\Cronkite\<Name>\fontconfig, not a shared folder;
#      - uninstalled (the Uninstall key's command, silently; then install.ps1 -Uninstall,
#        which prints "Removed <Name>"), nothing is left: the folder, the shortcut, every
#        key and value it wrote under HKCU\Software outside Microsoft's (diffed against
#        before the install), and every new file or folder in %APPDATA%,
#        %LOCALAPPDATA%, and the profile, except the user's own data, which stays:
#        %APPDATA%\Cronkite\<Name> and %LOCALAPPDATA%\Cronkite\<Name> without its
#        fontconfig cache (WIN-5's policy).
#   3. For the first app, uninstalling by hand (no options) asks first, then asks whether
#      to remove the user's data too; No keeps it.
# Settings > Apps and Programs and Features are screenshotted with the apps installed.
# Screenshots land in $Out\shots, logs in $Out\logs, the result lines in
# $Out\summary.txt; it ends with "check_win_install: OK" or exits 1 after every app ran.
param(
    [string]$Dist = "dist",
    [string]$Apps = "brick slate frame etch",
    [string]$Out = "out\win-install"
)
$ErrorActionPreference = "Continue"
$root = (Resolve-Path "$PSScriptRoot\..").Path
$Dist = (Resolve-Path $Dist).Path
foreach ($d in "shots", "logs", "files", "site") { New-Item -ItemType Directory -Force "$Out\$d" | Out-Null }
$Out = (Resolve-Path $Out).Path
$shots = "$Out\shots"; $logs = "$Out\logs"; $files = "$Out\files"; $site = "$Out\site"
$summary = "$Out\summary.txt"
Set-Content -Encoding utf8 $summary ""
$script:failed = $false
$script:oks = 0
function Note([string]$line) { Add-Content -Encoding utf8 $summary $line; Write-Host $line }
function Pass([string]$what) { $script:oks++; Note "ok   $what" }
function Fail([string]$what) { Note "FAIL $what"; $script:failed = $true }
function Check([bool]$ok, [string]$what) { if ($ok) { Pass $what } else { Fail $what } }

Add-Type -AssemblyName System.Windows.Forms, System.Drawing, System.IO.Compression.FileSystem
Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;

[ComImport, Guid("F04061AC-1659-4a3f-A954-775AA57FC083"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAssocHandler {
    [PreserveSig] int GetName([MarshalAs(UnmanagedType.LPWStr)] out string name);
    [PreserveSig] int GetUIName([MarshalAs(UnmanagedType.LPWStr)] out string name);
    [PreserveSig] int GetIconLocation([MarshalAs(UnmanagedType.LPWStr)] out string path, out int index);
    [PreserveSig] int IsRecommended();
    [PreserveSig] int MakeDefault([MarshalAs(UnmanagedType.LPWStr)] string description);
    [PreserveSig] int Invoke(IntPtr data);
    [PreserveSig] int CreateInvoker(IntPtr data, out IntPtr invoker);
}

[ComImport, Guid("973810ae-9599-4b88-9e4d-6ee98c9552da"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IEnumAssocHandlers {
    [PreserveSig] int Next(uint count, [Out, MarshalAs(UnmanagedType.LPArray, ArraySubType = UnmanagedType.Interface, SizeParamIndex = 0)] IAssocHandler[] handlers, out uint fetched);
}

public static class Win {
    delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)] static extern int SHAssocEnumHandlers(string extension, int filter, out IEnumAssocHandlers handlers);
    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)] static extern int AssocQueryStringW(int flags, int what, string assoc, string extra, StringBuilder result, ref uint size);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    // Visible top-level windows with a title, as "handle<TAB>pid<TAB>class<TAB>title"; of
    // process `pid`, or of every process when it's 0.
    public static string[] Windows(int pid) {
        var found = new List<string>();
        EnumWindows(delegate (IntPtr h, IntPtr p) {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if ((pid == 0 || owner == (uint)pid) && IsWindowVisible(h)) {
                var s = new StringBuilder(512);
                GetWindowTextW(h, s, 512);
                var c = new StringBuilder(256);
                GetClassNameW(h, c, 256);
                if (s.Length > 0) found.Add(h.ToInt64() + "\t" + owner + "\t" + c + "\t" + s);
            }
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    // Bring a window to the front: an Alt press first lets this process set the foreground.
    public static void Front(long h) {
        ShowWindow(new IntPtr(h), 9);
        keybd_event(0x12, 0, 0, UIntPtr.Zero);
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        SetForegroundWindow(new IntPtr(h));
    }

    public static void Close(long h) { PostMessageW(new IntPtr(h), 0x10, IntPtr.Zero, IntPtr.Zero); }

    // The window as it shows on screen (its visible bounds) into a PNG.
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

    // Key `vk` down and up with `mods` held (virtual keys).
    public static void Keys(byte[] mods, byte vk) {
        foreach (var m in mods) keybd_event(m, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 2, UIntPtr.Zero);
        for (int i = mods.Length - 1; i >= 0; i--) keybd_event(mods[i], 0, 2, UIntPtr.Zero);
    }

    // The programs the shell's Open with offers for `extension`, as "ui name<TAB>path".
    public static string[] Handlers(string extension) {
        var found = new List<string>();
        IEnumAssocHandlers e;
        if (SHAssocEnumHandlers(extension, 0, out e) != 0 || e == null) return found.ToArray();
        var one = new IAssocHandler[1];
        uint got;
        while (e.Next(1, one, out got) == 0 && got == 1) {
            string name, ui;
            one[0].GetName(out name);
            one[0].GetUIName(out ui);
            found.Add(ui + "\t" + name);
            Marshal.ReleaseComObject(one[0]);
        }
        Marshal.ReleaseComObject(e);
        return found.ToArray();
    }

    // The program the shell opens `extension` with (ASSOCSTR_EXECUTABLE), or "".
    public static string Opener(string extension) {
        var s = new StringBuilder(1024);
        uint n = 1024;
        return AssocQueryStringW(0x20 /* NOTRUNCATE */, 2, extension, "open", s, ref n) == 0 ? s.ToString() : "";
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct SHELLEXECUTEINFO {
        public int cbSize; public uint fMask; public IntPtr hwnd; public string lpVerb; public string lpFile;
        public string lpParameters; public string lpDirectory; public int nShow; public IntPtr hInstApp;
        public IntPtr lpIDList; public string lpClass; public IntPtr hkeyClass; public uint dwHotKey;
        public IntPtr hIcon; public IntPtr hProcess;
    }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool ShellExecuteExW(ref SHELLEXECUTEINFO i);

    // Open `file` with the open verb of ProgID `cls` (SEE_MASK_CLASSNAME), as Open with does.
    public static bool OpenAs(string file, string cls) {
        var i = new SHELLEXECUTEINFO();
        i.cbSize = Marshal.SizeOf(typeof(SHELLEXECUTEINFO));
        i.fMask = 0x1;
        i.lpVerb = "open"; i.lpFile = file; i.lpClass = cls; i.nShow = 1;
        return ShellExecuteExW(ref i);
    }
}
"@

# --- The apps ------------------------------------------------------------------------------
$Names = @{ brick = "Brick"; slate = "Slate"; frame = "Frame"; etch = "Etch" }
$Types = @{
    brick = ".csv", ".tsv", ".xlsx", ".xlsm"
    slate = ".docx", ".md", ".markdown", ".txt"
    frame = ".pptx", ".pptm"
    etch  = ".png", ".jpg", ".jpeg", ".bmp"
}
$FontApps = "brick", "slate", "frame"
$VK_SHIFT = 0x10; $VK_CONTROL = 0x11

# Type $text slowly into the window in front (letters, digits, spaces; Shift for capitals).
function Type-Text([string]$text) {
    foreach ($ch in $text.ToCharArray()) {
        $mods = @()
        if ([char]::IsUpper($ch)) { $mods = @([byte]$VK_SHIFT) }
        [Win]::Keys([byte[]]$mods, [byte][char]::ToUpperInvariant($ch))
        Start-Sleep -Milliseconds 300
    }
}
function Ctrl([char]$key, [switch]$Shift) {
    $mods = @([byte]$VK_CONTROL)
    if ($Shift) { $mods += [byte]$VK_SHIFT }
    [Win]::Keys([byte[]]$mods, [byte][char]::ToUpperInvariant($key))
    Start-Sleep -Milliseconds 500
}

# A visible titled window of a process whose title contains $text (any title for ""),
# within $seconds; returns its handle, 0 if none. $procs gives the processes.
function Wait-Window($procs, [string]$text, [int]$seconds = 60) {
    $deadline = (Get-Date).AddSeconds($seconds)
    do {
        $script:titles = @()
        foreach ($p in @(& $procs)) {
            foreach ($w in [Win]::Windows($p.Id)) {
                $h, $owner, $class, $t = $w -split "`t", 4
                $script:titles += $t
                if ($t.IndexOf($text, [StringComparison]::OrdinalIgnoreCase) -ge 0) { return [long]$h }
            }
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    return 0
}

# The running processes of the installed $exe.
function Of([string]$exe) {
    $name = [IO.Path]::GetFileNameWithoutExtension($exe)
    return { Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($_.Path -ieq $exe) } }.GetNewClosure()
}

function Stop-All([string]$exe) {
    & (Of $exe) | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 800
}

function Shot([long]$h, [string]$name) {
    [Win]::Front($h)
    Start-Sleep -Milliseconds 800
    [Win]::Shot($h, "$shots\$name.png")
}

# The PE subsystem of an exe: 2 GUI, 3 console.
function Subsystem([string]$exe) {
    $b = [IO.File]::ReadAllBytes($exe)
    $pe = [BitConverter]::ToInt32($b, 0x3C)
    return [BitConverter]::ToUInt16($b, $pe + 4 + 20 + 68)
}

# HKCU\Software's keys and value names, from `reg export`, as a set of strings.
function Registry-Names([string]$file) {
    & reg.exe export "HKCU\Software" $file /y | Out-Null
    $set = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $key = ""
    foreach ($line in [IO.File]::ReadLines($file)) {
        if ($line.StartsWith("[")) { $key = $line.Trim("[", "]"); [void]$set.Add($key) }
        elseif ($line -match '^"((?:[^"\\]|\\.)*)"=') { [void]$set.Add("$key\\$($Matches[1])") }
        elseif ($line.StartsWith("@=")) { [void]$set.Add("$key\\@") }
    }
    return , $set
}

# Files and folders an app could leave: everything under the suite's and the installs'
# folders and the Start menu's programs, and the top level of %APPDATA%, %LOCALAPPDATA%, and
# the profile (where GLib, GTK, and fontconfig would put theirs).
function File-Names {
    $set = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($d in "$env:APPDATA\Cronkite", "$env:LOCALAPPDATA\Cronkite", "$env:LOCALAPPDATA\Programs",
        "$env:APPDATA\Microsoft\Windows\Start Menu\Programs", "$env:LOCALAPPDATA\fontconfig", "$env:USERPROFILE\.cache") {
        if (Test-Path $d) { Get-ChildItem -Recurse -Force $d | ForEach-Object { [void]$set.Add($_.FullName) } }
    }
    foreach ($d in $env:APPDATA, $env:LOCALAPPDATA, $env:USERPROFILE) {
        Get-ChildItem -Force $d | ForEach-Object { [void]$set.Add($_.FullName) }
    }
    return , $set
}

function Snapshot([string]$tag) {
    return @{ reg = (Registry-Names "$logs\reg-$tag.reg"); files = (File-Names) }
}

# What's in $after and not in $before.
function Added($before, $after) { return @($after | Where-Object { -not $before.Contains($_) } | Sort-Object) }

# The user data an app may keep after uninstalling (WIN-5's policy): its two Cronkite
# folders, except fontconfig's cache, which the uninstaller removes; the parent Cronkite
# folders while they hold them.
function Kept([string]$path, [string]$name) {
    foreach ($d in "$env:APPDATA\Cronkite\$name", "$env:LOCALAPPDATA\Cronkite\$name") {
        if ($path -ieq $d -or $path -like "$d\*") { return -not ($path -like "$env:LOCALAPPDATA\Cronkite\$name\fontconfig*") }
    }
    return ($path -ieq "$env:APPDATA\Cronkite" -or $path -ieq "$env:LOCALAPPDATA\Cronkite")
}

# --- Setup -----------------------------------------------------------------------------------
# As Explorer starts programs, with nothing of MSYS2 anywhere.
$env:PATH = "$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\Wbem;$env:SystemRoot\System32\WindowsPowerShell\v1.0\"
foreach ($v in "HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_DATA_DIRS",
    "GSK_RENDERER", "PANGOCAIRO_BACKEND", "GTK_DATA_PREFIX", "GDK_PIXBUF_MODULE_FILE", "FONTCONFIG_FILE", "MSYSTEM") {
    Remove-Item "env:$v" -ErrorAction SilentlyContinue
}
$msys = @(Get-Command gcc, pacman, cygpath, ntldd -ErrorAction SilentlyContinue)
Check (($msys.Count -eq 0) -and -not (Test-Path C:\msys64)) "no MSYS2 on PATH or in C:\msys64 ($($msys.Count) of its programs found)"
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Note "Windows $([Environment]::OSVersion.Version), screen $($screen.Width)x$($screen.Height), this shell is admin: $admin (Setup must not ask for it)"

# The samples: a CSV, a text file, the fixture deck, and a 64x32 PNG.
$wide = New-Object System.Drawing.Bitmap 64, 32
$g = [System.Drawing.Graphics]::FromImage($wide)
$g.FillRectangle([System.Drawing.Brushes]::Firebrick, 0, 0, 32, 32)
$g.FillRectangle([System.Drawing.Brushes]::RoyalBlue, 32, 0, 32, 32)
$g.Dispose()
$wide.Save("$files\wide.png", [System.Drawing.Imaging.ImageFormat]::Png)
$wide.Dispose()
function Sample([string]$app, [string]$into) {
    switch ($app) {
        brick { [IO.File]::WriteAllText($into, "id,name,city`n1,Ada,Paris`n2,Linus,Austin`n") }
        slate { [IO.File]::WriteAllText($into, "Original line.`n") }
        frame { Copy-Item "$root\apps\frame\crates\pptx\tests\data\fixture.pptx" $into }
        etch { Copy-Item "$files\wide.png" $into }
    }
}
$SampleExt = @{ brick = ".csv"; slate = ".txt"; frame = ".pptx"; etch = ".png" }
function Slides([string]$pptx) {
    $zip = [IO.Compression.ZipFile]::OpenRead($pptx)
    try { return @($zip.Entries | Where-Object { $_.FullName -match '^ppt/slides/slide\d+\.xml$' }).Count } finally { $zip.Dispose() }
}
function Size([string]$png) {
    $img = [System.Drawing.Image]::FromFile($png)
    try { return "$($img.Width)x$($img.Height)" } finally { $img.Dispose() }
}

# The download site: each app's install.ps1, latest.json naming its Setup.exe, and a
# tampered copy whose Setup.exe has a byte more than latest.json's sha256 says.
$port = 8765
$base = "http://127.0.0.1:$port"
$server = Start-Job -ArgumentList $site, $port -ScriptBlock {
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
            $c.Response.OutputStream.Write($b, 0, $b.Length)
        } else { $c.Response.StatusCode = 404 }
        $c.Response.Close()
    }
}

$setups = @{}
$versions = @{}
foreach ($app in $Apps -split '\s+' | Where-Object { $_ }) {
    $setup = Get-ChildItem "$Dist\$app-*-x86_64-windows-setup.exe" | Select-Object -First 1
    if (-not $setup) { Fail "${app}: no $app-*-x86_64-windows-setup.exe in $Dist"; continue }
    $version = $setup.Name -replace "^$app-(.+)-x86_64-windows-setup\.exe$", '$1'
    $sha = (Get-FileHash -Algorithm SHA256 $setup.FullName).Hash.ToLowerInvariant()
    $recorded = ((Get-Content "$($setup.FullName).sha256") -split '\s+')[0]
    Check ($sha -eq $recorded) "${app}: $($setup.Name) matches its .sha256 ($($setup.Length) bytes)"
    $setups[$app] = $setup.FullName
    $versions[$app] = $version
    foreach ($kind in "", "-tampered") {
        $dir = "$site\$app$kind"
        New-Item -ItemType Directory -Force "$dir\releases\v$version" | Out-Null
        Copy-Item "$root\sites\$app\install.ps1" "$dir\install.ps1"
        $file = "$dir\releases\v$version\$($setup.Name)"
        Copy-Item $setup.FullName $file
        if ($kind) { [IO.File]::AppendAllText($file, "x") }
        $release = [ordered]@{
            platforms = [ordered]@{
                "linux-x86_64"   = [ordered]@{ url = "$base/$app$kind/releases/v$version/$app-$version-x86_64-linux.tar.gz"; sha256 = "0" * 64; bytes = 1 }
                "windows-x86_64" = [ordered]@{ url = "$base/$app$kind/releases/v$version/$($setup.Name)"; sha256 = $sha; bytes = $setup.Length }
            }
            version   = $version
            date      = (Get-Date -Format yyyy-MM-dd)
        }
        [IO.File]::WriteAllText("$dir\latest.json", ($release | ConvertTo-Json -Compress -Depth 4))
    }
}
$deadline = (Get-Date).AddSeconds(30)
do {
    try { $up = (Invoke-WebRequest -UseBasicParsing "$base/$(($setups.Keys | Select-Object -First 1))/latest.json").StatusCode -eq 200 } catch { $up = $false }
    if (-not $up) { Start-Sleep -Milliseconds 500 }
} while (-not $up -and (Get-Date) -lt $deadline)
Check $up "the download site answers on $base"

# Run install.ps1 as its page says, in a fresh Windows PowerShell; output and exit code.
function Install-Ps1([string]$app, [string]$kind = "", [switch]$Uninstall) {
    $var = "$($app.ToUpperInvariant())_DOWNLOAD_URL"
    Set-Item "env:$var" "$base/$app$kind"
    $url = "$base/$app$kind/install.ps1"
    if ($Uninstall) { $cmd = "& ([scriptblock]::Create((irm $url))) -Uninstall" } else { $cmd = "irm $url | iex" }
    $text = & powershell.exe -NoProfile -Command $cmd 2>&1 | Out-String
    $code = $LASTEXITCODE
    Remove-Item "env:$var"
    return @{ text = $text.Trim(); code = $code }
}

# --- Installed ---------------------------------------------------------------------------------
function Check-Installed([string]$app, [string]$tag, $defaults) {
    $name = $Names[$app]
    $dir = "$env:LOCALAPPDATA\Programs\$name"
    $exe = "$dir\bin\$app.exe"
    Check (Test-Path $exe) "${tag}: installed for this user in $dir"
    $missing = @("bin\libgtk-4-1.dll", "bin\libcairo-script-interpreter-2.dll", "bin\libtiff-6.dll",
        "lib\gdk-pixbuf-2.0\2.10.0\loaders.cache", "lib\gdk-pixbuf-2.0\2.10.0\loaders\pixbufloader_svg.dll",
        "share\glib-2.0\schemas\gschemas.compiled", "share\icons\Adwaita\index.theme",
        "share\icons\hicolor\48x48\apps\dev.cronkite.$name.png", "etc\fonts\fonts.conf", "etc\fonts\conf.d\30-metric-aliases.conf",
        "$app.ico", "LICENSE", "LICENSE-APACHE", "licenses\THIRD-PARTY.txt", "licenses\THIRD-PARTY-RUST.txt", "unins000.exe") |
        Where-Object { -not (Test-Path "$dir\$_") }
    Check ($missing.Count -eq 0) "${tag}: the bundle's files are there$(if ($missing) { '; missing: ' + ($missing -join ', ') })"
    Check (-not (Test-Path "$dir\bin\gdbus.exe")) "${tag}: no gdbus.exe (no D-Bus session bus on Windows)"
    $dlls = @(Get-ChildItem "$dir\bin\*.dll")
    $gpl = @($dlls | Where-Object { $_.Name -match 'lzo|jbig' })
    Check ($gpl.Count -eq 0) "${tag}: $($dlls.Count) DLLs in bin, none of them lzo or jbig ($($gpl.Name -join ', '))"
    $fonts = @(Get-ChildItem "$dir\fonts\*.ttf" -ErrorAction SilentlyContinue)
    if ($FontApps -contains $app) { Check ($fonts.Count -ge 10) "${tag}: carries the metric-compatible fonts ($($fonts.Count) .ttf)" }
    else { Check ($fonts.Count -eq 0) "${tag}: carries no fonts (Windows' own)" }
    $third = Get-Content -Raw "$dir\licenses\THIRD-PARTY.txt" -ErrorAction SilentlyContinue
    $rust = Get-Content -Raw "$dir\licenses\THIRD-PARTY-RUST.txt" -ErrorAction SilentlyContinue
    Check ($third -and $third -match '(?m)^gtk4 ' -and $third -notmatch '(?mi)^(lzo2|jbigkit) ') "${tag}: THIRD-PARTY.txt names gtk4 and neither lzo2 nor jbigkit ($([math]::Round($third.Length / 1KB)) KB)"
    Check ($rust -and $rust -match 'x86_64-pc-windows-gnu' -and $rust -match '(?m)^gtk4 ') "${tag}: THIRD-PARTY-RUST.txt is for x86_64-pc-windows-gnu and lists gtk4 ($([math]::Round($rust.Length / 1KB)) KB)"
    $sub = Subsystem $exe
    Check ($sub -eq 2) "${tag}: $app.exe is a GUI program (PE subsystem $sub)"

    $lnk = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\$name.lnk"
    $target = if (Test-Path $lnk) { (New-Object -ComObject WScript.Shell).CreateShortcut($lnk).TargetPath } else { "" }
    Check ($target -ieq $exe) "${tag}: the Start menu has $name, which starts $target"

    $u = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1" -ErrorAction SilentlyContinue
    $ok = $u -and $u.DisplayName -eq $name -and $u.DisplayVersion -eq $versions[$app] -and $u.Publisher -eq "Cronkite" -and
        $u.UninstallString -like "*$dir\unins000.exe*" -and $u.DisplayIcon -ieq "$dir\$app.ico"
    Check $ok "${tag}: Settings > Apps lists $($u.DisplayName) $($u.DisplayVersion) by $($u.Publisher) (HKCU Uninstall key, icon $($u.DisplayIcon))"
    $machine = Test-Path "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1"
    Check (-not $machine) "${tag}: nothing under HKLM (per-user install)"

    $classes = "HKCU:\Software\Classes"
    $command = "`"$exe`" `"%1`""
    $bad = @()
    foreach ($ext in $Types[$app]) {
        $progid = "Cronkite.$name$ext"
        $openWith = Get-Item "$classes\$ext\OpenWithProgids" -ErrorAction SilentlyContinue
        if (-not ($openWith -and ($openWith.GetValueNames() -contains $progid))) { $bad += "$ext\OpenWithProgids lacks $progid" }
        $cmd = (Get-ItemProperty "$classes\$progid\shell\open\command" -ErrorAction SilentlyContinue).'(default)'
        if ($cmd -ne $command) { $bad += "$progid opens with '$cmd'" }
        $supported = Get-ItemProperty "$classes\Applications\$app.exe\SupportedTypes" -ErrorAction SilentlyContinue
        if (-not ($supported -and ($supported.PSObject.Properties.Name -contains $ext))) { $bad += "SupportedTypes lacks $ext" }
        $assoc = (Get-ItemProperty "HKCU:\Software\Cronkite\$name\Capabilities\FileAssociations" -ErrorAction SilentlyContinue).$ext
        if ($assoc -ne $progid) { $bad += "Capabilities says $ext is '$assoc'" }
    }
    $registered = (Get-ItemProperty "HKCU:\Software\RegisteredApplications" -ErrorAction SilentlyContinue)."Cronkite $name"
    if ($registered -ne "Software\Cronkite\$name\Capabilities") { $bad += "RegisteredApplications says '$registered'" }
    $friendly = (Get-ItemProperty "$classes\Applications\$app.exe" -ErrorAction SilentlyContinue).FriendlyAppName
    if ($friendly -ne $name) { $bad += "FriendlyAppName '$friendly'" }
    Check ($bad.Count -eq 0) "${tag}: HKCU registers $($Types[$app] -join ' ') for Open with (ProgIDs, OpenWithProgids, Applications\$app.exe, Capabilities, RegisteredApplications)$(if ($bad) { ': ' + ($bad -join '; ') })"

    $offered = @(); $notOffered = @(); $changed = @(); $kept = @(); $unowned = @()
    foreach ($ext in $Types[$app]) {
        $handlers = [Win]::Handlers($ext)
        if ($handlers | Where-Object { ($_ -split "`t")[1] -ieq $exe }) { $offered += $ext } else { $notOffered += "$ext ($((@($handlers | ForEach-Object { ($_ -split "`t")[0] })) -join ', '))" }
        $now = @{
            default    = (Get-ItemProperty "$classes\$ext" -ErrorAction SilentlyContinue).'(default)'
            userChoice = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice" -ErrorAction SilentlyContinue).ProgId
            opener     = [Win]::Opener($ext)
        }
        $was = $defaults[$ext]
        # A default the user (or Windows) had: an HKCU ProgID, Explorer's UserChoice, or a
        # program other than Open with's picker. With none, Windows may now open the type
        # with the one program registered for it; that's Windows' choice, not a default set.
        $had = $was.default -or $was.userChoice -or ($was.opener -and $was.opener -notlike "*\OpenWith.exe")
        if ($now.default -ne $was.default -or $now.userChoice -ne $was.userChoice -or ($had -and $now.opener -ne $was.opener)) {
            $changed += "$ext ($($was.default)/$($was.userChoice)/$($was.opener) -> $($now.default)/$($now.userChoice)/$($now.opener))"
        } elseif ($had) { $kept += "$ext ($([IO.Path]::GetFileName([string]$was.opener)))" } else { $unowned += $ext }
        Add-Content -Encoding utf8 "$logs\$tag-associations.txt" "$ext before: default '$($was.default)', UserChoice '$($was.userChoice)', opens with '$($was.opener)'; after: '$($now.default)', '$($now.userChoice)', '$($now.opener)'; Open with: $($handlers -join ' | ')"
    }
    Check ($notOffered.Count -eq 0) "${tag}: the shell's Open with offers $name for $($offered -join ' ')$(if ($notOffered) { '; not for ' + ($notOffered -join ', ') })"
    Check ($changed.Count -eq 0) "${tag}: no default written; the user's stay ($(if ($kept) { $kept -join ', ' } else { 'none' }); types with none before: $(if ($unowned) { $unowned -join ' ' } else { 'none' }))$(if ($changed) { '; changed: ' + ($changed -join ', ') })"
    return $exe
}

# The extensions' defaults before the install: HKCU's default value, Explorer's UserChoice,
# and the program the shell opens them with.
function Defaults([string]$app) {
    $d = @{}
    foreach ($ext in $Types[$app]) {
        $d[$ext] = @{
            default    = (Get-ItemProperty "HKCU:\Software\Classes\$ext" -ErrorAction SilentlyContinue).'(default)'
            userChoice = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice" -ErrorAction SilentlyContinue).ProgId
            opener     = [Win]::Opener($ext)
        }
    }
    return $d
}

# --- Using it ----------------------------------------------------------------------------------
function Use-App([string]$app, [string]$tag, [string]$exe) {
    $name = $Names[$app]
    $dir = Split-Path (Split-Path $exe)
    $procs = Of $exe

    # From the Start menu, as a user starts it.
    $lnk = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\$name.lnk"
    $t0 = Get-Date
    Start-Process -FilePath $lnk
    $h = Wait-Window $procs "" 90
    $ms = [math]::Round(((Get-Date) - $t0).TotalMilliseconds)
    if ($h) {
        Start-Sleep -Seconds 3
        Shot $h "$tag-1-start-menu"
        Pass "${tag}: started from the Start menu, a window '$($script:titles[0])' after $ms ms"
        $p = @(& $procs)[0]
        $outside = @($p.Modules | Where-Object { $_.FileName -notlike "$dir\*" -and $_.FileName -notlike "$env:SystemRoot\*" } | ForEach-Object { $_.FileName })
        $mine = @($p.Modules | Where-Object { $_.FileName -like "$dir\*" }).Count
        Check ($outside.Count -eq 0) "${tag}: $mine modules from its folder, the rest from Windows$(if ($outside) { '; also: ' + ($outside -join ', ') })"
        $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($p.Id)" | ForEach-Object { $_.Name })
        $consoles = @($children | Where-Object { $_ -in "conhost.exe", "OpenConsole.exe", "WindowsTerminal.exe" })
        $consoleWindows = @([Win]::Windows(0) | Where-Object { ($_ -split "`t")[2] -in "ConsoleWindowClass", "CASCADIA_HOSTING_WINDOW_CLASS" -and ($_ -split "`t")[3] -like "*$app.exe*" })
        Check ($consoles.Count -eq 0 -and $consoleWindows.Count -eq 0) "${tag}: no console with it (its child processes: $(if ($children) { $children -join ', ' } else { 'none' }))"
    } else {
        [Win]::Screen("$shots\$tag-1-start-menu-screen.png")
        Fail "${tag}: no window from the Start menu shortcut within 90 s (titles: $($script:titles -join ' | '))"
    }
    Stop-All $exe

    # Open with: the ProgID's open command on a file.
    $other = "$files\$tag opened$($SampleExt[$app])"
    Sample $app $other
    $ok = [Win]::OpenAs($other, "Cronkite.$name$($SampleExt[$app])")
    $h = Wait-Window $procs "$tag opened" 60
    if ($ok -and $h) { Start-Sleep -Seconds 2; Shot $h "$tag-2-open-with"; Pass "${tag}: Open with's command opens '$tag opened$($SampleExt[$app])' in a window" }
    else { [Win]::Screen("$shots\$tag-2-open-with-screen.png"); Fail "${tag}: Open with's command opened no window for the file (shell: $ok; titles: $($script:titles -join ' | '))" }
    Stop-All $exe

    # Started on a file: edit, Control+S, and the file changes.
    $file = "$files\$tag$($SampleExt[$app])"
    Sample $app $file
    $before = if ($app -eq "frame") { Slides $file } else { 0 }
    $p = Start-Process -FilePath $exe -ArgumentList "`"$file`"" -WorkingDirectory $files -PassThru `
        -RedirectStandardOutput "$logs\$tag.out.txt" -RedirectStandardError "$logs\$tag.err.txt"
    $h = Wait-Window $procs "$tag$($SampleExt[$app])" 60
    if (-not $h) {
        [Win]::Screen("$shots\$tag-3-file-screen.png")
        Fail "${tag}: no window for $file (titles: $($script:titles -join ' | '); exited: $($p.HasExited))"
        Stop-All $exe
        return
    }
    Start-Sleep -Seconds 3
    [Win]::Front($h)
    Start-Sleep -Milliseconds 800
    switch ($app) {
        brick { Type-Text "zz"; [Win]::Keys(@(), 0x0D); Start-Sleep -Milliseconds 500 }
        slate { Type-Text "Hello " }
        frame { Ctrl 'm' }
        etch { Ctrl 'g' }
    }
    Start-Sleep -Milliseconds 800
    Ctrl 's'
    Start-Sleep -Seconds 3
    Shot $h "$tag-3-saved"
    switch ($app) {
        brick { $line = (Get-Content $file)[1]; Check ($line -eq "zz,Ada,Paris") "${tag}: typed zz into A2 and Control+S saved it (row 2: $line)" }
        slate { $text = (Get-Content -Raw $file).TrimEnd(); Check ($text -eq "Hello Original line.") "${tag}: typed Hello and Control+S saved it ('$text')" }
        frame { $n = Slides $file; Check ($n -eq $before + 1) "${tag}: New Slide and Control+S saved the deck ($before -> $n slides)" }
        etch { $s = Size $file; Check ($s -eq "32x64") "${tag}: Rotate and Control+S saved the picture (64x32 -> $s)" }
    }
    Stop-All $exe
    $log = Get-Content -Raw "$logs\$tag.err.txt" -ErrorAction SilentlyContinue
    Check ($log -match "running from the bundle") "${tag}: it says it runs from the bundle ($dir)"
    $cache = "$env:LOCALAPPDATA\Cronkite\$name\fontconfig"
    $caches = @(Get-ChildItem "$cache\*cache*" -ErrorAction SilentlyContinue)
    $shared = @("$env:LOCALAPPDATA\fontconfig", "$env:USERPROFILE\.cache\fontconfig") | Where-Object { Test-Path $_ }
    Check ($caches.Count -gt 0 -and $shared.Count -eq 0) "${tag}: fontconfig's cache is in $cache ($($caches.Count) files), not in a shared folder ($($shared -join ', '))"
}

# --- Uninstalled -----------------------------------------------------------------------------
function Check-Gone([string]$app, [string]$tag, $before) {
    $name = $Names[$app]
    $dir = "$env:LOCALAPPDATA\Programs\$name"
    Check (-not (Test-Path $dir)) "${tag}: $dir is gone"
    Check (-not (Test-Path "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\$name.lnk")) "${tag}: the Start menu entry is gone"
    Check (-not (Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1")) "${tag}: Settings > Apps no longer lists it"
    $after = Snapshot "$tag-after"
    $added = Added $before.reg $after.reg
    # Windows' own records (Software\Microsoft, Classes\Local Settings), those of other
    # programs on the runner that run on their own schedule (Firefox's default browser
    # agent, Software\Mozilla), and the Store packages Windows registers for the user in the
    # background (Feedback Hub, WSL: Classes\AppX*, Classes\Extensions\ContractId, their
    # protocols and extensions, RegisteredApplications\PackagedApps) aren't the app's. Under
    # Classes, the app writes only its types, its Cronkite.* ProgIDs, and Applications\.
    function Theirs($key) {
        $classes = "HKEY_CURRENT_USER\Software\Classes\"
        if ($key -like "HKEY_CURRENT_USER\Software\Microsoft\*" -or $key -like "${classes}Local Settings*" -or $key -like "HKEY_CURRENT_USER\Software\Mozilla*" -or $key -like "HKEY_CURRENT_USER\Software\RegisteredApplications\PackagedApps*") { return $true }
        if (-not $key.StartsWith($classes, [StringComparison]::OrdinalIgnoreCase)) { return $false }
        $top = $key.Substring($classes.Length).Split('\')[0]
        if ($key -match '\\OpenWithProgids\\\\AppX') { return $true }
        -not ($Types[$app] -contains $top -or $top -like "Cronkite.*" -or $top -ieq "Applications")
    }
    $windows = @($added | Where-Object { Theirs $_ })
    $left = @($added | Where-Object { $windows -notcontains $_ })
    Set-Content -Encoding utf8 "$logs\$tag-registry-windows.txt" $windows
    Check ($left.Count -eq 0) "${tag}: nothing it wrote is left under HKCU\Software ($($windows.Count) records of Windows' and other programs', in logs)$(if ($left) { '; left: ' + ($left -join '; ') })"
    $removed = Added $after.reg $before.reg
    # The shell drops its own records too (Local Settings\...\AppModel\ShellRefresh).
    $removedOutside = @($removed | Where-Object { -not (Theirs $_) })
    Check ($removedOutside.Count -eq 0) "${tag}: and nothing that was there before is gone$(if ($removedOutside) { ': ' + ($removedOutside -join '; ') })"
    $newFiles = Added $before.files $after.files
    $others = @($newFiles | Where-Object { $_ -like "$env:LOCALAPPDATA\Mozilla*" -or $_ -like "$env:APPDATA\Mozilla*" })
    $kept = @($newFiles | Where-Object { Kept $_ $name })
    $leftFiles = @($newFiles | Where-Object { -not (Kept $_ $name) -and $others -notcontains $_ })
    Set-Content -Encoding utf8 "$logs\$tag-files-kept.txt" $kept
    Check ($leftFiles.Count -eq 0) "${tag}: no file or folder left but the user's data ($($kept.Count) kept in %APPDATA%\Cronkite\$name and %LOCALAPPDATA%\Cronkite\$name, in logs)$(if ($leftFiles) { '; left: ' + ($leftFiles -join '; ') })"
    Check (-not (Test-Path "$env:LOCALAPPDATA\Cronkite\$name\fontconfig")) "${tag}: fontconfig's cache is gone"
}

function Wait-Uninstalled([string]$name) {
    $deadline = (Get-Date).AddSeconds(120)
    while ((Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1") -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    # The uninstaller's last steps (removing its own files) follow the key.
    $deadline = (Get-Date).AddSeconds(30)
    while ((Test-Path "$env:LOCALAPPDATA\Programs\$name") -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    Start-Sleep -Seconds 2
}

# Uninstall by hand, as Settings > Apps' Uninstall does: Inno asks first, then (WIN-5)
# whether to remove the user's data too. Yes to the first, No to the second, OK to the
# last; every dialog is screenshotted.
function Uninstall-By-Hand([string]$app, [string]$tag) {
    $name = $Names[$app]
    $u = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1"
    Start-Process -FilePath $u.UninstallString.Trim('"') -ArgumentList "/LOG=`"$logs\$tag-uninstall.log`""
    $answers = @(@{ key = 0x59; what = "confirm" }, @{ key = 0x4E; what = "data" }, @{ key = 0x0D; what = "done" })
    $seen = @()
    $handled = @{}
    foreach ($a in $answers) {
        $deadline = (Get-Date).AddSeconds(60)
        $dialog = $null
        do {
            foreach ($w in [Win]::Windows(0)) {
                $h, $owner, $class, $t = $w -split "`t", 4
                $uninstaller = (Get-Process -Id $owner -ErrorAction SilentlyContinue).Name -match '^_?(iu|unins)'
                if ($class -eq "#32770" -and ($uninstaller -or $t -like "*$name*") -and -not $handled.ContainsKey($h)) { $dialog = $w; break }
            }
            if (-not $dialog) {
                Start-Sleep -Milliseconds 300
                # What's on screen meanwhile, for the logs.
                $all = [Win]::Windows(0) | ForEach-Object {
                    $f = $_ -split "`t", 4
                    "$($f[1]) $((Get-Process -Id $f[1] -ErrorAction SilentlyContinue).Name) $($f[2]) '$($f[3])'"
                }
                Add-Content -Encoding utf8 "$logs\$tag-uninstall-windows.txt" "-- $(Get-Date -Format HH:mm:ss.fff) waiting for $($a.what)`n$($all -join "`n")"
            }
        } while (-not $dialog -and (Get-Date) -lt $deadline)
        if (-not $dialog) { break }
        $h = ($dialog -split "`t")[0]
        $handled[$h] = $true
        Start-Sleep -Milliseconds 800
        Shot ([long]$h) "$tag-uninstall-$($a.what)"
        $seen += "$($a.what): '$(($dialog -split "`t")[3])'"
        [Win]::Front([long]$h)
        Start-Sleep -Milliseconds 300
        [Win]::Keys(@(), [byte]$a.key)
        Start-Sleep -Seconds 2
    }
    Wait-Uninstalled $name
    $left = @(Get-Process | Where-Object { $_.Name -match '^_?(iu|unins)' })
    if ($left -or (Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1")) {
        # A dialog this didn't answer: keep a picture, then end the uninstaller.
        [Win]::Screen("$shots\$tag-uninstall-stuck.png")
        $left | Stop-Process -Force -ErrorAction SilentlyContinue
        if (Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1") {
            Start-Process -FilePath $u.UninstallString.Trim('"') -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait
            Wait-Uninstalled $name
        }
    }
    Check ($seen.Count -eq 3) "${tag}: uninstalling by hand asked to confirm, then whether to remove the user's data, then said it was done ($($seen -join '; '))"
}

# --- Each app, each way ---------------------------------------------------------------------
$first = $true
foreach ($app in $Apps -split '\s+' | Where-Object { $_ -and $setups.ContainsKey($_) }) {
    $name = $Names[$app]
    $version = $versions[$app]
    Note "== $name $version"

    # A tampered download is refused.
    $r = Install-Ps1 $app "-tampered"
    Add-Content -Encoding utf8 "$logs\$app-tampered.txt" $r.text
    $nothing = -not (Test-Path "$env:LOCALAPPDATA\Programs\$name") -and -not (Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1")
    Check ($r.code -ne 0 -and $r.text -match "checksum mismatch" -and $nothing) "${app}: install.ps1 refuses a tampered Setup.exe and installs nothing (exit $($r.code))"

    foreach ($how in "setup", "ps1") {
        $tag = "$app-$how"
        $before = Snapshot "$tag-before"
        $defaults = Defaults $app
        if ($how -eq "setup") {
            $log = "$logs\$tag-setup.log"
            $p = Start-Process -FilePath $setups[$app] -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CURRENTUSER", "/LOG=`"$log`"" -Wait -PassThru
            $text = Get-Content -Raw $log -ErrorAction SilentlyContinue
            $mode = @(Select-String -Path $log -Pattern "install mode" -ErrorAction SilentlyContinue | ForEach-Object { $_.Line -replace '^\S+ \S+\s+', '' })
            $ok = $p.ExitCode -eq 0 -and $text -match "Administrative install mode: No" -and $text -match "Install mode root key: HKEY_CURRENT_USER"
            Check $ok "${tag}: Setup.exe /VERYSILENT installs without admin rights (exit $($p.ExitCode); its log: $($mode -join '; '))"
        } else {
            $r = Install-Ps1 $app
            Set-Content -Encoding utf8 "$logs\$tag-install.txt" $r.text
            Check ($r.code -eq 0 -and $r.text -match "(?m)^Installed $name $([regex]::Escape($version))\r?$") "${tag}: irm install.ps1 | iex says '$(($r.text -split "`n" | Where-Object { $_ -match 'Installed|Updated|install:' } | Select-Object -First 1).Trim())'"
        }
        $exe = Check-Installed $app $tag $defaults
        if ($first) {
            # Settings > Apps and Programs and Features with the app installed.
            Start-Process "ms-settings:appsfeatures"
            Start-Sleep -Seconds 10
            [Win]::Screen("$shots\settings-apps.png")
            Get-Process -Name SystemSettings -ErrorAction SilentlyContinue | Stop-Process -Force
            Start-Process control.exe -ArgumentList "appwiz.cpl"
            Start-Sleep -Seconds 8
            [Win]::Screen("$shots\programs-and-features.png")
            foreach ($w in [Win]::Windows(0)) { if (($w -split "`t")[3] -eq "Programs and Features") { [Win]::Close([long](($w -split "`t")[0])) } }
        }
        Use-App $app $tag $exe

        if ($how -eq "setup") {
            if ($first) {
                Uninstall-By-Hand $app $tag
                $kept = (Test-Path "$env:APPDATA\Cronkite\$name") -or (Test-Path "$env:LOCALAPPDATA\Cronkite\$name")
                Check $kept "${tag}: No kept the user's data ($env:APPDATA\Cronkite\$name, $env:LOCALAPPDATA\Cronkite\$name)"
            } else {
                $u = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.cronkite.${name}_is1"
                Start-Process -FilePath $u.UninstallString.Trim('"') -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait
                Wait-Uninstalled $name
                Pass "${tag}: its uninstaller ran silently"
            }
        } else {
            $r = Install-Ps1 $app -Uninstall
            Set-Content -Encoding utf8 "$logs\$tag-uninstall.txt" $r.text
            Wait-Uninstalled $name
            Check ($r.code -eq 0 -and $r.text -match "^Removed $name\.") "${tag}: install.ps1 -Uninstall says '$(($r.text -split "`n")[0].Trim())'"
        }
        Check-Gone $app $tag $before
    }
    $first = $false
}

Stop-Job $server -ErrorAction SilentlyContinue
Remove-Job -Force $server -ErrorAction SilentlyContinue
if ($script:failed) { Note "check_win_install: FAILED"; exit 1 }
Note "check_win_install: OK ($script:oks checks)"

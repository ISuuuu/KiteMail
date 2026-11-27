<#
.SYNOPSIS
    Same-machine cold-start and memory benchmark comparing GPUI and Tauri+Vue `--demo` clients.

.DESCRIPTION
    Implements Milestone B.2 verification from docs/tauri-vue-main-window-migration.zh-CN.md:
    launches each available binary with `--demo` in an isolated temporary data directory,
    measures cold-start time until MainWindowHandle is non-zero, samples working set and
    private memory over `-SampleSeconds`, gracefully closes each client, and prints both
    a Markdown comparison table and JSON output. Missing binaries are reported cleanly
    without throwing terminating errors.
#>
[CmdletBinding()]
param(
    [string]$GpuiExe,
    [string]$TauriExe,
    [ValidateRange(1, 300)]
    [int]$SampleSeconds = 5,
    [ValidateRange(5, 180)]
    [int]$StartupTimeoutSeconds = 30,
    [string]$JsonOutPath,
    [switch]$KeepDataDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptRoot

if (-not $GpuiExe) {
    $GpuiExe = Join-Path $repoRoot "target\x86_64-pc-windows-msvc\release\kitemail.exe"
}
if (-not $TauriExe) {
    $TauriExe = Join-Path $repoRoot "src-tauri\target\x86_64-pc-windows-msvc\release\kitemail-tauri-preview.exe"
}

function ConvertTo-Megabytes {
    param([double]$Bytes)
    return [Math]::Round($Bytes / 1MB, 2)
}

function Format-NullableNumber {
    param(
        $Value,
        [int]$Decimals = 2
    )
    if ($null -eq $Value) {
        return "-"
    }
    return ([Math]::Round([double]$Value, $Decimals)).ToString("F$Decimals", [System.Globalization.CultureInfo]::InvariantCulture)
}

function Get-DescendantProcessIds {
    param([int]$RootPid)

    $allProcs = @()
    try {
        $allProcs = @(Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId -ErrorAction Stop)
    } catch {
        return @($RootPid)
    }

    $byParent = @{}
    foreach ($p in $allProcs) {
        $parentId = [int]$p.ParentProcessId
        if (-not $byParent.ContainsKey($parentId)) {
            $byParent[$parentId] = [System.Collections.Generic.List[int]]::new()
        }
        $byParent[$parentId].Add([int]$p.ProcessId)
    }

    $visited = [System.Collections.Generic.HashSet[int]]::new()
    $queue = [System.Collections.Generic.Queue[int]]::new()
    $null = $visited.Add($RootPid)
    $queue.Enqueue($RootPid)

    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        if ($byParent.ContainsKey($current)) {
            foreach ($childPid in $byParent[$current]) {
                if ($visited.Add($childPid)) {
                    $queue.Enqueue($childPid)
                }
            }
        }
    }

    return @($visited)
}

function Get-ProcessTreeMemorySample {
    param([int]$RootPid)

    $rootProc = Get-Process -Id $RootPid -ErrorAction SilentlyContinue
    if ($null -eq $rootProc) {
        return $null
    }

    $treePids = Get-DescendantProcessIds -RootPid $RootPid
    $treeWorkingSet = 0.0
    $treePrivateMem = 0.0
    $liveCount = 0

    foreach ($procId in $treePids) {
        $p = if ($procId -eq $RootPid) { $rootProc } else { Get-Process -Id $procId -ErrorAction SilentlyContinue }
        if ($null -ne $p) {
            $treeWorkingSet += [double]$p.WorkingSet64
            $treePrivateMem += [double]$p.PrivateMemorySize64
            $liveCount += 1
        }
    }

    return [PSCustomObject]@{
        mainWorkingSetBytes  = [double]$rootProc.WorkingSet64
        mainPrivateBytes     = [double]$rootProc.PrivateMemorySize64
        treeWorkingSetBytes  = $treeWorkingSet
        treePrivateBytes     = $treePrivateMem
        childProcessCount    = [Math]::Max(0, $liveCount - 1)
    }
}

function Stop-BenchmarkProcess {
    param([System.Diagnostics.Process]$Process)

    if ($null -eq $Process) {
        return $true
    }

    try {
        if ($Process.HasExited) {
            return $true
        }
    } catch {
        return $true
    }

    $treePids = Get-DescendantProcessIds -RootPid $Process.Id
    $graceful = $false

    try {
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) {
            $null = $Process.CloseMainWindow()
        }
        $graceful = $Process.WaitForExit(5000)
    } catch {
        $graceful = $false
    }

    if (-not $graceful) {
        foreach ($procId in $treePids) {
            if ($procId -ne $Process.Id) {
                Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
            }
        }
        try {
            if (-not $Process.HasExited) {
                Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
                $null = $Process.WaitForExit(3000)
            }
        } catch {
            # Ignore already-exited race
        }
    } else {
        # Clean up any orphaned child processes (e.g. delayed WebView2 teardown)
        Start-Sleep -Milliseconds 300
        foreach ($procId in $treePids) {
            if ($procId -ne $Process.Id) {
                $child = Get-Process -Id $procId -ErrorAction SilentlyContinue
                if ($null -ne $child) {
                    Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
                }
            }
        }
    }

    return $graceful
}

function Measure-ClientDemo {
    param(
        [string]$Name,
        [string]$ExePath,
        [string]$BuildHint,
        [int]$SampleSeconds,
        [int]$StartupTimeoutSeconds,
        [bool]$KeepDataDir
    )

    $resolvedPath = $ExePath
    if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
        Write-Warning "[$Name] Binary not found at '$ExePath'. $BuildHint"
        return [ordered]@{
            client              = $Name
            executable          = $resolvedPath
            status              = "missing"
            coldStartMs         = $null
            mainWorkingSetMB    = $null
            mainPrivateMemoryMB = $null
            treeWorkingSetMB    = $null
            treePrivateMemoryMB = $null
            peakTreeWorkingSetMB = $null
            childProcesses      = $null
            gracefulExit        = $null
            note                = "Binary missing. $BuildHint"
        }
    }

    $resolvedPath = (Resolve-Path -LiteralPath $ExePath).Path
    $tempDataDir = Join-Path ([System.IO.Path]::GetTempPath()) ("kitemail-bench-" + [Guid]::NewGuid().ToString("N"))
    $null = New-Item -ItemType Directory -Path $tempDataDir -Force

    $proc = $null
    try {
        $psi = [System.Diagnostics.ProcessStartInfo]::new()
        $psi.FileName = $resolvedPath
        $psi.Arguments = "--demo"
        $psi.WorkingDirectory = Split-Path -Parent $resolvedPath
        $psi.UseShellExecute = $false
        $psi.EnvironmentVariables["KITEMAIL_DATA_DIR"] = $tempDataDir
        $psi.EnvironmentVariables["KITEMAIL_PREVIEW_DATA_DIR"] = $tempDataDir
        $psi.EnvironmentVariables["KITEMAIL_DEMO"] = "1"

        $proc = [System.Diagnostics.Process]::new()
        $proc.StartInfo = $psi

        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        if (-not $proc.Start()) {
            throw "Failed to start process: $resolvedPath"
        }

        $windowReady = $false
        $exitedEarly = $false

        while ($sw.Elapsed.TotalSeconds -lt $StartupTimeoutSeconds) {
            if ($proc.HasExited) {
                $exitedEarly = $true
                break
            }
            $proc.Refresh()
            if ($proc.MainWindowHandle -ne [IntPtr]::Zero) {
                $windowReady = $true
                break
            }
            Start-Sleep -Milliseconds 50
        }

        $sw.Stop()
        $coldStartMs = [Math]::Round($sw.Elapsed.TotalMilliseconds, 1)

        if ($exitedEarly) {
            Write-Warning "[$Name] Process exited early (exit code $($proc.ExitCode)) before MainWindowHandle appeared. Ensure no other single-instance KiteMail process is already running."
            return [ordered]@{
                client               = $Name
                executable           = $resolvedPath
                status               = "exited_early"
                coldStartMs          = $null
                mainWorkingSetMB     = $null
                mainPrivateMemoryMB  = $null
                treeWorkingSetMB     = $null
                treePrivateMemoryMB  = $null
                peakTreeWorkingSetMB = $null
                childProcesses       = $null
                gracefulExit         = $true
                note                 = "Exited with code $($proc.ExitCode) before main window appeared (possible single-instance conflict)."
            }
        }

        if (-not $windowReady) {
            $null = Stop-BenchmarkProcess -Process $proc
            Write-Warning "[$Name] Timed out after ${StartupTimeoutSeconds}s waiting for MainWindowHandle."
            return [ordered]@{
                client               = $Name
                executable           = $resolvedPath
                status               = "timeout"
                coldStartMs          = $null
                mainWorkingSetMB     = $null
                mainPrivateMemoryMB  = $null
                treeWorkingSetMB     = $null
                treePrivateMemoryMB  = $null
                peakTreeWorkingSetMB = $null
                childProcesses       = $null
                gracefulExit         = $false
                note                 = "Timed out waiting for MainWindowHandle (${StartupTimeoutSeconds}s)."
            }
        }

        $samples = [System.Collections.Generic.List[object]]::new()
        for ($i = 0; $i -lt $SampleSeconds; $i++) {
            Start-Sleep -Seconds 1
            if ($proc.HasExited) {
                break
            }
            $sample = Get-ProcessTreeMemorySample -RootPid $proc.Id
            if ($null -ne $sample) {
                $samples.Add($sample)
            }
        }

        if ($samples.Count -eq 0) {
            $fallback = Get-ProcessTreeMemorySample -RootPid $proc.Id
            if ($null -ne $fallback) {
                $samples.Add($fallback)
            }
        }

        $gracefulExit = Stop-BenchmarkProcess -Process $proc

        if ($samples.Count -eq 0) {
            return [ordered]@{
                client               = $Name
                executable           = $resolvedPath
                status               = "no_samples"
                coldStartMs          = $coldStartMs
                mainWorkingSetMB     = $null
                mainPrivateMemoryMB  = $null
                treeWorkingSetMB     = $null
                treePrivateMemoryMB  = $null
                peakTreeWorkingSetMB = $null
                childProcesses       = $null
                gracefulExit         = $gracefulExit
                note                 = "Window appeared, but process exited before memory sampling completed."
            }
        }

        $avgMainWs = ($samples | Measure-Object -Property mainWorkingSetBytes -Average).Average
        $avgMainPriv = ($samples | Measure-Object -Property mainPrivateBytes -Average).Average
        $avgTreeWs = ($samples | Measure-Object -Property treeWorkingSetBytes -Average).Average
        $avgTreePriv = ($samples | Measure-Object -Property treePrivateBytes -Average).Average
        $peakTreeWs = ($samples | Measure-Object -Property treeWorkingSetBytes -Maximum).Maximum
        $maxChildren = ($samples | Measure-Object -Property childProcessCount -Maximum).Maximum

        return [ordered]@{
            client               = $Name
            executable           = $resolvedPath
            status               = "ok"
            coldStartMs          = $coldStartMs
            mainWorkingSetMB     = ConvertTo-Megabytes $avgMainWs
            mainPrivateMemoryMB  = ConvertTo-Megabytes $avgMainPriv
            treeWorkingSetMB     = ConvertTo-Megabytes $avgTreeWs
            treePrivateMemoryMB  = ConvertTo-Megabytes $avgTreePriv
            peakTreeWorkingSetMB = ConvertTo-Megabytes $peakTreeWs
            childProcesses       = [int]$maxChildren
            gracefulExit         = $gracefulExit
            note                 = "Sampled $($samples.Count)s in --demo mode"
        }
    } finally {
        if ($null -ne $proc) {
            $null = Stop-BenchmarkProcess -Process $proc
            $proc.Dispose()
        }
        if (-not $KeepDataDir -and (Test-Path -LiteralPath $tempDataDir)) {
            Remove-Item -LiteralPath $tempDataDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

$targets = @(
    @{
        Name      = "GPUI (kitemail)"
        ExePath   = $GpuiExe
        BuildHint = "Build with: cargo build -p kitemail --bin kitemail --release --target x86_64-pc-windows-msvc --locked"
    },
    @{
        Name      = "Tauri+Vue (kitemail-tauri-preview)"
        ExePath   = $TauriExe
        BuildHint = "Build with: pnpm tauri build --target x86_64-pc-windows-msvc --no-bundle"
    }
)

$results = @()
foreach ($target in $targets) {
    $results += Measure-ClientDemo `
        -Name $target.Name `
        -ExePath $target.ExePath `
        -BuildHint $target.BuildHint `
        -SampleSeconds $SampleSeconds `
        -StartupTimeoutSeconds $StartupTimeoutSeconds `
        -KeepDataDir ([bool]$KeepDataDir)
}

$report = [ordered]@{
    generatedAt           = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    machineName           = $env:COMPUTERNAME
    sampleSeconds         = $SampleSeconds
    startupTimeoutSeconds = $StartupTimeoutSeconds
    results               = $results
}

Write-Output ""
Write-Output "## KiteMail Demo Benchmark Comparison"
Write-Output ""
Write-Output "| Client | Status | Cold Start (ms) | Main WS (MB) | Main Private (MB) | Tree WS (MB) | Tree Private (MB) | Peak Tree WS (MB) | Child Procs | Graceful Exit |"
Write-Output "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"

foreach ($r in $results) {
    $cold = Format-NullableNumber -Value $r.coldStartMs -Decimals 1
    $mainWs = Format-NullableNumber -Value $r.mainWorkingSetMB
    $mainPriv = Format-NullableNumber -Value $r.mainPrivateMemoryMB
    $treeWs = Format-NullableNumber -Value $r.treeWorkingSetMB
    $treePriv = Format-NullableNumber -Value $r.treePrivateMemoryMB
    $peakWs = Format-NullableNumber -Value $r.peakTreeWorkingSetMB
    $children = if ($null -eq $r.childProcesses) { "-" } else { [string]$r.childProcesses }
    $graceful = if ($null -eq $r.gracefulExit) { "-" } elseif ($r.gracefulExit) { "yes" } else { "no" }
    Write-Output "| $($r.client) | $($r.status) | $cold | $mainWs | $mainPriv | $treeWs | $treePriv | $peakWs | $children | $graceful |"
}

$json = $report | ConvertTo-Json -Depth 6

if ($JsonOutPath) {
    $jsonDir = Split-Path -Parent $JsonOutPath
    if ($jsonDir -and -not (Test-Path -LiteralPath $jsonDir)) {
        $null = New-Item -ItemType Directory -Path $jsonDir -Force
    }
    Set-Content -LiteralPath $JsonOutPath -Value $json -Encoding UTF8
}

Write-Output ""
Write-Output "## JSON Result"
Write-Output ""
Write-Output $json

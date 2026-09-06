# Windows half of the `burrow-engine` test fixture. Same contract as the POSIX `burrow-engine`
# script beside it - read that one for what each branch is reproducing and why.
# Kept strictly ASCII, deliberately: the `.cmd` shim runs this through Windows PowerShell 5.1
# with `-File`, which decodes a BOM-less script as the machine's ANSI codepage rather than UTF-8.
# The one non-ASCII character this used to hold was an em dash inside this comment, so it only
# ever mojibaked harmlessly - but the next one need not be in a comment. ASCII needs no BOM and
# reads identically under every codepage, so nothing here depends on the runner's locale.
$cmd = if ($args.Count -gt 0) { $args[0] } else { '' }
$items = @($args | ForEach-Object { $_ | ConvertTo-Json -Compress })
$invoked = '[' + ($items -join ',') + ']'

function Refuse($message) {
    Write-Output ('{"ok":false,"burrow_cli":"0.1.0","engine":"burrow-engine","command":"' + $cmd + '","error":{"kind":"error","message":"' + $message + '","platform":"fixture"}}')
    exit 2
}

if ($args -contains '--raw')            { Refuse "unknown $cmd option: --raw" }
if ($args -contains '--watch-interval') { Refuse "unknown $cmd option: --watch-interval" }

$watch    = $args -contains '--watch'
$interval = $args -contains '--interval'
$progress = $args -contains '--progress'
$stream   = $args -contains '--stream'

# Each stream flag belongs to exactly the commands that implement it; elsewhere it is unknown.
if ($watch -and $cmd -ne 'status')       { Refuse "unknown $cmd option: --watch" }
if ($interval -and $cmd -ne 'status')    { Refuse "unknown $cmd option: --interval" }
if ($interval -and -not $watch)          { Refuse "--interval only applies with --watch" }
if ($progress -and $cmd -ne 'analyze')   { Refuse "unknown $cmd option: --progress" }
if ($stream -and $cmd -ne 'clean' -and $cmd -ne 'optimize' -and $cmd -ne 'purge') {
    Refuse "unknown $cmd option: --stream"
}

$apply = $args -contains '--apply'
$dry   = ($args -contains '--dry-run') -or ($args -contains '-n')
if ($apply -and $dry) {
    Refuse "$cmd cannot take both --apply and --dry-run: --apply removes files and --dry-run guarantees it will not. Pass one."
}

if ($cmd -eq 'uninstall' -and ($args -contains '--list')) {
    Write-Output '[{"name":"Fixture","bundle_id":"com.fixture.App","source":"App"}]'
    exit 0
}

# clean/optimize/purge --stream: NDJSON, one line per item, no envelope.
if ($stream) {
    if ($apply) {
        Write-Output '{"event":"removed","path":"/fixture/Caches/A","bytes":4}'
        Write-Output '{"event":"removed","path":"/fixture/Caches/B","bytes":8}'
        Write-Output ('{"event":"done","freed_bytes":12,"freed_human":"12 B","moved_to_trash_bytes":12,"moved_to_trash_human":"12 B","removed":2,"failed":0,"protected":0,"invoked":' + $invoked + '}')
    } else {
        Write-Output '{"event":"would_remove","path":"/fixture/Caches/A","bytes":4}'
        Write-Output '{"event":"would_remove","path":"/fixture/Caches/B","bytes":8}'
        Write-Output ('{"event":"done","dry_run":true,"would_free_bytes":12,"would_free_human":"12 B","count":2,"invoked":' + $invoked + '}')
    }
    exit 0
}

$statusData = '{"collected_at":"2026-06-25T10:30:00Z","host":"fixture","health_score":92,"cpu":{"usage":12.4},"invoked":' + $invoked + '}'

# status --watch: one `status` data object per tick, no envelope, BURROW_WATCH_FRAMES ticks.
if ($watch) {
    $frames = 2
    if ($env:BURROW_WATCH_FRAMES) { $frames = [int]$env:BURROW_WATCH_FRAMES }
    for ($i = 0; $i -lt $frames; $i++) { Write-Output $statusData }
    exit 0
}

# analyze --progress <path>: progress frames, then the result - or the error envelope, exit 1.
if ($progress) {
    $path = ''
    foreach ($a in ($args | Select-Object -Skip 1)) {
        if (-not $a.StartsWith('-') -and $path -eq '') { $path = $a }
    }
    # ConvertTo-Json yields a quoted, escaped JSON string (backslashes included).
    $jpath = $path | ConvertTo-Json -Compress
    $jsub  = { param($s) ($path + $s) | ConvertTo-Json -Compress }
    if ($path -eq '' -or -not (Test-Path -LiteralPath $path)) {
        Write-Output ('{"ok":false,"burrow_cli":"0.1.0","engine":"burrow-engine","command":"analyze","error":{"kind":"error","message":' + (('scan ' + $path + ': No such file or directory') | ConvertTo-Json -Compress) + ',"platform":"fixture"}}')
        exit 1
    }
    Write-Output ('{"type":"progress","files":10,"dirs":2,"bytes":4096,"path":' + (& $jsub '/a') + '}')
    Write-Output ('{"type":"progress","files":20,"dirs":4,"bytes":8192,"path":' + (& $jsub '/b') + '}')
    Write-Output ('{"type":"result","data":{"path":' + $jpath + ',"overview":false,"entries":[],"total_size":8192}}')
    exit 0
}

switch ($cmd) {
    'status'  { $data = $statusData }
    'analyze' { $data = '{"path":"/","overview":true,"entries":[],"total_size":0,"invoked":' + $invoked + '}' }
    default   { $data = '{"invoked":' + $invoked + '}' }
}
Write-Output ('{"ok":true,"burrow_cli":"0.1.0","engine":"burrow-engine","command":"' + $cmd + '","data":' + $data + '}')

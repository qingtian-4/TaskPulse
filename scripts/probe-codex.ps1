param([string]$RemoteHost, [switch]$Pipeline)
$ErrorActionPreference = 'Stop'
if ($RemoteHost) {
    $probe = @'
import glob, os, sqlite3, json
paths = sorted(glob.glob(os.path.expanduser('~/.codex/sessions/**/rollout-*.jsonl'), recursive=True), key=os.path.getmtime, reverse=True)
dbs = glob.glob(os.path.expanduser('~/.codex/state_*.sqlite'))
for db in dbs:
    c = sqlite3.connect('file:' + db + '?mode=ro', uri=True)
    cols = [r[1] for r in c.execute('pragma table_info(threads)')]
    print(json.dumps({'db': os.path.basename(db), 'columns': cols}))
    fields = [f for f in ['id','name','title','cwd','updated_at','rollout_path'] if f in cols]
    for r in c.execute('select ' + ','.join(fields) + ' from threads order by updated_at desc limit 3'):
        print(json.dumps(dict(zip(fields, r)), ensure_ascii=False))
for p in paths[:3]:
    events = []
    with open(p, encoding='utf-8') as f:
        for line in f:
            try:
                o = json.loads(line)
            except ValueError:
                continue
            payload = o.get('payload', {})
            if o.get('type') == 'event_msg':
                events.append({'time':o.get('timestamp'), 'type':payload.get('type')})
    print(json.dumps({'file':os.path.basename(p), 'last_events':events[-8:]}))
'@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($probe))
    & ssh -o BatchMode=yes -o ConnectTimeout=5 $RemoteHost "echo $encoded | base64 -d | python3"
    exit $LASTEXITCODE
}
$binary = Get-ChildItem "$env:LOCALAPPDATA\OpenAI\Codex\bin" -Filter codex.exe -Recurse |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
$info = New-Object Diagnostics.ProcessStartInfo
$info.FileName = $binary
$info.Arguments = 'app-server --stdio'
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardInput = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$process = New-Object Diagnostics.Process
$process.StartInfo = $info
[void]$process.Start()
$stderr = $process.StandardError.ReadToEndAsync()
try {
    $process.StandardInput.WriteLine('{"id":1,"method":"initialize","params":{"clientInfo":{"name":"taskpulse_probe","version":"0.1"},"capabilities":{"experimentalApi":true}}}')
    if ($Pipeline) {
        $process.StandardInput.WriteLine('{"method":"initialized","params":{}}')
        $process.StandardInput.WriteLine('{"id":2,"method":"thread/list","params":{"limit":100,"sortKey":"updated_at","sortDirection":"desc","archived":false,"useStateDbOnly":false}}')
        $process.StandardInput.WriteLine('{"id":3,"method":"account/rateLimits/read","params":null}')
    }
    $pending = $process.StandardOutput.ReadLineAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $requested = [bool]$Pipeline
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not $pending.Wait(100)) { continue }
        $line = $pending.Result
        if ($null -eq $line) { break }
        $reply = $line | ConvertFrom-Json
        if ($reply.id -eq 1 -and -not $requested) {
            if ($reply.error) { Write-Output $line; break }
            $process.StandardInput.WriteLine('{"method":"initialized","params":{}}')
            $process.StandardInput.WriteLine('{"id":3,"method":"account/rateLimits/read","params":null}')
            $requested = $true
        }
        if ($reply.id -eq 2) { @{ method='thread/list'; error=$reply.error; count=$reply.result.data.Count } | ConvertTo-Json -Compress }
        if ($reply.id -eq 3) {
            @{ method='account/rateLimits/read'; observed_at=[DateTime]::UtcNow.ToString('o'); error=$reply.error; rateLimits=$reply.result.rateLimits } | ConvertTo-Json -Depth 8 -Compress
            break
        }
        $pending = $process.StandardOutput.ReadLineAsync()
    }
} finally {
    if (-not $process.HasExited) { $process.Kill() }
    $process.WaitForExit()
}

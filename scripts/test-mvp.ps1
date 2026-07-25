# Multi-process M1 smoke: TCP-only bootstrap, bidirectional CRDT dataflow,
# idempotency, and datastore isolation.
# Usage: from the repository root:
#   powershell -NoProfile -File scripts/test-mvp.ps1

$ErrorActionPreference = "Stop"

$repo = [System.IO.Path]::GetFullPath((Get-Location).Path)
if (-not (Test-Path -LiteralPath (Join-Path $repo "Cargo.toml"))) {
  throw "run this script from the ZeroDB repository root"
}

$target = [System.IO.Path]::GetFullPath((Join-Path $repo "target"))
$root = [System.IO.Path]::GetFullPath((Join-Path $target "mvp-test"))
$targetPrefix = $target.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
if (-not $root.StartsWith($targetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "unsafe smoke-test root: $root"
}
if ([System.IO.Path]::GetFileName($root) -ne "mvp-test") {
  throw "refusing to reset unexpected smoke-test root: $root"
}

Write-Host "building zerodb..."
cargo build -p zerodb-cli
if ($LASTEXITCODE -ne 0) { throw "cargo build -p zerodb-cli failed" }

$z = Join-Path $target "debug\zerodb.exe"
if (-not (Test-Path -LiteralPath $z)) { throw "missing CLI after build: $z" }

if (Test-Path -LiteralPath $root) {
  Remove-Item -LiteralPath $root -Recurse -Force
}
New-Item -ItemType Directory -Path $root | Out-Null

$servers = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()

function Invoke-ZeroText {
  param([Parameter(Mandatory = $true)][string[]]$ZeroArgs)

  $lines = & $script:z @ZeroArgs
  $code = $LASTEXITCODE
  $text = ($lines | Out-String).Trim()
  if ($code -ne 0) {
    throw "zerodb $($ZeroArgs -join ' ') failed with exit $code`: $text"
  }
  return $text
}

function Start-ZeroServer {
  param(
    [Parameter(Mandatory = $true)][string]$Database,
    [Parameter(Mandatory = $true)][int]$Port
  )

  $listen = "127.0.0.1:$Port"
  $process = Start-Process -FilePath $script:z -ArgumentList @(
    "serve", "--path", $Database, "--listen", $listen
  ) -PassThru -WindowStyle Hidden
  $script:servers.Add($process) | Out-Null

  for ($attempt = 0; $attempt -lt 50; $attempt++) {
    $process.Refresh()
    if ($process.HasExited) {
      throw "zerodb serve exited before listening on $listen (exit $($process.ExitCode))"
    }

    $client = [System.Net.Sockets.TcpClient]::new()
    $connected = $false
    try {
      $pending = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
      if ($pending.AsyncWaitHandle.WaitOne(100)) {
        $client.EndConnect($pending)
        $connected = $true
      }
    } catch {
      $connected = $false
    } finally {
      $client.Dispose()
    }

    if ($connected) { return $process }
    Start-Sleep -Milliseconds 100
  }

  throw "timed out waiting for zerodb serve on $listen"
}

function Stop-ZeroServer {
  param([System.Diagnostics.Process]$Process)

  if ($null -eq $Process) { return }
  try {
    $Process.Refresh()
    if (-not $Process.HasExited) {
      Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
      $Process.WaitForExit(5000) | Out-Null
    }
  } catch {
    # The outer cleanup is best-effort; the test assertion remains primary.
  }
}

function Invoke-Pull {
  param(
    [Parameter(Mandatory = $true)][string]$Database,
    [Parameter(Mandatory = $true)][int]$Port,
    [Parameter(Mandatory = $true)][int]$Accepted
  )

  $output = Invoke-ZeroText @("pull", "--path", $Database, "--from", "127.0.0.1:$Port")
  $expected = "accepted=$Accepted skipped=0"
  if ($output -notmatch [regex]::Escape($expected)) {
    throw "unexpected pull result; wanted '$expected', got '$output'"
  }
  return $output
}

function Get-Inspect {
  param([Parameter(Mandatory = $true)][string]$Database)

  return (Invoke-ZeroText @("inspect", "--path", $Database) | ConvertFrom-Json)
}

function Get-NormalizedInspect {
  param([Parameter(Mandatory = $true)][string]$Database)

  $report = Get-Inspect $Database
  return ([ordered]@{
    datastore_id = $report.datastore_id
    ops = [int64]$report.ops
    nodes = $report.nodes
  } | ConvertTo-Json -Compress -Depth 32)
}

function Assert-Value {
  param(
    [Parameter(Mandatory = $true)][string]$Database,
    [Parameter(Mandatory = $true)][string]$Node,
    [Parameter(Mandatory = $true)][string]$Key,
    [Parameter(Mandatory = $true)][string]$Expected
  )

  $actual = Invoke-ZeroText @("get", "--path", $Database, "--node", $Node, "--key", $Key)
  if ($actual -ne $Expected) {
    throw "unexpected $Key in $Database`: expected '$Expected', got '$actual'"
  }
}

try {
  # The documented file-sync bootstrap starts with one populated datastore and
  # one separately initialized empty datastore. Both the first merge and an
  # idempotent repeat must succeed and leave normalized state identical.
  $syncSource = Join-Path $root "sync-source.sqlite"
  $syncPeer = Join-Path $root "sync-peer.sqlite"
  Invoke-ZeroText @("init", "--path", $syncSource) | Out-Null
  Invoke-ZeroText @("init", "--path", $syncPeer) | Out-Null
  $syncNode = Invoke-ZeroText @("create-node", "--path", $syncSource, "--label", "SyncProbe")
  Invoke-ZeroText @("set", "--path", $syncSource, "--node", $syncNode, "--key", "title", "--value", "seed") | Out-Null

  $syncFirst = Invoke-ZeroText @("sync", "--path", $syncSource, "--peer", $syncPeer)
  $syncSourceState = Get-NormalizedInspect $syncSource
  $syncPeerState = Get-NormalizedInspect $syncPeer
  if ($syncSourceState -ne $syncPeerState) {
    throw "file-sync bootstrap did not converge: source=$syncSourceState peer=$syncPeerState"
  }

  $syncSecond = Invoke-ZeroText @("sync", "--path", $syncSource, "--peer", $syncPeer)
  $syncSourceState = Get-NormalizedInspect $syncSource
  $syncPeerState = Get-NormalizedInspect $syncPeer
  if ($syncSourceState -ne $syncPeerState) {
    throw "idempotent file sync changed convergence: source=$syncSourceState peer=$syncPeerState"
  }

  # Select an otherwise ordinary six-op source whose OpId-sorted TCP stream is
  # adversarial: a property arrives before CreateNode. Export is used only to
  # prove the test precondition; B receives all data solely over TCP.
  $a = $null
  $node = $null
  for ($attempt = 1; $attempt -le 64; $attempt++) {
    $candidate = Join-Path $root "a-$attempt.sqlite"
    $orderProbe = Join-Path $root "a-$attempt-order.json"
    Invoke-ZeroText @("init", "--path", $candidate) | Out-Null
    $candidateNode = Invoke-ZeroText @("create-node", "--path", $candidate, "--label", "Todo")
    Invoke-ZeroText @("set", "--path", $candidate, "--node", $candidateNode, "--key", "title", "--value", "seed") | Out-Null
    Invoke-ZeroText @("inc", "--path", $candidate, "--node", $candidateNode, "--key", "score", "--n", "5") | Out-Null
    Invoke-ZeroText @("inc", "--path", $candidate, "--node", $candidateNode, "--key", "views", "--n", "2", "--kind", "g") | Out-Null
    Invoke-ZeroText @("set-add", "--path", $candidate, "--node", $candidateNode, "--key", "tags", "--value", "base") | Out-Null
    Invoke-ZeroText @("flag-on", "--path", $candidate, "--node", $candidateNode, "--key", "done") | Out-Null
    Invoke-ZeroText @("export", "--path", $candidate, "--out", $orderProbe) | Out-Null

    $firstWireOp = ((Get-Content -Raw -LiteralPath $orderProbe | ConvertFrom-Json).ops |
      Sort-Object -Property id |
      Select-Object -First 1)
    if ([int64]$firstWireOp.kind -ne 1) {
      $a = $candidate
      $node = $candidateNode
      break
    }
  }
  if ($null -eq $a) { throw "could not construct adversarial property-before-create TCP order" }

  $b = Join-Path $root "b.sqlite"
  $foreign = Join-Path $root "foreign.sqlite"
  Invoke-ZeroText @("init", "--path", $b) | Out-Null

  $server = Start-ZeroServer -Database $a -Port 17701
  try {
    $bootstrap = Invoke-Pull -Database $b -Port 17701 -Accepted 6
  } finally {
    Stop-ZeroServer $server
  }

  $bootstrapA = Get-NormalizedInspect $a
  $bootstrapB = Get-NormalizedInspect $b

  # Concurrent writes on both peers cover every currently exposed property
  # CRDT. Pull is a two-way session: one pull converges both peers, so the
  # reversed pull afterwards must be a no-op.
  Invoke-ZeroText @("set", "--path", $a, "--node", $node, "--key", "title", "--value", "from-a") | Out-Null
  Invoke-ZeroText @("inc", "--path", $a, "--node", $node, "--key", "score", "--n", "4") | Out-Null
  Invoke-ZeroText @("inc", "--path", $a, "--node", $node, "--key", "views", "--n", "3", "--kind", "g") | Out-Null
  Invoke-ZeroText @("set-add", "--path", $a, "--node", $node, "--key", "tags", "--value", "local-a") | Out-Null
  Invoke-ZeroText @("flag-on", "--path", $a, "--node", $node, "--key", "done") | Out-Null

  Invoke-ZeroText @("set", "--path", $b, "--node", $node, "--key", "title", "--value", "from-b") | Out-Null
  Invoke-ZeroText @("dec", "--path", $b, "--node", $node, "--key", "score", "--n", "2") | Out-Null
  Invoke-ZeroText @("inc", "--path", $b, "--node", $node, "--key", "views", "--n", "4", "--kind", "g") | Out-Null
  Invoke-ZeroText @("set-add", "--path", $b, "--node", $node, "--key", "tags", "--value", "local-b") | Out-Null
  Invoke-ZeroText @("flag-off", "--path", $b, "--node", $node, "--key", "done") | Out-Null

  $server = Start-ZeroServer -Database $b -Port 17702
  try {
    $aFromB = Invoke-Pull -Database $a -Port 17702 -Accepted 5
  } finally {
    Stop-ZeroServer $server
  }

  $server = Start-ZeroServer -Database $a -Port 17703
  try {
    $bFromA = Invoke-Pull -Database $b -Port 17703 -Accepted 0
  } finally {
    Stop-ZeroServer $server
  }

  $server = Start-ZeroServer -Database $b -Port 17704
  try {
    $idempotentA = Invoke-Pull -Database $a -Port 17704 -Accepted 0
  } finally {
    Stop-ZeroServer $server
  }

  $server = Start-ZeroServer -Database $a -Port 17705
  try {
    $idempotentB = Invoke-Pull -Database $b -Port 17705 -Accepted 0
  } finally {
    Stop-ZeroServer $server
  }

  $inspectA = Get-Inspect $a
  $inspectB = Get-Inspect $b
  if ($inspectA.datastore_id -ne $inspectB.datastore_id) { throw "datastore IDs diverged" }
  if ($inspectA.peer -eq $inspectB.peer) { throw "peer identities unexpectedly match" }
  if ([int64]$inspectA.ops -ne 16 -or [int64]$inspectB.ops -ne 16) {
    throw "unexpected converged op counts: $($inspectA.ops) / $($inspectB.ops)"
  }

  Assert-Value -Database $a -Node $node -Key "score" -Expected "7"
  Assert-Value -Database $b -Node $node -Key "score" -Expected "7"
  Assert-Value -Database $a -Node $node -Key "views" -Expected "9"
  Assert-Value -Database $b -Node $node -Key "views" -Expected "9"
  Assert-Value -Database $a -Node $node -Key "tags" -Expected '["base","local-a","local-b"]'
  Assert-Value -Database $b -Node $node -Key "tags" -Expected '["base","local-a","local-b"]'
  Assert-Value -Database $a -Node $node -Key "done" -Expected "true"
  Assert-Value -Database $b -Node $node -Key "done" -Expected "true"
  $titleA = Invoke-ZeroText @("get", "--path", $a, "--node", $node, "--key", "title")
  $titleB = Invoke-ZeroText @("get", "--path", $b, "--node", $node, "--key", "title")
  if ($titleA -ne $titleB) { throw "LWW title did not converge: '$titleA' / '$titleB'" }

  $normalizedA = Get-NormalizedInspect $a
  $normalizedB = Get-NormalizedInspect $b
  if ($normalizedA -ne $normalizedB) {
    throw "bidirectional TCP materialized different state (including label): A=$normalizedA B=$normalizedB"
  }

  # Both peers have observed the complete 16-op state. Remove the seed tag and
  # disable every currently live flag dot, exchange both causal operations in a
  # single two-way pull, and prove another pair of pulls is idempotent.
  Invoke-ZeroText @("set-remove", "--path", $a, "--node", $node, "--key", "tags", "--value", "base") | Out-Null
  Invoke-ZeroText @("flag-off", "--path", $b, "--node", $node, "--key", "done") | Out-Null

  $server = Start-ZeroServer -Database $b -Port 17706
  try {
    $cleanupA = Invoke-Pull -Database $a -Port 17706 -Accepted 1
  } finally {
    Stop-ZeroServer $server
  }

  $server = Start-ZeroServer -Database $a -Port 17707
  try {
    $cleanupB = Invoke-Pull -Database $b -Port 17707 -Accepted 0
  } finally {
    Stop-ZeroServer $server
  }

  $server = Start-ZeroServer -Database $b -Port 17708
  try {
    $cleanupIdempotentA = Invoke-Pull -Database $a -Port 17708 -Accepted 0
  } finally {
    Stop-ZeroServer $server
  }

  $server = Start-ZeroServer -Database $a -Port 17709
  try {
    $cleanupIdempotentB = Invoke-Pull -Database $b -Port 17709 -Accepted 0
  } finally {
    Stop-ZeroServer $server
  }

  $cleanupInspectA = Get-Inspect $a
  $cleanupInspectB = Get-Inspect $b
  if ([int64]$cleanupInspectA.ops -ne 18 -or [int64]$cleanupInspectB.ops -ne 18) {
    throw "unexpected causal-cleanup op counts: $($cleanupInspectA.ops) / $($cleanupInspectB.ops)"
  }
  Assert-Value -Database $a -Node $node -Key "score" -Expected "7"
  Assert-Value -Database $b -Node $node -Key "score" -Expected "7"
  Assert-Value -Database $a -Node $node -Key "views" -Expected "9"
  Assert-Value -Database $b -Node $node -Key "views" -Expected "9"
  Assert-Value -Database $a -Node $node -Key "tags" -Expected '["local-a","local-b"]'
  Assert-Value -Database $b -Node $node -Key "tags" -Expected '["local-a","local-b"]'
  Assert-Value -Database $a -Node $node -Key "done" -Expected "false"
  Assert-Value -Database $b -Node $node -Key "done" -Expected "false"

  # Each inspect/replay invocation opens a fresh process and connection. Capture
  # the restart state, replay both oplogs, and require byte-for-byte normalized
  # equivalence plus the value oracle after replay.
  $restartA = Get-NormalizedInspect $a
  $restartB = Get-NormalizedInspect $b
  if ($restartA -ne $restartB) {
    throw "causal cleanup did not converge after reopen: A=$restartA B=$restartB"
  }
  $replayA = Invoke-ZeroText @("replay", "--path", $a)
  $replayB = Invoke-ZeroText @("replay", "--path", $b)
  $replayedA = Get-NormalizedInspect $a
  $replayedB = Get-NormalizedInspect $b
  if ($replayedA -ne $restartA -or $replayedB -ne $restartB -or $replayedA -ne $replayedB) {
    throw "restart/replay changed normalized state: beforeA=$restartA beforeB=$restartB afterA=$replayedA afterB=$replayedB"
  }
  Assert-Value -Database $a -Node $node -Key "score" -Expected "7"
  Assert-Value -Database $b -Node $node -Key "score" -Expected "7"
  Assert-Value -Database $a -Node $node -Key "views" -Expected "9"
  Assert-Value -Database $b -Node $node -Key "views" -Expected "9"
  Assert-Value -Database $a -Node $node -Key "tags" -Expected '["local-a","local-b"]'
  Assert-Value -Database $b -Node $node -Key "tags" -Expected '["local-a","local-b"]'
  Assert-Value -Database $a -Node $node -Key "done" -Expected "false"
  Assert-Value -Database $b -Node $node -Key "done" -Expected "false"

  # A nonempty foreign datastore must reject the pull without adopting the
  # remote identity or accepting any operation.
  Invoke-ZeroText @("init", "--path", $foreign) | Out-Null
  Invoke-ZeroText @("create-node", "--path", $foreign, "--label", "Foreign") | Out-Null
  $foreignBefore = Get-NormalizedInspect $foreign
  $server = Start-ZeroServer -Database $a -Port 17710
  try {
    $savedPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
      $foreignLines = & $z pull --path $foreign --from "127.0.0.1:17710" 2>&1
      $foreignExit = $LASTEXITCODE
      $foreignOutput = ($foreignLines | Out-String).Trim()
    } finally {
      $ErrorActionPreference = $savedPreference
    }
  } finally {
    Stop-ZeroServer $server
  }
  if ($foreignExit -eq 0) { throw "foreign datastore pull unexpectedly succeeded" }
  if ($foreignOutput -notmatch "datastore mismatch") {
    throw "unexpected foreign datastore error: $foreignOutput"
  }
  $foreignAfter = Get-NormalizedInspect $foreign
  if ($foreignAfter -ne $foreignBefore) {
    throw "foreign datastore mutated after rejected pull: before=$foreignBefore after=$foreignAfter"
  }

  Write-Host "OK bootstrap    -> $bootstrap"
  Write-Host "OK file sync    -> first: $syncFirst; second: $syncSecond"
  Write-Host "OK A <- B       -> $aFromB"
  Write-Host "OK B <- A       -> $bFromA"
  Write-Host "OK idempotent A -> $idempotentA"
  Write-Host "OK idempotent B -> $idempotentB"
  Write-Host "OK values       -> title=$titleA score=7 views=9 tags=[base,local-a,local-b] done=true"
  Write-Host "OK causal A     -> $cleanupA"
  Write-Host "OK causal B     -> $cleanupB"
  Write-Host "OK causal idem  -> A: $cleanupIdempotentA; B: $cleanupIdempotentB"
  Write-Host "OK replay       -> A: $replayA; B: $replayB; tags=[local-a,local-b] done=false"
  Write-Host "OK isolation    -> foreign datastore rejected without mutation"

  if ($bootstrapA -ne $bootstrapB) {
    throw "TCP bootstrap materialized different state (including label): A=$bootstrapA B=$bootstrapB"
  }
  Write-Host "MVP multi-process TCP smoke passed."
} finally {
  foreach ($process in $servers) {
    Stop-ZeroServer $process
  }
}

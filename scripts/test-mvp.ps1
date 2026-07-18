# Multi-process MVP smoke: file sync + localhost TCP pull.
# Usage: from repo root, after `cargo build -p zerodb-cli`
#   powershell -File scripts/test-mvp.ps1

$ErrorActionPreference = "Stop"
$z = Join-Path (Get-Location) "target\debug\zerodb.exe"
if (-not (Test-Path $z)) {
  Write-Host "building zerodb..."
  cargo build -p zerodb-cli
}
$root = "target\mvp-test"
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $root | Out-Null

& $z init --path "$root\a.sqlite" | Out-Null
& $z init --path "$root\b.sqlite" | Out-Null

$node = (& $z create-node --path "$root\a.sqlite" --label Todo).Trim()
& $z set --path "$root\a.sqlite" --node $node --key title --value "oat milk" | Out-Null
& $z export --path "$root\a.sqlite" --out "$root\a.json" | Out-Null
& $z import --path "$root\b.sqlite" --file "$root\a.json" | Out-Null

& $z set --path "$root\b.sqlite" --node $node --key title --value "almond milk" | Out-Null
& $z set --path "$root\a.sqlite" --node $node --key title --value "soy milk" | Out-Null
& $z sync --path "$root\a.sqlite" --peer "$root\b.sqlite" | Out-Null

$ta = (& $z get --path "$root\a.sqlite" --node $node --key title).Trim()
$tb = (& $z get --path "$root\b.sqlite" --node $node --key title).Trim()
if ($ta -ne $tb) { throw "file sync not converged: $ta vs $tb" }
Write-Host "OK file sync  -> $ta"

& $z set --path "$root\b.sqlite" --node $node --key title --value "hemp milk" | Out-Null
$serve = Start-Process -FilePath $z -ArgumentList @("serve","--path","$root\b.sqlite","--listen","127.0.0.1:17701") -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 1
try {
  & $z pull --path "$root\a.sqlite" --from "127.0.0.1:17701" | Out-Null
} finally {
  Stop-Process -Id $serve.Id -Force -ErrorAction SilentlyContinue
}
$ta2 = (& $z get --path "$root\a.sqlite" --node $node --key title).Trim()
$tb2 = (& $z get --path "$root\b.sqlite" --node $node --key title).Trim()
if ($ta2 -ne $tb2) { throw "tcp pull not converged: $ta2 vs $tb2" }
Write-Host "OK tcp pull   -> $ta2"
Write-Host "MVP multi-process smoke passed."

# M1 Windows ↔ Raspberry Pi LAN test

This runbook verifies the experimental M1 data path between a Windows x64 development host and a Raspberry Pi 5 on one trusted private network. The protocol is plaintext and does not authenticate the peer or encrypt metadata. Use disposable databases, bind each server to its specific private-LAN address, and remove the temporary firewall rule afterward.

## Local release gate

Run this on Windows from the candidate branch before copying anything to the Pi:

```powershell
cargo fmt --all -- --check
cargo test --workspace
node conformance/ts/runner.mjs
powershell -NoProfile -File scripts/test-mvp.ps1
cargo build --locked --release -p zerodb-cli
```

Record the commit SHA and create a self-contained bundle so both machines build the **same** commit (do not pin a branch name):

```powershell
$Sha = git rev-parse HEAD
git bundle create target/zero-m1-lan-$Sha.bundle $Sha
scp target/zero-m1-lan-$Sha.bundle <pi-user>@<pi-ip>:/tmp/
```

On the Pi, build the native `aarch64` executable from that bundle:

```bash
test "$(uname -m)" = "aarch64"
rustup show active-toolchain
rustc --version
sudo apt-get update
sudo apt-get install -y build-essential pkg-config jq
# set BUNDLE and EXPECT_SHA to the values recorded on Windows
CANDIDATE=$(mktemp -d /tmp/zero-m1-lan-candidate.XXXXXX)
git clone "$BUNDLE" "$CANDIDATE"
cd "$CANDIDATE"
git checkout "$EXPECT_SHA"
test "$(git rev-parse HEAD)" = "$EXPECT_SHA"
cargo test --workspace
cargo build --locked --release -p zerodb-cli
```

The two `git rev-parse HEAD` values must match. A Windows executable cannot be copied to and run on the Pi; build natively as shown.

## Network preparation

Use the actual private addresses reported by `ipconfig` on Windows and `hostname -I` on the Pi. The examples below use `192.168.1.12` for Windows and `192.168.1.70` for the Pi.

Before opening a port, verify all of the following:

- Both addresses are RFC1918 private addresses on the intended LAN, not VPN or overlay interfaces.
- `Find-NetRoute -RemoteIPAddress 192.168.1.70` on Windows and `ip route get 192.168.1.12` on the Pi select those LAN interfaces.
- The hosts can ping each other; if they cannot, disable Wi-Fi client isolation or move both hosts to the same trusted LAN.
- Port 7700 is not already listening (`Get-NetTCPConnection -State Listen -LocalPort 7700` on Windows and `ss -ltn 'sport = :7700'` on the Pi should return nothing).
- Their UTC clocks differ by less than 30 seconds, leaving margin inside ZeroDB's 60-second ingress limit. One simple check is to compare Windows `[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()` with `ssh <pi-user>@192.168.1.70 'date +%s'`.

In an elevated Windows PowerShell, allow only the Pi to reach the temporary test port on the Private firewall profile:

```powershell
New-NetFirewallRule -DisplayName "ZeroDB M1 LAN 7700" `
  -Direction Inbound -Action Allow -Protocol TCP -LocalPort 7700 `
  -LocalAddress 192.168.1.12 -RemoteAddress 192.168.1.70 -Profile Private
```

Do not use `0.0.0.0`, a public address, port forwarding, or public Wi-Fi for this test.

If `sudo ufw status` reports that UFW is active on the Pi, add an equally narrow rule before the Pi serves:

```bash
sudo ufw allow from 192.168.1.12 to any port 7700 proto tcp comment 'ZeroDB M1 LAN 7700'
```

## Bootstrap over TCP

On Windows:

```powershell
$Zero = Resolve-Path .\target\release\zerodb.exe
$RunDir = Join-Path (Resolve-Path .\target) ("lan-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
New-Item -ItemType Directory -Path $RunDir | Out-Null
$A = Join-Path $RunDir "lan-a.sqlite"
& $Zero init --path $A
$Node = (& $Zero create-node --path $A --label Todo).Trim()
& $Zero set --path $A --node $Node --key title --value seed
& $Zero inc --path $A --node $Node --key score --n 5
& $Zero inc --path $A --node $Node --key views --n 2 --kind g
& $Zero set-add --path $A --node $Node --key tags --value base
& $Zero flag-on --path $A --node $Node --key done
& $Zero serve --path $A --listen 192.168.1.12:7700 --allow-insecure-lan
```

Leave the Windows server running. On the Pi, initialize an empty database and bootstrap it only through TCP:

```bash
ZERO=./target/release/zerodb
DATA_DIR=$(mktemp -d /tmp/zero-m1-lan-data.XXXXXX)
B="$DATA_DIR/lan-b.sqlite"
printf 'Pi data directory: %s\n' "$DATA_DIR"
$ZERO init --path "$B"
$ZERO pull --path "$B" --from 192.168.1.12:7700
$ZERO inspect --path "$B"
```

The pull must report `accepted=6`, and the Pi must show the `Todo` node with all five property values. Stop the Windows server after the pull.

## Independent edits and two-way convergence

On Windows, make five local changes:

```powershell
& $Zero set --path $A --node $Node --key title --value from-a
& $Zero inc --path $A --node $Node --key score --n 4
& $Zero inc --path $A --node $Node --key views --n 3 --kind g
& $Zero set-add --path $A --node $Node --key tags --value local-a
& $Zero flag-on --path $A --node $Node --key done
```

On the Pi, use the same node ID and make five independent changes:

```bash
NODE=<node-id-printed-on-windows>
$ZERO set --path "$B" --node "$NODE" --key title --value from-b
$ZERO dec --path "$B" --node "$NODE" --key score --n 2
$ZERO inc --path "$B" --node "$NODE" --key views --n 4 --kind g
$ZERO set-add --path "$B" --node "$NODE" --key tags --value local-b
$ZERO flag-off --path "$B" --node "$NODE" --key done
$ZERO serve --path "$B" --listen 192.168.1.70:7700 --allow-insecure-lan
```

While the Pi serves, pull its five operations on Windows:

```powershell
& $Zero pull --path $A --from 192.168.1.70:7700
```

Stop the Pi server. Serve Windows again, then pull on the Pi:

```powershell
& $Zero serve --path $A --listen 192.168.1.12:7700 --allow-insecure-lan
```

```bash
$ZERO pull --path "$B" --from 192.168.1.12:7700
```

Both databases must now contain 16 operations and agree on `score=7`, `views=9`, tags `base`, `local-a`, and `local-b`, and the same deterministic LWW title. The concurrent enable/disable resolves to `done=true`.

Next exercise causal removals after both peers have observed the merged state. On Windows, remove the original tag; on the Pi, disable the now-observed flag enables:

```powershell
& $Zero set-remove --path $A --node $Node --key tags --value base
```

```bash
$ZERO flag-off --path "$B" --node "$NODE" --key done
```

Exchange operations once in each direction again. Both peers must reach 18 operations with `score=7`, `views=9`, tags `local-a` and `local-b` only, and `done=false`. Repeat both serving/pulling directions; both duplicate pulls must report `accepted=0` and leave normalized `inspect` output unchanged.

First serve the Pi again and pull on Windows:

```bash
$ZERO serve --path "$B" --listen 192.168.1.70:7700 --allow-insecure-lan
```

```powershell
& $Zero pull --path $A --from 192.168.1.70:7700
```

Stop the Pi server. Then serve Windows and pull on the Pi:

```powershell
& $Zero serve --path $A --listen 192.168.1.12:7700 --allow-insecure-lan
```

```bash
$ZERO pull --path "$B" --from 192.168.1.12:7700
```

Stop the Windows server. Repeat those same two serve/pull pairs once more. Each pull must report `accepted=0`.

Capture each peer before replay. Raw reports intentionally have different `path` and `peer` fields, so those are the only fields removed during comparison:

```powershell
& $Zero inspect --path $A | Set-Content -Encoding utf8 (Join-Path $RunDir "windows-pre-replay.json")
& $Zero replay --path $A
& $Zero inspect --path $A | Set-Content -Encoding utf8 (Join-Path $RunDir "windows-post-replay.json")
$PiDataDir = "<DATA_DIR printed by the Pi's mktemp command>"
scp (Join-Path $RunDir "windows-pre-replay.json") "<pi-user>@192.168.1.70:$PiDataDir/"
scp (Join-Path $RunDir "windows-post-replay.json") "<pi-user>@192.168.1.70:$PiDataDir/"
```

```bash
$ZERO inspect --path "$B" > "$DATA_DIR/pi-pre-replay.json"
$ZERO replay --path "$B"
$ZERO inspect --path "$B" > "$DATA_DIR/pi-post-replay.json"
for report in windows-pre-replay windows-post-replay pi-pre-replay pi-post-replay; do
  jq -S 'del(.path, .peer)' "$DATA_DIR/$report.json" > "$DATA_DIR/$report.normalized.json"
done
diff -u "$DATA_DIR/windows-pre-replay.normalized.json" "$DATA_DIR/pi-pre-replay.normalized.json"
diff -u "$DATA_DIR/windows-pre-replay.normalized.json" "$DATA_DIR/windows-post-replay.normalized.json"
diff -u "$DATA_DIR/pi-pre-replay.normalized.json" "$DATA_DIR/pi-post-replay.normalized.json"
diff -u "$DATA_DIR/windows-post-replay.normalized.json" "$DATA_DIR/pi-post-replay.normalized.json"
```

All four diffs must be empty. They prove both convergence and per-peer replay stability.

Finally, create an independent nonempty database on the Pi:

```bash
FOREIGN="$DATA_DIR/foreign.sqlite"
$ZERO init --path "$FOREIGN"
$ZERO create-node --path "$FOREIGN" --label Foreign
$ZERO inspect --path "$FOREIGN" | jq -S 'del(.path, .peer)' > "$DATA_DIR/foreign-before.json"
```

Serve A once more on Windows, then attempt the foreign pull on the Pi:

```powershell
& $Zero serve --path $A --listen 192.168.1.12:7700 --allow-insecure-lan
```

```bash
set +e
FOREIGN_OUTPUT=$($ZERO pull --path "$FOREIGN" --from 192.168.1.12:7700 2>&1)
FOREIGN_STATUS=$?
set -e
printf '%s\n' "$FOREIGN_OUTPUT"
test "$FOREIGN_STATUS" -ne 0
$ZERO inspect --path "$FOREIGN" | jq -S 'del(.path, .peer)' > "$DATA_DIR/foreign-after.json"
diff -u "$DATA_DIR/foreign-before.json" "$DATA_DIR/foreign-after.json"
```

The pull must fail with a datastore mismatch and the diff must be empty. Stop the Windows server.

## Cleanup

Stop both servers and remove the narrowly scoped Windows firewall rule in elevated PowerShell:

```powershell
Remove-NetFirewallRule -DisplayName "ZeroDB M1 LAN 7700"
```

If a Pi UFW rule was added, remove that exact rule:

```bash
sudo ufw delete allow from 192.168.1.12 to any port 7700 proto tcp
```

Keep the two inspect reports, command output, commit ID, Windows target triple, and Pi target triple with the M1 test record. Do not push the branch until the local and live-LAN evidence has been reviewed.

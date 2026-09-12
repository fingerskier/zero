// Measure the wasm-pack artifact against ISSUES O4.
//
// O4 is pinned (Decision Log 2026-09-12): the Automerge ~250 KB / Loro
// ~200 KB gz figure is informational, not an M4a gate. This script records
// raw + gzip and fails only above the regression ceiling (300 KiB gz; the
// size-oriented artifact is ~262.6 KiB) so size cannot silently regress.

import fs from 'node:fs'
import path from 'node:path'
import zlib from 'node:zlib'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const wasmPath = path.join(here, '..', 'pkg', 'zerodb_wasm_bg.wasm')

if (!fs.existsSync(wasmPath)) {
  console.error('wasm artifact missing — run: zerodb-wasm/scripts/build.sh')
  process.exit(1)
}

const raw = fs.readFileSync(wasmPath)
const gz = zlib.gzipSync(raw, { level: 9 })
const O4_TARGET_GZIP = 250 * 1024
const REGRESSION_GZIP = 300 * 1024

const fmt = n => `${n} bytes (${(n / 1024).toFixed(1)} KiB)`
console.log(`zerodb_wasm_bg.wasm: ${fmt(raw.length)}`)
console.log(`gzip -9:             ${fmt(gz.length)}`)
console.log(`O4 target:           ${fmt(O4_TARGET_GZIP)} gzip (Automerge-comparable; informational)`)
console.log(`regression ceiling:  ${fmt(REGRESSION_GZIP)} gzip (CI gate; O4 pinned 2026-09-12)`)

if (gz.length > O4_TARGET_GZIP) {
  console.log('O4: above Automerge-comparable target (informational; O4 pinned)')
} else {
  console.log('O4: within Automerge-comparable target')
}

if (gz.length > REGRESSION_GZIP) {
  console.error(`WASM gzip ${gz.length} exceeds regression ceiling ${REGRESSION_GZIP}`)
  process.exit(1)
}

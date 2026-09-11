// Measure the wasm-pack artifact against ISSUES O4.
//
// O4 writes a *target* vs Automerge ~250 KB / Loro ~200 KB gz — no prior
// hard CI number. This slice records raw + gzip. The Automerge figure is
// informational. Fail only if gzip exceeds the pre-optimization artifact
// this crate already shipped (~393 KiB gz) so size cannot silently regress.

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
const REGRESSION_GZIP = 400 * 1024

const fmt = n => `${n} bytes (${(n / 1024).toFixed(1)} KiB)`
console.log(`zerodb_wasm_bg.wasm: ${fmt(raw.length)}`)
console.log(`gzip -9:             ${fmt(gz.length)}`)
console.log(`O4 target:           ${fmt(O4_TARGET_GZIP)} gzip (Automerge-comparable; informational)`)
console.log(`regression ceiling:  ${fmt(REGRESSION_GZIP)} gzip (pre-optimization artifact ~393 KiB)`)

if (gz.length > O4_TARGET_GZIP) {
  console.log('O4: above Automerge-comparable target (issue stays open)')
} else {
  console.log('O4: within Automerge-comparable target')
}

if (gz.length > REGRESSION_GZIP) {
  console.error(`WASM gzip ${gz.length} exceeds regression ceiling ${REGRESSION_GZIP}`)
  process.exit(1)
}

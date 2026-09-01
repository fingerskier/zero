import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const tsRoot = resolve(here, '..')

function walkFiles(dir, acc = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    const st = statSync(p)
    if (st.isDirectory()) walkFiles(p, acc)
    else if (name.endsWith('.mjs') || name.endsWith('.js') || name.endsWith('.json')) acc.push(p)
  }
  return acc
}

function staticImports(source) {
  const out = []
  const re = /(?:from|import)\s+['"]([^'"]+)['"]/g
  let m
  while ((m = re.exec(source))) out.push(m[1])
  return out
}

function walkImportGraph(entry) {
  const seen = new Set()
  const queue = [resolve(entry)]
  while (queue.length) {
    const file = queue.pop()
    if (seen.has(file)) continue
    seen.add(file)
    const src = readFileSync(file, 'utf8')
    for (const spec of staticImports(src)) {
      if (spec.startsWith('node:')) continue
      if (!spec.startsWith('.') && !spec.startsWith('/')) {
        if (spec.includes('zerodb-napi') || spec === 'zerodb-napi') {
          throw new Error(`${file} imports ${spec}`)
        }
        continue
      }
      const next = resolve(dirname(file), spec)
      queue.push(next)
    }
  }
  return seen
}

test('TS peer import graph does not include zerodb-napi', () => {
  const entries = ['index.mjs', 'cli.mjs', 'client.mjs', 'store.mjs', 'ws.mjs', 'crypto.mjs'].map((n) =>
    join(here, n),
  )
  const all = new Set()
  for (const entry of entries) {
    for (const f of walkImportGraph(entry)) all.add(f)
  }
  const banned = /(?:from|import|require)\s*\(?['"][^'"]*zerodb-napi[^'"]*['"]/
  for (const f of all) {
    assert.equal(f.includes('zerodb-napi'), false, `NAPI leaked into graph: ${f}`)
    const src = readFileSync(f, 'utf8')
    assert.equal(banned.test(src), false, `zerodb-napi import in ${f}`)
    assert.equal(/dlopen|createRequire\(|node-gyp/.test(src), false, `native load in ${f}`)
  }
  assert.ok([...all].some((f) => f.endsWith('models/relay.mjs')))
  assert.ok([...all].some((f) => f.endsWith('models/op.mjs')))
})

test('conformance/ts implementation files do not import the native addon', () => {
  const banned = /(?:from|import|require)\s*\(?['"][^'"]*zerodb-napi[^'"]*['"]/
  for (const file of walkFiles(tsRoot)) {
    if (file.endsWith('.test.mjs') || file.endsWith('.md')) continue
    const src = readFileSync(file, 'utf8')
    assert.equal(banned.test(src), false, file)
  }
})

/**
 * M2 parity vectors: exercise the 5 shipped CRDTs (LWW, GCounter, PNCounter,
 * ORSet, EWFlag) through the NAPI binding and assert materialized results
 * match the semantics established by the core test-suite
 * (zerodb-storage/tests/e1_e2_acceptance.rs and friends) and the conformance
 * CRDT vectors (conformance/vectors/required/crdt/*).
 *
 * What "parity" covers here:
 *   - single-store semantics: counter inc/dec accumulation, gcounter grow-only,
 *     orset add/remove/re-add, flag enable/disable, LWW last-write.
 *   - replay idempotence: re-importing the same bundle is a no-op (dedup).
 *   - one cross-peer convergence case per CRDT via exportJson/importJson in
 *     BOTH directions, including the interesting concurrent cases:
 *       * ORSet: concurrent remove vs re-add (add-wins / observed-remove)
 *       * EWFlag: disable only cancels observed enables (enable-wins for a
 *         concurrent unobserved enable)
 *
 * What this does NOT cover (full E11 parity remains open):
 *   - byte-level replay of the kernel conformance vectors
 *     (conformance/vectors/required/crdt/CRDT-*.json) through the binding —
 *     that needs a NAPI entry point that ingests raw encoded ops / vector
 *     traces, plus equivocation-exclusion and equal-timestamp total-order
 *     tie-break cases that require crafted op ids/HLC values not expressible
 *     through the current high-level API.
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Database } = require('../index.js')

const root = join(fileURLToPath(new URL('.', import.meta.url)), '../../target/m2-napi-test')

function tempDb(name) {
  mkdirSync(root, { recursive: true })
  return join(root, `${name}-${process.pid}-${Date.now()}-${Math.floor(Math.random() * 1e6)}.sqlite`)
}

function cleanup(path, ...dbs) {
  for (const db of dbs) {
    try {
      db?.close?.()
    } catch {
      /* ignore */
    }
  }
  for (const suffix of ['', '-wal', '-shm']) {
    try {
      rmSync(path + suffix, { force: true })
    } catch {
      /* ignore */
    }
  }
}

/** Two stores on the same datastore: B bootstraps by importing A's bundle. */
function twinStores(name) {
  const pathA = tempDb(`${name}-a`)
  const pathB = tempDb(`${name}-b`)
  const a = Database.init(pathA)
  const b = Database.init(pathB)
  return { a, b, pathA, pathB }
}

function syncBoth(a, b) {
  const ra = b.importJson(a.exportJson())
  const rb = a.importJson(b.exportJson())
  return { ra, rb }
}

test('parity: PNCounter inc/dec accumulation and cross-peer convergence', () => {
  const { a, b, pathA, pathB } = twinStores('pn')
  try {
    const node = a.createNode('Counter')
    a.counterInc(node, 'score', 5)
    a.counterDec(node, 'score', 2)
    assert.equal(a.getProp(node, 'score'), 3) // core: inc/dec accumulate (CRDT-PN-001)

    syncBoth(a, b)
    // concurrent: A +4, B -1 +3
    a.counterInc(node, 'score', 4)
    b.counterDec(node, 'score', 1)
    b.counterInc(node, 'score', 3)
    syncBoth(a, b)
    assert.equal(a.getProp(node, 'score'), 9)
    assert.equal(b.getProp(node, 'score'), 9)
  } finally {
    cleanup(pathA, a)
    cleanup(pathB, b)
  }
})

test('parity: GCounter grow-only accumulation and replay dedup', () => {
  const { a, b, pathA, pathB } = twinStores('gc')
  try {
    const node = a.createNode('Counter')
    a.gcounterInc(node, 'views', 2)
    a.gcounterInc(node, 'views', 1)
    assert.equal(a.getProp(node, 'views'), 3)

    const bundle = a.exportJson()
    const first = b.importJson(bundle)
    assert.ok(first.accepted >= 1)
    // replay dedup: importing the same bundle again accepts nothing (CRDT-GC-001)
    const again = b.importJson(bundle)
    assert.equal(again.accepted, 0)
    assert.ok(again.skipped >= 1)
    assert.equal(b.getProp(node, 'views'), 3)

    // concurrent increments merge additively
    a.gcounterInc(node, 'views', 10)
    b.gcounterInc(node, 'views', 4)
    syncBoth(a, b)
    assert.equal(a.getProp(node, 'views'), 17)
    assert.equal(b.getProp(node, 'views'), 17)
  } finally {
    cleanup(pathA, a)
    cleanup(pathB, b)
  }
})

test('parity: ORSet add/remove/re-add and concurrent remove vs re-add', () => {
  const { a, b, pathA, pathB } = twinStores('orset')
  try {
    const node = a.createNode('Todo')
    a.setAdd(node, 'tags', 'urgent')
    a.setRemove(node, 'tags', 'urgent')
    assert.deepEqual(a.getProp(node, 'tags'), []) // observed remove clears
    a.setAdd(node, 'tags', 'urgent')
    assert.deepEqual(a.getProp(node, 'tags'), ['urgent']) // re-add after remove (CRDT-ORSET-002)

    a.setAdd(node, 'tags', 'food')
    syncBoth(a, b)
    assert.deepEqual([...b.getProp(node, 'tags')].sort(), ['food', 'urgent'])

    // concurrent: B removes 'food' (observed), A re-adds 'food' with a new tag
    // (unobserved by B's remove) -> add wins (CRDT-ORSET-001 semantics)
    b.setRemove(node, 'tags', 'food')
    a.setAdd(node, 'tags', 'food')
    syncBoth(a, b)
    assert.deepEqual([...a.getProp(node, 'tags')].sort(), ['food', 'urgent'])
    assert.deepEqual([...b.getProp(node, 'tags')].sort(), ['food', 'urgent'])
  } finally {
    cleanup(pathA, a)
    cleanup(pathB, b)
  }
})

test('parity: EWFlag enable/disable with observed dots', () => {
  const { a, b, pathA, pathB } = twinStores('flag')
  try {
    const node = a.createNode('Todo')
    a.flagEnable(node, 'done')
    assert.equal(a.getProp(node, 'done'), true)
    a.flagDisable(node, 'done')
    assert.equal(a.getProp(node, 'done'), false) // observed disable (CRDT-FLAG-002)
    a.flagEnable(node, 'done')
    assert.equal(a.getProp(node, 'done'), true)

    syncBoth(a, b)
    assert.equal(b.getProp(node, 'done'), true)

    // concurrent: B disables (observes A's enable), A enables again with a
    // fresh dot B has not observed -> enable wins (CRDT-FLAG-001)
    b.flagDisable(node, 'done')
    a.flagEnable(node, 'done')
    syncBoth(a, b)
    assert.equal(a.getProp(node, 'done'), true)
    assert.equal(b.getProp(node, 'done'), true)
  } finally {
    cleanup(pathA, a)
    cleanup(pathB, b)
  }
})

test('parity: LWW last write wins and converges both directions', () => {
  const { a, b, pathA, pathB } = twinStores('lww')
  try {
    const node = a.createNode('Todo')
    a.setLww(node, 'title', 'first')
    a.setLww(node, 'title', 'second')
    assert.equal(a.getLww(node, 'title'), 'second') // last write within one peer

    syncBoth(a, b)
    // sequential cross-peer: B writes strictly after observing A's write
    // (higher HLC) -> B's value wins on both replicas
    b.setLww(node, 'title', 'from-b')
    syncBoth(a, b)
    assert.equal(a.getLww(node, 'title'), 'from-b')
    assert.equal(b.getLww(node, 'title'), 'from-b')

    // concurrent writes: both replicas must converge to the SAME value
    // (deterministic HLC/total-order tie-break; exact winner depends on
    // timestamps so parity asserts agreement, not a specific value)
    a.setLww(node, 'title', 'race-a')
    b.setLww(node, 'title', 'race-b')
    syncBoth(a, b)
    const va = a.getLww(node, 'title')
    const vb = b.getLww(node, 'title')
    assert.equal(va, vb)
    assert.ok(va === 'race-a' || va === 'race-b')
  } finally {
    cleanup(pathA, a)
    cleanup(pathB, b)
  }
})

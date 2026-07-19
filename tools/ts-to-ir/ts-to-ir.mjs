#!/usr/bin/env node
/**
 * Minimal O2 trail: TypeScript-ish authoring JSON → simplified schema IR JSON
 * consumed by `zerodb schema-apply`.
 *
 * Authoring input (JSON for M1; full TS AST later):
 * {
 *   "name": "todo",
 *   "nodes": {
 *     "Todo": {
 *       "title": { "crdt": "lww", "type": "text" },
 *       "done":  { "crdt": "flag", "type": "bool" }
 *     }
 *   }
 * }
 *
 * Output pin format for LocalStore:
 * { "nodes": { "Todo": { "props": { "title": "lww", "done": "flag" } } } }
 */

import { readFileSync, writeFileSync } from 'node:fs'

const CRDT = new Set(['lww', 'gcounter', 'pncounter', 'orset', 'flag'])

export function compile(authoring) {
  if (!authoring || typeof authoring !== 'object') {
    throw new Error('authoring root must be an object')
  }
  const nodesIn = authoring.nodes
  if (!nodesIn || typeof nodesIn !== 'object') {
    throw new Error('authoring.nodes required')
  }
  const nodes = {}
  for (const [label, entity] of Object.entries(nodesIn)) {
    const propsIn = entity?.props ?? entity
    if (!propsIn || typeof propsIn !== 'object') {
      throw new Error(`nodes.${label}: expected props map`)
    }
    const props = {}
    for (const [path, def] of Object.entries(propsIn)) {
      if (path === 'props') continue
      const crdt = typeof def === 'string' ? def : def?.crdt
      if (!CRDT.has(crdt)) {
        throw new Error(`nodes.${label}.${path}: invalid crdt ${crdt}`)
      }
      if (def && typeof def === 'object' && def.unique === true) {
        throw new Error(`nodes.${label}.${path}: unique is not representable in v0.1 (DQ-10)`)
      }
      props[path] = crdt
    }
    nodes[label] = { props }
  }
  return {
    name: authoring.name ?? null,
    nodes,
  }
}

function main(argv) {
  const args = argv.slice(2)
  if (args.length < 1) {
    console.error('usage: zerodb-ts-to-ir <authoring.json> [-o out.json]')
    process.exit(2)
  }
  const input = args[0]
  let out = null
  for (let i = 1; i < args.length; i++) {
    if (args[i] === '-o') out = args[++i]
  }
  const authoring = JSON.parse(readFileSync(input, 'utf8'))
  const ir = compile(authoring)
  const text = JSON.stringify(ir, null, 2) + '\n'
  if (out) writeFileSync(out, text)
  else process.stdout.write(text)
}

import { pathToFileURL } from 'node:url'
const entry = process.argv[1] && pathToFileURL(process.argv[1]).href
if (entry && import.meta.url === entry) {
  try {
    main(process.argv)
  } catch (e) {
    console.error(String(e.message || e))
    process.exit(1)
  }
}

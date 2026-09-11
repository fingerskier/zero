#!/usr/bin/env node
// Generate wire JSON Schemas + a limits snapshot from registry.json.
// The registry is the H9 machine-readable RELAY 0.2 protocol definition.
// Draft-1 / unfrozen — this does not freeze wrap-body or claim a format freeze.
//
// Usage:
//   node conformance/generate-protocol.mjs          # write schemas/
//   node conformance/generate-protocol.mjs --check  # fail if generated files drift

import { readFileSync, writeFileSync, mkdirSync, readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const registry = JSON.parse(readFileSync(join(here, 'registry.json'), 'utf8'))
const wire = registry.relay_wire
if (!wire || !wire.messages) {
  console.error('registry.json missing relay_wire.messages')
  process.exit(2)
}

const outDir = join(here, 'schemas')
mkdirSync(outDir, { recursive: true })

function envelopeSchema() {
  return {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    $id: 'zerodb-relay-envelope',
    title: 'RELAY 0.2.2-draft envelope (draft-1, unfrozen)',
    type: 'object',
    additionalProperties: false,
    required: ['type', 'request_id', 'payload'],
    properties: {
      type: { type: 'integer', minimum: 0, maximum: 255 },
      request_id: { type: 'integer', minimum: 0 },
      payload: { type: 'object' },
      dir: { type: 'string', enum: ['P→R', 'R→P'] },
      cbor_hex: { type: 'string', pattern: '^[0-9a-f]*$' },
    },
  }
}

function messageSchemas() {
  const defs = {}
  const oneOf = []
  for (const [name, spec] of Object.entries(wire.messages)) {
    const properties = {}
    for (const key of spec.required) {
      properties[key] = { description: `${name}.${key}` }
    }
    defs[name] = {
      type: 'object',
      title: name,
      description: `RELAY ${name} (0x${Number(spec.type).toString(16)})`,
      required: spec.required,
      properties,
      'x-wire-type': spec.type,
      'x-dir': spec.dir,
      'x-responses': spec.responses || [],
    }
    oneOf.push({ $ref: `#/$defs/${name}` })
  }
  return {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    $id: 'zerodb-relay-messages',
    title: 'RELAY 0.2.2-draft message payloads (generated from registry.relay_wire)',
    $defs: defs,
    oneOf,
    'x-byte-fields': wire.byte_fields,
  }
}

function limitsSchema() {
  return {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    $id: 'zerodb-relay-welcome-limits',
    title: 'Advertised WELCOME.limits (RELAY-SPEC §8.1) vs format limits (KERNEL/O6)',
    welcome_limits: wire.welcome_limits,
    format_limits: registry.limits,
    errors: wire.errors,
    peer_rejects: registry.peer_rejects,
    domain_handshake: wire.domain_handshake,
    capabilities: registry.relay_capabilities.tokens,
  }
}

function transcriptSchema() {
  return {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    $id: 'zerodb-relay-transcript-vector',
    title: 'relay-transcript vector (ordered envelopes + kind)',
    type: 'object',
    required: ['id', 'type', 'kind', 'frames'],
    properties: {
      id: { type: 'string' },
      type: { const: 'relay-transcript' },
      kind: {
        type: 'string',
        enum: ['handshake', 'dual-root', 'resume', 'reject-ack', 'ops-push', 'merkle-walk', 'limits'],
      },
      frames: { type: 'array', minItems: 1, items: { $ref: 'zerodb-relay-envelope' } },
    },
  }
}

const files = {
  'relay-envelope.json': envelopeSchema(),
  'relay-messages.json': messageSchemas(),
  'relay-limits.json': limitsSchema(),
  'relay-transcript.json': transcriptSchema(),
}

const check = process.argv.includes('--check')
let drift = 0
for (const [name, body] of Object.entries(files)) {
  const text = `${JSON.stringify(body, null, 2)}\n`
  const path = join(outDir, name)
  if (check) {
    let existing
    try {
      existing = readFileSync(path, 'utf8')
    } catch {
      console.error(`missing generated ${name}`)
      drift += 1
      continue
    }
    if (existing !== text) {
      console.error(`generated ${name} is stale — re-run node conformance/generate-protocol.mjs`)
      drift += 1
    }
  } else {
    writeFileSync(path, text)
  }
}

if (check) {
  const expected = new Set(Object.keys(files))
  for (const name of readdirSync(outDir)) {
    if (name.endsWith('.json') && !expected.has(name)) {
      console.error(`unexpected schema file ${name}`)
      drift += 1
    }
  }
  if (drift) process.exit(1)
  console.log(`[generate-protocol] ${expected.size} schemas match registry.json`)
} else {
  console.log(`[generate-protocol] wrote ${Object.keys(files).length} schemas under conformance/schemas/`)
}

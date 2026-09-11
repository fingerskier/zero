// Load conformance/registry.json — the H9 machine-readable protocol definition.
// A runner whose constants disagree with this file fails.

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const path = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'registry.json')

export const registry = JSON.parse(readFileSync(path, 'utf8'))
export const relayWire = registry.relay_wire
export const peerRejects = registry.peer_rejects

export function messageType(name) {
  const spec = relayWire.messages[name]
  if (!spec) throw new Error(`unknown relay message ${name}`)
  return spec.type
}

export function errorCode(name) {
  const spec = relayWire.errors[name]
  if (!spec) throw new Error(`unknown relay error ${name}`)
  return spec.code
}

export function requiredKeys(typeCode) {
  for (const spec of Object.values(relayWire.messages)) {
    if (spec.type === typeCode) return spec.required
  }
  return []
}

export function knownMessageTypes() {
  return new Set(Object.values(relayWire.messages).map((m) => m.type))
}

export function fixedDirection(typeCode) {
  for (const spec of Object.values(relayWire.messages)) {
    if (spec.type === typeCode) return spec.dir || null
  }
  return null
}

export function expectedResponses(typeCode) {
  for (const spec of Object.values(relayWire.messages)) {
    if (spec.type === typeCode) {
      return (spec.responses || []).map((name) => messageType(name))
    }
  }
  return []
}

/** Fail if hardcoded constants disagree with the registry. */
export function assertRelayConstants(got) {
  const caps = registry.relay_capabilities.tokens
  if (JSON.stringify(got.capabilities) !== JSON.stringify(caps)) {
    throw new Error(`RELAY_CAPS ${JSON.stringify(got.capabilities)} != registry ${JSON.stringify(caps)}`)
  }
  const limits = relayWire.welcome_limits
  for (const [k, v] of Object.entries(got.welcomeLimits)) {
    if (limits[k] !== v) {
      throw new Error(`welcome limit ${k}=${v} != registry ${limits[k]}`)
    }
  }
  for (const name of Object.keys(relayWire.messages)) {
    if (!Object.prototype.hasOwnProperty.call(got.messages, name)) {
      throw new Error(`MSG_${name} missing from runner (registry Appendix A)`)
    }
  }
  for (const [name, ty] of Object.entries(got.messages)) {
    const want = messageType(name)
    if (ty !== want) throw new Error(`MSG_${name} ${ty} != registry ${want}`)
  }
  if (got.requiredKeys) {
    for (const [name, spec] of Object.entries(relayWire.messages)) {
      const gotKeys = JSON.stringify(got.requiredKeys(spec.type))
      const want = JSON.stringify(spec.required)
      if (gotKeys !== want) {
        throw new Error(`${name} required keys ${gotKeys} != registry ${want}`)
      }
    }
  }
  for (const [name, code] of Object.entries(got.errors || {})) {
    const want = errorCode(name)
    if (code !== want) throw new Error(`ERR_${name} ${code} != registry ${want}`)
  }
  const bytes = relayWire.byte_fields
  if (JSON.stringify([...got.byteFields].sort()) !== JSON.stringify([...bytes].sort())) {
    throw new Error(`BYTE_FIELDS ${JSON.stringify(got.byteFields)} != registry ${JSON.stringify(bytes)}`)
  }
  if (got.domainHandshake !== relayWire.domain_handshake) {
    throw new Error(`handshake domain ${got.domainHandshake} != ${relayWire.domain_handshake}`)
  }
  if (got.maxDriftMs != null && got.maxDriftMs !== peerRejects.max_drift_ms) {
    throw new Error(`maxDriftMs ${got.maxDriftMs} != registry ${peerRejects.max_drift_ms}`)
  }
}

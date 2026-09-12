// DTLS channel binding for the DataChannel profile (RELAY-SPEC §14.2, H5
// slice). Works on real RTCPeerConnection descriptions (`.sdp` strings)
// and on the fake ones in channel.mjs — both carry the standard
// `a=fingerprint:sha-256 AA:BB:...` line.

import { channelBinding } from '../models/relay.mjs'

/** Render a 32-byte fingerprint as the SDP attribute line. */
export function formatFingerprint(fp) {
  if (!(fp instanceof Uint8Array) || fp.length !== 32) throw new Error('fingerprint must be 32 bytes')
  const hex = Array.from(fp, (b) => b.toString(16).padStart(2, '0').toUpperCase())
  return `a=fingerprint:sha-256 ${hex.join(':')}`
}

/**
 * Extract the SHA-256 DTLS certificate fingerprint from an SDP string or a
 * description object. Fails closed on a missing line, a non-sha-256
 * algorithm, or a malformed digest — never "no binding".
 */
export function dtlsFingerprintFromSdp(sdpOrDesc) {
  const sdp = typeof sdpOrDesc === 'string' ? sdpOrDesc : sdpOrDesc && sdpOrDesc.sdp
  if (typeof sdp !== 'string') throw new Error('SDP missing')
  const m = /^a=fingerprint:([A-Za-z0-9-]+)\s+([0-9A-Fa-f:]+)\s*$/m.exec(sdp)
  if (!m) throw new Error('SDP has no a=fingerprint line')
  if (m[1].toLowerCase() !== 'sha-256') throw new Error(`unsupported DTLS fingerprint algorithm ${m[1]}`)
  const parts = m[2].split(':')
  if (parts.length !== 32 || parts.some((p) => !/^[0-9A-Fa-f]{2}$/.test(p))) {
    throw new Error('malformed sha-256 fingerprint')
  }
  return Uint8Array.from(parts, (p) => parseInt(p, 16))
}

/** `HELLO.channel_binding` from this side's local and remote descriptions. */
export function channelBindingFromDescriptions(local, remote) {
  return channelBinding(dtlsFingerprintFromSdp(local), dtlsFingerprintFromSdp(remote))
}

/** Convenience over a (real or fake) RTCPeerConnection after negotiation. */
export function channelBindingFor(pc) {
  if (!pc || !pc.localDescription || !pc.remoteDescription) {
    throw new Error('peer connection has no local+remote description yet')
  }
  return channelBindingFromDescriptions(pc.localDescription, pc.remoteDescription)
}

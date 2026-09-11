// In-process ordered/reliable DataChannel double (RELAY-SPEC §14.2).
// Not libwebrtc / wrtc. CI must not need public STUN/TURN.

export const CHANNEL_LABEL = 'zerodb-relay'

function asBytes(data) {
  if (data instanceof Uint8Array) return data
  if (data instanceof ArrayBuffer) return new Uint8Array(data)
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
  throw new Error('DataChannel message must be binary')
}

export class FakeDataChannel {
  constructor(label = CHANNEL_LABEL) {
    this.label = label
    this.ordered = true
    this.reliable = true
    this.readyState = 'connecting'
    this.onmessage = null
    this.onopen = null
    this._peer = null
  }

  send(data) {
    if (this.readyState !== 'open') throw new Error('DataChannel is not open')
    const bytes = asBytes(data)
    const sink = this._peer
    queueMicrotask(() => {
      if (!sink || sink.readyState !== 'open') return
      if (sink.onmessage) sink.onmessage({ data: bytes })
    })
  }

  close() {
    this.readyState = 'closed'
  }

  _open() {
    this.readyState = 'open'
    if (this.onopen) this.onopen()
  }
}

/** Pair two already-created channels (tests that skip fake SDP). */
export function pairDataChannels(a, b) {
  a._peer = b
  b._peer = a
  a._open()
  b._open()
}

/**
 * Minimal RTCPeerConnection stand-in. Offer/answer/ICE are opaque JSON
 * strings for SIGNAL. After both sides have local+remote descriptions,
 * `connectFakeRtc` wires the `zerodb-relay` DataChannel.
 */
export class FakeRTCPeerConnection {
  constructor() {
    this.localDescription = null
    this.remoteDescription = null
    this.onicecandidate = null
    this.ondatachannel = null
    this._channel = null
    this._iceSeq = 0
  }

  createDataChannel(label, opts = {}) {
    if (label !== CHANNEL_LABEL) {
      throw new Error(`RELAY-SPEC §14.2 channel label is ${CHANNEL_LABEL}`)
    }
    if (opts.ordered === false) throw new Error('DataChannel must be ordered')
    this._channel = new FakeDataChannel(label)
    return this._channel
  }

  async createOffer() {
    return { type: 'offer', sdp: 'zerodb-fake-sdp-offer' }
  }

  async createAnswer() {
    return { type: 'answer', sdp: 'zerodb-fake-sdp-answer' }
  }

  async setLocalDescription(desc) {
    this.localDescription = desc
    const emit = this.onicecandidate
    if (!emit) return
    queueMicrotask(() => {
      this._iceSeq += 1
      emit({ candidate: { candidate: `fake-ice-${this._iceSeq}`, sdpMid: '0' } })
      emit({ candidate: null })
    })
  }

  async setRemoteDescription(desc) {
    this.remoteDescription = desc
  }

  async addIceCandidate(_candidate) {
    // Opaque to this double. Real ICE is not this slice.
  }
}

/** After SIGNAL has exchanged offer/answer, attach the ordered channel. */
export function connectFakeRtc(offerer, answerer) {
  if (!offerer._channel) {
    throw new Error('initiator must createDataChannel before the offer')
  }
  if (!answerer._channel) {
    answerer._channel = new FakeDataChannel(CHANNEL_LABEL)
    if (answerer.ondatachannel) {
      answerer.ondatachannel({ channel: answerer._channel })
    }
  }
  pairDataChannels(offerer._channel, answerer._channel)
  return { initiator: offerer._channel, answerer: answerer._channel }
}

export class ChannelTransport {
  constructor(channel) {
    this.channel = channel
    this.queue = []
    this.waiters = []
    channel.onmessage = (ev) => {
      const bytes = asBytes(ev.data)
      if (this.waiters.length) this.waiters.shift()(bytes)
      else this.queue.push(bytes)
    }
  }

  send(frame) {
    this.channel.send(frame)
  }

  async recv() {
    if (this.queue.length) return this.queue.shift()
    return await new Promise((resolve) => {
      this.waiters.push(resolve)
    })
  }
}

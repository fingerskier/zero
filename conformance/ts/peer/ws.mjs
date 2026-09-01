// Binary WebSocket transport for RELAY 0.2. One WS message = one CBOR envelope.
// Uses Node ≥22 global WebSocket. MUST NOT import zerodb-napi.

import { repliesComplete } from '../models/relay.mjs'

const REQUEST_TIMEOUT_MS = 30_000

function asBytes(data) {
  if (data instanceof Uint8Array) return data
  if (data instanceof ArrayBuffer) return new Uint8Array(data)
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
  throw new Error('expected binary WebSocket frame')
}

export class WsTransport {
  constructor(url) {
    this.url = url
    this.ws = null
    this.queue = []
    this.waiters = []
  }

  async connect() {
    const ws = new WebSocket(this.url)
    ws.binaryType = 'arraybuffer'
    this.ws = ws
    ws.addEventListener('message', (ev) => {
      const bytes = asBytes(ev.data)
      if (this.waiters.length) this.waiters.shift()(bytes)
      else this.queue.push(bytes)
    })
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`WebSocket connect timeout ${this.url}`)), REQUEST_TIMEOUT_MS)
      ws.addEventListener('open', () => {
        clearTimeout(timer)
        resolve()
      })
      ws.addEventListener('error', () => {
        clearTimeout(timer)
        reject(new Error(`WebSocket error ${this.url}`))
      })
    })
    return this
  }

  async readBinary() {
    if (this.queue.length) return this.queue.shift()
    return await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('WebSocket read timeout')), REQUEST_TIMEOUT_MS)
      this.waiters.push((bytes) => {
        clearTimeout(timer)
        resolve(bytes)
      })
    })
  }

  sendBinary(frame) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error('WebSocket is not open')
    }
    this.ws.send(frame)
  }

  /** Send one envelope and drain reply frames until `repliesComplete`. */
  async request(frame) {
    this.sendBinary(frame)
    const replies = []
    while (!repliesComplete(frame, replies)) {
      replies.push(await this.readBinary())
    }
    return replies
  }

  close() {
    try {
      this.ws?.close()
    } catch {
      /* ignore */
    }
    this.ws = null
  }
}

export async function connectRelay(url) {
  const t = new WsTransport(url)
  await t.connect()
  return t
}

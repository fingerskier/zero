// Framework-free sync driver for the zerodb wasm browser peer.
//
// Speaks sync protocol v2 as the *client* over a WebSocket (works in browsers
// and Node >= 22 with the global WebSocket):
//   client -> Hello   { v: 2, datastore_id, peer, op_ids }
//   server -> HelloOk { v, datastore_id, peer, need }
//   server -> OpsMsg  { ops }              (ops we lack)
//   client -> OpsMsg  { ops }              (ops from `need` that we have)
//   server -> OpsAck  { accepted, skipped }
//
// Frames are u32 big-endian length + JSON bytes inside WS *binary* messages.
// WS messages may fragment or coalesce frames, so bytes are buffered and
// frames extracted independently of message boundaries.

export const SYNC_PROTOCOL_VERSION = 2

const encoder = new TextEncoder()
const decoder = new TextDecoder()

/** Encode one protocol frame: u32 BE length + JSON bytes. */
export function encodeFrame(msg) {
  const body = encoder.encode(JSON.stringify(msg))
  const frame = new Uint8Array(4 + body.length)
  new DataView(frame.buffer).setUint32(0, body.length, false)
  frame.set(body, 4)
  return frame
}

/** Buffers incoming bytes and yields complete JSON frames. */
export class FrameReader {
  #chunks = []
  #length = 0

  push(bytes) {
    const arr = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes)
    this.#chunks.push(arr)
    this.#length += arr.length
    const out = []
    for (;;) {
      const frame = this.#next()
      if (frame === null) return out
      out.push(frame)
    }
  }

  #next() {
    if (this.#length < 4) return null
    const buf = this.#compact()
    const len = new DataView(buf.buffer, buf.byteOffset).getUint32(0, false)
    if (this.#length < 4 + len) return null
    const body = buf.subarray(4, 4 + len)
    this.#chunks = [buf.subarray(4 + len)]
    this.#length -= 4 + len
    return JSON.parse(decoder.decode(body))
  }

  #compact() {
    if (this.#chunks.length === 1) return this.#chunks[0]
    const all = new Uint8Array(this.#length)
    let at = 0
    for (const c of this.#chunks) {
      all.set(c, at)
      at += c.length
    }
    this.#chunks = [all]
    return all
  }
}

function openSocket(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url)
    ws.binaryType = 'arraybuffer'
    ws.onopen = () => resolve(ws)
    ws.onerror = () => reject(new Error(`websocket error connecting to ${url}`))
  })
}

/**
 * One two-way sync session between a wasm `ZeroDb` and a peer's WS listener.
 * Returns `{ accepted, skipped, sent, remoteAccepted, remoteSkipped }`.
 */
export async function syncOnce(db, url, { timeoutMs = 30_000 } = {}) {
  const ws = await openSocket(url)
  try {
    const reader = new FrameReader()
    const queue = []
    let wake = null
    let closed = false
    ws.onmessage = e => {
      for (const frame of reader.push(e.data)) queue.push(frame)
      if (wake) wake()
    }
    ws.onclose = () => {
      closed = true
      if (wake) wake()
    }
    const nextFrame = async () => {
      const deadline = Date.now() + timeoutMs
      while (queue.length === 0) {
        if (closed) throw new Error('connection closed mid-session')
        if (Date.now() > deadline) throw new Error('sync timeout')
        await new Promise(r => {
          wake = r
          setTimeout(r, 250)
        })
        wake = null
      }
      return queue.shift()
    }

    ws.send(encodeFrame({
      v: SYNC_PROTOCOL_VERSION,
      datastore_id: db.datastoreId(),
      peer: db.peerId(),
      op_ids: db.opIds(),
    }))

    const helloOk = await nextFrame()
    if (helloOk.v !== SYNC_PROTOCOL_VERSION) {
      throw new Error(`bad hello version ${helloOk.v}`)
    }
    const opsMsg = await nextFrame()

    let accepted = 0
    let skipped = 0
    if (opsMsg.ops.length > 0) {
      const result = db.importJson(JSON.stringify({
        format: 1,
        datastore_id: helloOk.datastore_id,
        ops: opsMsg.ops,
      }))
      accepted = result.accepted
      skipped = result.skipped
    }

    const ops = JSON.parse(db.exportOpsByIds(helloOk.need))
    ws.send(encodeFrame({ ops }))
    const ack = await nextFrame()

    if (accepted > 0) db.replay()
    return {
      accepted,
      skipped,
      sent: ops.length,
      remoteAccepted: ack.accepted,
      remoteSkipped: ack.skipped,
    }
  } finally {
    try { ws.close() } catch { /* already closed */ }
  }
}

/**
 * Poll-based auto-sync: run `syncOnce` every `intervalMs` (no dirty-flag —
 * the wasm store has no change events yet). Returns a stop function.
 * `onSync(summary)` / `onError(err)` are optional observers.
 */
export function autoSync(db, url, intervalMs = 2000, { onSync, onError } = {}) {
  let stopped = false
  let timer = null
  const loop = async () => {
    if (stopped) return
    try {
      const summary = await syncOnce(db, url)
      if (onSync) onSync(summary)
    } catch (err) {
      if (onError) onError(err)
    }
    if (!stopped) timer = setTimeout(loop, intervalMs)
  }
  loop()
  return () => {
    stopped = true
    if (timer) clearTimeout(timer)
  }
}

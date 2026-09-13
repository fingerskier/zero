// Counting TCP proxy between bots and the relay. Bytes and connections per
// direction are captured here so the numbers do not depend on which client
// (NAPI, TS peer, browser) is under test.
import net from 'node:net'

export function startProxy(targetPort, host = '127.0.0.1') {
  const counters = { connections: 0, active: 0, bytesToRelay: 0, bytesFromRelay: 0 }
  const server = net.createServer((client) => {
    counters.connections += 1
    counters.active += 1
    const upstream = net.connect(targetPort, host)
    client.on('data', (b) => {
      counters.bytesToRelay += b.length
      upstream.write(b)
    })
    upstream.on('data', (b) => {
      counters.bytesFromRelay += b.length
      client.write(b)
    })
    const done = () => {
      if (counters.active > 0 && !client.destroyed) counters.active -= 1
      client.destroy()
      upstream.destroy()
    }
    client.on('close', done)
    client.on('error', done)
    upstream.on('close', done)
    upstream.on('error', done)
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, host, () => {
      const { port } = server.address()
      resolve({
        port,
        url: `ws://${host}:${port}`,
        counters,
        snapshot: () => ({ ...counters }),
        /** Delta since `mark`; bytes/connections are monotonic. */
        since: (mark) => ({
          connections: counters.connections - mark.connections,
          bytesToRelay: counters.bytesToRelay - mark.bytesToRelay,
          bytesFromRelay: counters.bytesFromRelay - mark.bytesFromRelay,
        }),
        close: () => new Promise((r) => server.close(() => r())),
      })
    })
  })
}

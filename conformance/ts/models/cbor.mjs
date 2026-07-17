// Independent deterministic-CBOR codec per doc/KERNEL.md §3 — written
// against the contract prose, not ported from zerodb-core/src/cbor.rs.
// Values: {t:'uint'|'bytes'|'text'|'array'|'map'|'bool'|'null', ...} tagged
// form (the same shape vectors use).

const MAX_DEPTH = 16;

function hexToBytes(hex) {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) out[i] = parseInt(hex.substr(i * 2, 2), 16);
  return out;
}

export function bytesToHex(bytes) {
  return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function header(major, arg) {
  const mt = major << 5;
  if (arg < 24) return [mt | Number(arg)];
  if (arg < 0x100) return [mt | 24, Number(arg)];
  if (arg < 0x10000) return [mt | 25, Number(arg) >> 8, Number(arg) & 0xff];
  if (arg < 0x100000000) {
    const n = Number(arg);
    return [mt | 26, (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff];
  }
  const big = BigInt(arg);
  const bytes = [mt | 27];
  for (let shift = 56n; shift >= 0n; shift -= 8n) bytes.push(Number((big >> shift) & 0xffn));
  return bytes;
}

function encodeTextKey(key) {
  const utf8 = new TextEncoder().encode(key);
  return [...header(3, utf8.length), ...utf8];
}

function encodeInto(value, out, depth) {
  if (depth > MAX_DEPTH) throw new Error('Depth');
  switch (value.t) {
    case 'uint':
      out.push(...header(0, value.v));
      break;
    case 'bytes': {
      const bytes = hexToBytes(value.hex);
      out.push(...header(2, bytes.length), ...bytes);
      break;
    }
    case 'text': {
      const utf8 = new TextEncoder().encode(value.v);
      out.push(...header(3, utf8.length), ...utf8);
      break;
    }
    case 'array':
      out.push(...header(4, value.v.length));
      for (const item of value.v) encodeInto(item, out, depth + 1);
      break;
    case 'map': {
      const entries = Object.entries(value.v).map(([k, v]) => [encodeTextKey(k), v]);
      entries.sort((a, b) => compareBytes(a[0], b[0]));
      for (let i = 1; i < entries.length; i += 1) {
        if (compareBytes(entries[i - 1][0], entries[i][0]) === 0) throw new Error('DuplicateKey');
      }
      out.push(...header(5, entries.length));
      for (const [keyBytes, v] of entries) {
        out.push(...keyBytes);
        encodeInto(v, out, depth + 1);
      }
      break;
    }
    case 'bool':
      out.push(value.v ? 0xf5 : 0xf4);
      break;
    case 'null':
      out.push(0xf6);
      break;
    default:
      throw new Error(`unknown tagged type ${value.t}`);
  }
}

function compareBytes(a, b) {
  const len = Math.min(a.length, b.length);
  for (let i = 0; i < len; i += 1) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

export function encode(value) {
  const out = [];
  encodeInto(value, out, 0);
  return new Uint8Array(out);
}

// ------------------------------------------------------------- decoder

class Cursor {
  constructor(bytes) {
    this.bytes = bytes;
    this.pos = 0;
  }

  byte() {
    if (this.pos >= this.bytes.length) throw new Error('Truncated');
    return this.bytes[this.pos++];
  }

  take(n) {
    if (this.pos + n > this.bytes.length) throw new Error('Truncated');
    const slice = this.bytes.subarray(this.pos, this.pos + n);
    this.pos += n;
    return slice;
  }

  arg(additional) {
    if (additional < 24) return additional;
    if (additional === 24) {
      const v = this.byte();
      if (v < 24) throw new Error('NonCanonical');
      return v;
    }
    if (additional === 25) {
      const b = this.take(2);
      const v = (b[0] << 8) | b[1];
      if (v < 0x100) throw new Error('NonCanonical');
      return v;
    }
    if (additional === 26) {
      const b = this.take(4);
      const v = b[0] * 0x1000000 + (b[1] << 16) + (b[2] << 8) + b[3];
      if (v < 0x10000) throw new Error('NonCanonical');
      return v;
    }
    if (additional === 27) {
      const b = this.take(8);
      let v = 0n;
      for (const byte of b) v = (v << 8n) | BigInt(byte);
      if (v < 0x100000000n) throw new Error('NonCanonical');
      return v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v;
    }
    throw new Error('NonCanonical'); // 28-30 reserved, 31 indefinite
  }

  item(depth) {
    if (depth > MAX_DEPTH) throw new Error('Depth');
    const initial = this.byte();
    const major = initial >> 5;
    const additional = initial & 0x1f;
    switch (major) {
      case 0:
        return { t: 'uint', v: this.arg(additional) };
      case 2: {
        const len = Number(this.arg(additional));
        return { t: 'bytes', hex: bytesToHex(this.take(len)) };
      }
      case 3: {
        const len = Number(this.arg(additional));
        return { t: 'text', v: new TextDecoder('utf-8', { fatal: true }).decode(this.take(len)) };
      }
      case 4: {
        const len = Number(this.arg(additional));
        const items = [];
        for (let i = 0; i < len; i += 1) items.push(this.item(depth + 1));
        return { t: 'array', v: items };
      }
      case 5: {
        const len = Number(this.arg(additional));
        const obj = {};
        let prevKey = null;
        for (let i = 0; i < len; i += 1) {
          const keyStart = this.pos;
          const key = this.item(depth + 1);
          if (key.t !== 'text') throw new Error('Unsupported');
          const keyBytes = this.bytes.subarray(keyStart, this.pos);
          if (prevKey !== null) {
            const cmp = compareBytes(prevKey, keyBytes);
            if (cmp === 0) throw new Error('DuplicateKey');
            if (cmp > 0) throw new Error('KeyOrder');
          }
          prevKey = keyBytes;
          obj[key.v] = this.item(depth + 1);
        }
        return { t: 'map', v: obj };
      }
      case 7:
        if (initial === 0xf4) return { t: 'bool', v: false };
        if (initial === 0xf5) return { t: 'bool', v: true };
        if (initial === 0xf6) return { t: 'null' };
        throw new Error('Unsupported');
      default:
        throw new Error('Unsupported'); // major 1 (nint) unused; 6 = tags
    }
  }
}

export function decode(bytes) {
  const cursor = new Cursor(bytes);
  let value;
  try {
    value = cursor.item(0);
  } catch (e) {
    if (e instanceof RangeError || e.name === 'TypeError') throw new Error('Utf8');
    throw e;
  }
  if (cursor.pos !== bytes.length) throw new Error('Trailing');
  return value;
}

export { hexToBytes };

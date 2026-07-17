// op-encoding / op-decode-negative handlers per doc/KERNEL.md §4/§9.
//
// op-encoding: rebuild the §4.1 envelope map from the vector's logical
// description, encode with the independent CBOR codec, and require byte
// agreement with canonical_hex (the I-6 cross-implementation check), plus
// decode→re-encode identity. OpId (BLAKE3) is verified by the Rust harness;
// a JS BLAKE3 lands with the crypto layer, at which point this handler
// gains the op_id_hex check too.

import { encode, decode, bytesToHex, hexToBytes } from './cbor.mjs';

function envelopeToTagged(op) {
  return {
    t: 'map',
    v: {
      v: { t: 'uint', v: op.v },
      ds: { t: 'bytes', hex: op.ds },
      ep: { t: 'uint', v: op.ep },
      author: { t: 'bytes', hex: op.author },
      ts: {
        t: 'map',
        v: {
          l: { t: 'uint', v: op.ts.l },
          p: { t: 'uint', v: op.ts.p },
        },
      },
      deps: { t: 'array', v: op.deps.map((d) => ({ t: 'bytes', hex: d })) },
      grp: op.grp === null ? { t: 'null' } : { t: 'bytes', hex: op.grp },
      kind: { t: 'uint', v: op.kind },
      body: op.body,
    },
  };
}

export function runOpEncodingVector(vector) {
  const bytes = encode(envelopeToTagged(vector.op));
  const hex = bytesToHex(bytes);
  if (hex !== vector.canonical_hex) {
    throw new Error(
      `canonical bytes mismatch:\n  expected ${vector.canonical_hex}\n  got      ${hex}`
    );
  }
  const reencoded = bytesToHex(encode(decode(bytes)));
  if (reencoded !== hex) {
    throw new Error('decode -> re-encode is not byte-identical');
  }
}

export function runOpDecodeNegativeVector(vector) {
  let error = null;
  try {
    decode(hexToBytes(vector.raw_hex));
  } catch (e) {
    error = e.message;
  }
  if (error === null) {
    throw new Error(`expected ${vector.expect_error}, but decode succeeded`);
  }
  if (error !== vector.expect_error) {
    throw new Error(`expected ${vector.expect_error}, got ${error}`);
  }
}

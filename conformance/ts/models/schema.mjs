// schema-ir handler per doc/SCHEMA.md §2/§6: encode the tagged IR
// description with the independent CBOR codec, require byte agreement
// with the Rust-emitted canonical_hex plus round-trip identity, and
// derive SchemaId = BLAKE3(domain ‖ bytes) with the independent BLAKE3.

import { encode, decode, bytesToHex } from './cbor.mjs';
import { blake3 } from './blake3.mjs';

const DOMAIN_SCHEMA_IR = new TextEncoder().encode('zerodb-schema-ir-v1');

export function runSchemaIrVector(vector) {
  const bytes = encode(vector.ir);
  const hex = bytesToHex(bytes);
  if (hex !== vector.canonical_hex) {
    throw new Error(`canonical IR bytes mismatch:\n  expected ${vector.canonical_hex}\n  got      ${hex}`);
  }
  const reencoded = bytesToHex(encode(decode(bytes)));
  if (reencoded !== hex) {
    throw new Error('decode -> re-encode is not byte-identical');
  }

  const preimage = new Uint8Array(DOMAIN_SCHEMA_IR.length + bytes.length);
  preimage.set(DOMAIN_SCHEMA_IR, 0);
  preimage.set(bytes, DOMAIN_SCHEMA_IR.length);
  const idHex = bytesToHex(blake3(preimage));
  if (idHex !== vector.schema_id_hex) {
    throw new Error(`schema id mismatch: expected ${vector.schema_id_hex}, got ${idHex}`);
  }
}

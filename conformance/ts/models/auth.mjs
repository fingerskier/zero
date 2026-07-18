// M0d auth models: device-cert, genesis-id, authz-predicate (doc/AUTH.md).
// Independent of the Rust core.

import { createPublicKey, verify as edVerify } from 'node:crypto';
import { Buffer } from 'node:buffer';

import { encode, bytesToHex, hexToBytes } from './cbor.mjs';
import { blake3 } from './blake3.mjs';

const DOMAIN_DEVICE_CERT = new TextEncoder().encode('zerodb-device-cert-v1');
const DOMAIN_GENESIS = new TextEncoder().encode('zerodb-genesis-v1');
const DOMAIN_OP_ID = new TextEncoder().encode('zerodb-op-id-v1');
const SPKI_PREFIX = hexToBytes('302a300506032b6570032100');

const KR_DEVICE_CERT = 0;
const KR_DEVICE_REVOKE = 1;

const SCOPE_WRITE = 0;
const SCOPE_ADMIN = 1;
const SCOPE_READ = 2;
const SCOPE_SYNC = 3;

const KIND_GENESIS = 0;
const KIND_CREATE_NODE = 1;
const KIND_CREATE_EDGE = 2;
const KIND_SET_PROPERTY = 3;
const KIND_TOMBSTONE = 4;
const KIND_SCHEMA_EPOCH = 5;
const KIND_CAP_GRANT = 6;
const KIND_CAP_REVOKE = 7;
const KIND_KEY_RECORD = 8;
const KIND_CHECKPOINT = 9;

function principalId(rootPk) {
  return blake3(rootPk);
}

function peerId(devicePk) {
  return blake3(devicePk);
}

function blake3Domain(domain, body) {
  const pre = new Uint8Array(domain.length + body.length);
  pre.set(domain, 0);
  pre.set(body, domain.length);
  return blake3(pre);
}

// ---------------------------------------------------------------- device-cert

function certMapTagged(cert) {
  return {
    t: 'map',
    v: {
      kr: { t: 'uint', v: cert.kr },
      device: { t: 'bytes', hex: cert.device },
      principal: { t: 'bytes', hex: cert.principal },
      root_pk: { t: 'bytes', hex: cert.root_pk },
      issued: { t: 'uint', v: cert.issued },
      expiry:
        cert.expiry === null || cert.expiry === undefined
          ? { t: 'null' }
          : { t: 'uint', v: cert.expiry },
      revoke_of:
        cert.revoke_of === null || cert.revoke_of === undefined
          ? { t: 'null' }
          : { t: 'bytes', hex: cert.revoke_of },
    },
  };
}

function certPreimageFixed(cert) {
  const body = encode(certMapTagged(cert));
  const pre = new Uint8Array(DOMAIN_DEVICE_CERT.length + body.length);
  pre.set(DOMAIN_DEVICE_CERT, 0);
  pre.set(body, DOMAIN_DEVICE_CERT.length);
  return { body, pre };
}

function verifySig(rootPk, message, sig) {
  const key = createPublicKey({
    key: Buffer.concat([Buffer.from(SPKI_PREFIX), Buffer.from(rootPk)]),
    format: 'der',
    type: 'spki',
  });
  return edVerify(null, Buffer.from(message), key, Buffer.from(sig));
}

export function verifyDeviceCert(cert) {
  if (cert.kr !== KR_DEVICE_CERT && cert.kr !== KR_DEVICE_REVOKE) {
    return 'CAP_INVALID';
  }
  const rootPk = hexToBytes(cert.root_pk);
  if (bytesToHex(principalId(rootPk)) !== cert.principal) {
    return 'CAP_INVALID';
  }
  if (cert.kr === KR_DEVICE_CERT && cert.revoke_of != null) {
    return 'CAP_INVALID';
  }
  if (cert.kr === KR_DEVICE_REVOKE && cert.revoke_of == null) {
    return 'CAP_INVALID';
  }
  const { pre } = certPreimageFixed(cert);
  if (!verifySig(rootPk, pre, hexToBytes(cert.cert_sig))) {
    return 'AUTH_SIG_INVALID';
  }
  return 'ok';
}

export function runDeviceCertVector(vector) {
  const cert = vector.cert;
  const { body } = certPreimageFixed(cert);
  if (vector.preimage_body_hex && bytesToHex(body) !== vector.preimage_body_hex) {
    throw new Error(
      `preimage body mismatch:\n  expected ${vector.preimage_body_hex}\n  got      ${bytesToHex(body)}`
    );
  }
  if (vector.peer_id_hex) {
    const pid = bytesToHex(peerId(hexToBytes(cert.device)));
    if (pid !== vector.peer_id_hex) {
      throw new Error(`peer id mismatch:\n  expected ${vector.peer_id_hex}\n  got      ${pid}`);
    }
  }
  if (vector.principal_id_hex) {
    const prid = bytesToHex(principalId(hexToBytes(cert.root_pk)));
    if (prid !== vector.principal_id_hex) {
      throw new Error(
        `principal id mismatch:\n  expected ${vector.principal_id_hex}\n  got      ${prid}`
      );
    }
  }
  const outcome = verifyDeviceCert(cert);
  if (outcome !== vector.expect) {
    throw new Error(`expect ${vector.expect}, got ${outcome}`);
  }
}

// ---------------------------------------------------------------- genesis-id

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

function isZeroDs(hex) {
  return /^0+$/.test(hex);
}

export function runGenesisIdVector(vector) {
  const op = vector.op;
  if (op.kind !== KIND_GENESIS) {
    throw new Error('genesis-id vector kind must be 0');
  }
  if (!isZeroDs(op.ds)) {
    if (vector.expect !== 'AUTH_WRONG_DATASTORE') {
      throw new Error(`expect ${vector.expect}, got AUTH_WRONG_DATASTORE`);
    }
    return;
  }
  const body = encode(envelopeToTagged(op));
  if (vector.preimage_body_hex && bytesToHex(body) !== vector.preimage_body_hex) {
    throw new Error(
      `preimage body mismatch:\n  expected ${vector.preimage_body_hex}\n  got      ${bytesToHex(body)}`
    );
  }
  const ds = blake3Domain(DOMAIN_GENESIS, body);
  const dsHex = bytesToHex(ds);
  if (vector.expect !== 'ok') {
    throw new Error(`expect ${vector.expect}, got ok`);
  }
  if (vector.datastore_id_hex && dsHex !== vector.datastore_id_hex) {
    throw new Error(
      `datastore id mismatch:\n  expected ${vector.datastore_id_hex}\n  got      ${dsHex}`
    );
  }
  if (vector.op_id_hex) {
    const oid = bytesToHex(blake3Domain(DOMAIN_OP_ID, body));
    if (oid !== vector.op_id_hex) {
      throw new Error(`op id mismatch:\n  expected ${vector.op_id_hex}\n  got      ${oid}`);
    }
  }
  if (vector.different_from_hex && dsHex === vector.different_from_hex) {
    throw new Error('salt-sensitive DatastoreId collided with baseline');
  }
}

// ---------------------------------------------------------------- authz-predicate

function isKnownScope(s) {
  return s === SCOPE_WRITE || s === SCOPE_ADMIN || s === SCOPE_READ || s === SCOPE_SYNC;
}

function requiredScope(kind) {
  if (
    kind === KIND_CREATE_NODE ||
    kind === KIND_CREATE_EDGE ||
    kind === KIND_SET_PROPERTY ||
    kind === KIND_TOMBSTONE ||
    kind === KIND_KEY_RECORD
  ) {
    return SCOPE_WRITE;
  }
  if (
    kind === KIND_SCHEMA_EPOCH ||
    kind === KIND_CAP_GRANT ||
    kind === KIND_CAP_REVOKE ||
    kind === KIND_CHECKPOINT
  ) {
    return SCOPE_ADMIN;
  }
  throw new Error(`CAP_INVALID kind ${kind}`);
}

function causalPast(op, byId) {
  const seen = new Set();
  const q = [...op.deps];
  while (q.length) {
    const id = q.shift();
    if (seen.has(id)) continue;
    seen.add(id);
    const dep = byId.get(id);
    if (dep) {
      for (const d of dep.deps) q.push(d);
    }
  }
  return seen;
}

export function authorize(datastoreIdHex, applied, candidate) {
  const byId = new Map(applied.map((o) => [o.id, o]));

  if (candidate.kind === KIND_GENESIS) {
    if (!isZeroDs(candidate.ds)) return 'AUTH_WRONG_DATASTORE';
    if (candidate.body.type !== 'genesis') return 'CAP_INVALID';
    if (candidate.principal !== candidate.body.founder) return 'AUTH_NO_MEMBERSHIP';
    return 'ok';
  }

  if (candidate.ds !== datastoreIdHex) return 'AUTH_WRONG_DATASTORE';
  for (const o of applied) {
    if (o.kind !== KIND_GENESIS && o.ds !== datastoreIdHex) {
      return 'AUTH_WRONG_DATASTORE';
    }
  }

  const genesis = applied.find((o) => o.kind === KIND_GENESIS);
  if (!genesis || genesis.body.type !== 'genesis') return 'CAP_INVALID';
  const founder = genesis.body.founder;

  const past = causalPast(candidate, byId);
  if (!past.has(genesis.id)) return 'AUTH_NO_MEMBERSHIP';

  let need;
  try {
    need = requiredScope(candidate.kind);
  } catch {
    return 'CAP_INVALID';
  }

  const grants = [];
  if (candidate.principal === founder) {
    grants.push({
      id: genesis.id,
      scopes: [SCOPE_WRITE, SCOPE_ADMIN, SCOPE_READ, SCOPE_SYNC],
      expiry: null,
    });
  }

  for (const id of past) {
    const op = byId.get(id);
    if (!op || op.kind !== KIND_CAP_GRANT || op.body.type !== 'grant') continue;
    if (op.body.subject !== candidate.principal) continue;
    if (op.body.ds_bind !== datastoreIdHex) continue;
    for (const s of op.body.scopes) {
      if (!isKnownScope(s)) return 'CAP_SCOPE_UNKNOWN';
    }
    grants.push({
      id: op.id,
      scopes: op.body.scopes,
      expiry: op.body.expiry,
    });
  }

  const revoked = new Set();
  for (const id of past) {
    const op = byId.get(id);
    if (!op || op.kind !== KIND_CAP_REVOKE || op.body.type !== 'revoke') continue;
    revoked.add(op.body.grant);
  }

  let sawExpired = false;
  let sawScopeButNotAdmin = false;

  for (const g of grants) {
    if (revoked.has(g.id)) continue;
    if (g.expiry != null && candidate.ts_p >= g.expiry) {
      sawExpired = true;
      continue;
    }
    if (!g.scopes.includes(need)) {
      if (need === SCOPE_ADMIN && g.scopes.includes(SCOPE_WRITE)) {
        sawScopeButNotAdmin = true;
      }
      continue;
    }
    return 'ok';
  }

  if (
    grants.some((g) => g.scopes.includes(need) && revoked.has(g.id)) &&
    !grants.some((g) => g.scopes.includes(need) && !revoked.has(g.id))
  ) {
    return 'AUTH_REVOKED';
  }
  if (sawExpired) return 'AUTH_EXPIRED';
  if (need === SCOPE_ADMIN || sawScopeButNotAdmin) return 'AUTH_NOT_ADMIN';
  return 'AUTH_NO_MEMBERSHIP';
}

export function runAuthzPredicateVector(vector) {
  const outcome = authorize(vector.datastore_id_hex, vector.applied, vector.candidate);
  if (outcome !== vector.expect) {
    throw new Error(`expect ${vector.expect}, got ${outcome}`);
  }
}

// ---------------------------------------------------------------- admission-token

const DOMAIN_RELAY_AUTH = new TextEncoder().encode('zerodb-relay-auth-v1');

function certMapForToken(cert) {
  return {
    t: 'map',
    v: {
      cert_sig: { t: 'bytes', hex: cert.cert_sig },
      device: { t: 'bytes', hex: cert.device },
      expiry:
        cert.expiry === null || cert.expiry === undefined
          ? { t: 'null' }
          : { t: 'uint', v: cert.expiry },
      issued: { t: 'uint', v: cert.issued },
      kr: { t: 'uint', v: cert.kr },
      principal: { t: 'bytes', hex: cert.principal },
      root_pk: { t: 'bytes', hex: cert.root_pk },
      revoke_of:
        cert.revoke_of === null || cert.revoke_of === undefined
          ? { t: 'null' }
          : { t: 'bytes', hex: cert.revoke_of },
    },
  };
}

function tokenPreimageBody(token) {
  return encode({
    t: 'map',
    v: {
      cert: certMapForToken(token.cert),
      device: { t: 'bytes', hex: token.device },
      ds: { t: 'bytes', hex: token.ds },
      grant: { t: 'bytes', hex: token.grant },
      scopes: { t: 'array', v: token.scopes.map((s) => ({ t: 'uint', v: s })) },
      subject: { t: 'bytes', hex: token.subject },
    },
  });
}

export function verifyAdmissionToken(token, knownGrants) {
  const certOutcome = verifyDeviceCert(token.cert);
  if (certOutcome !== 'ok') return certOutcome;
  if (token.cert.kr !== KR_DEVICE_CERT) return 'CAP_INVALID';
  if (token.device !== token.cert.device) return 'CAP_INVALID';
  if (token.subject !== token.cert.principal) return 'AUTH_NO_MEMBERSHIP';

  const grant = knownGrants.find((g) => g.id === token.grant);
  if (!grant) return 'AUTH_NO_MEMBERSHIP';
  if (grant.ds !== token.ds || grant.subject !== token.subject) return 'AUTH_NO_MEMBERSHIP';
  if (grant.revoked) return 'AUTH_REVOKED';
  for (const s of token.scopes) {
    if (!isKnownScope(s)) return 'CAP_SCOPE_UNKNOWN';
    if (!grant.scopes.includes(s)) return 'AUTH_NO_MEMBERSHIP';
  }

  const body = tokenPreimageBody(token);
  const pre = new Uint8Array(DOMAIN_RELAY_AUTH.length + body.length);
  pre.set(DOMAIN_RELAY_AUTH, 0);
  pre.set(body, DOMAIN_RELAY_AUTH.length);
  if (!verifySig(hexToBytes(token.device), pre, hexToBytes(token.sig))) {
    return 'AUTH_SIG_INVALID';
  }
  return 'ok';
}

export function runAdmissionTokenVector(vector) {
  if (vector.preimage_body_hex) {
    const body = tokenPreimageBody(vector.token);
    if (bytesToHex(body) !== vector.preimage_body_hex) {
      throw new Error(
        `token preimage body mismatch:\n  expected ${vector.preimage_body_hex}\n  got      ${bytesToHex(body)}`
      );
    }
  }
  const outcome = verifyAdmissionToken(vector.token, vector.known_grants);
  if (outcome !== vector.expect) {
    throw new Error(`expect ${vector.expect}, got ${outcome}`);
  }
}


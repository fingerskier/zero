// IR structural validator per doc/SCHEMA.md §2, over the tagged form.
// Independent of zerodb-core/src/schema.rs; outcome names are the contract.

const CRDT_LWW = 0;
const CRDT_GCOUNTER = 1;
const CRDT_PNCOUNTER = 2;
const CRDT_FLAG = 4;
const TYPE_BOOL = 1;
const TYPE_INT = 2;
const TYPE_BYTES = 5;

class IrError extends Error {}
const fail = (name) => {
  throw new IrError(name);
};

function mapEntries(v) {
  if (!v || v.t !== 'map') fail('IR_INVALID');
  return v.v;
}

function checkKeys(obj, allowed, required) {
  for (const k of Object.keys(obj)) {
    if (!allowed.includes(k)) fail('IR_UNKNOWN_KEY');
  }
  for (const r of required) {
    if (!(r in obj)) fail('IR_INVALID');
  }
}

const uint = (v) => (v && v.t === 'uint' ? v.v : fail('IR_INVALID'));
const bool = (v) => (v && v.t === 'bool' ? v.v : fail('IR_INVALID'));

function validateProp(v) {
  const p = mapEntries(v);
  checkKeys(p, ['crdt', 'type', 'nullable', 'encrypted'], ['crdt', 'type', 'nullable', 'encrypted']);
  const crdt = uint(p.crdt);
  const valueType = uint(p.type);
  bool(p.nullable);
  const encrypted = bool(p.encrypted);

  if (crdt > CRDT_FLAG) fail('IR_CRDT_UNSUPPORTED'); // 5-7 reserved (M2), ≥8 unregistered
  if (valueType > TYPE_BYTES) fail('IR_INVALID');
  if (encrypted && crdt !== CRDT_LWW) fail('IR_ENCRYPTED_INVALID');
  if ((crdt === CRDT_GCOUNTER || crdt === CRDT_PNCOUNTER) && valueType !== TYPE_INT) fail('IR_TYPE_MISMATCH');
  if (crdt === CRDT_FLAG && valueType !== TYPE_BOOL) fail('IR_TYPE_MISMATCH');
}

function validateProps(v) {
  for (const def of Object.values(mapEntries(v))) validateProp(def);
}

function validateLabels(v) {
  if (v && v.t === 'null') return;
  if (!v || v.t !== 'array') fail('IR_INVALID');
  for (const item of v.v) {
    if (item.t !== 'text') fail('IR_INVALID');
  }
}

/** Validate a tagged IR; returns null on success or the outcome name. */
export function validateIr(ir) {
  try {
    const top = mapEntries(ir);
    checkKeys(top, ['v', 'name', 'nodes', 'edges'], ['v', 'nodes', 'edges']);
    if (uint(top.v) !== 1) fail('IR_VERSION_UNSUPPORTED');
    if ('name' in top && top.name.t !== 'text') fail('IR_INVALID');

    for (const def of Object.values(mapEntries(top.nodes))) {
      const entity = mapEntries(def);
      checkKeys(entity, ['props'], ['props']);
      validateProps(entity.props);
    }
    for (const def of Object.values(mapEntries(top.edges))) {
      const edge = mapEntries(def);
      checkKeys(edge, ['props', 'src', 'dst'], ['props', 'src', 'dst']);
      validateProps(edge.props);
      validateLabels(edge.src);
      validateLabels(edge.dst);
    }
    return null;
  } catch (e) {
    if (e instanceof IrError) return e.message;
    throw e;
  }
}

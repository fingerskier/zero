// v0.1 query evaluator per doc/SCHEMA.md §5 — mirror of
// zerodb-core/src/queryeval.rs. Evaluates a parsed query (from query.mjs) over
// a fixture graph with the spec's deterministic read semantics: two-valued null
// logic, cross-type comparisons false (except int/float numeric), one ORDER BY
// total order, MVRegister conflict surfacing. Values are the tagged form the
// vectors use: {t:'null'|'bool'|'int'|'float'|'text'|'bytes'|'mv'|'node'|'edge', ...}.

import { parse } from './query.mjs';

const NULL = { t: 'null' };
const enc = new TextEncoder();

function asF64(v) {
  return v.t === 'int' || v.t === 'float' ? v.v : null;
}

function hexToBytes(h) {
  const out = new Uint8Array(h.length / 2);
  for (let i = 0; i < out.length; i += 1) out[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function bytesOf(v) {
  return v.t === 'text' ? enc.encode(v.v) : hexToBytes(v.hex);
}

function cmpBytes(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i += 1) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
}

// Collapse an MVRegister for a WHERE comparison: a singleton unwraps; any other
// size returns null (sentinel) making the comparison false.
function collapseForCmp(v) {
  if (v.t === 'mv') return v.v.length === 1 ? v.v[0] : null;
  return v;
}

function typeRank(v) {
  return { null: 0, bool: 1, int: 2, float: 2, text: 3, bytes: 4, mv: 5, node: 6, edge: 7 }[v.t];
}

function cmpOrder(a, b) {
  const fa = asF64(a);
  const fb = asF64(b);
  if (fa !== null && fb !== null) return fa < fb ? -1 : fa > fb ? 1 : 0;
  if (a.t === 'bool' && b.t === 'bool') return a.v === b.v ? 0 : a.v ? 1 : -1;
  if (a.t === 'text' && b.t === 'text') return cmpBytes(bytesOf(a), bytesOf(b));
  if (a.t === 'bytes' && b.t === 'bytes') return cmpBytes(bytesOf(a), bytesOf(b));
  return null; // cross-type
}

// WHERE comparison — two-valued (null / cross-type / non-singleton mv -> false).
function cmpPredicate(a, op, b) {
  const ca = collapseForCmp(a);
  const cb = collapseForCmp(b);
  if (ca === null || cb === null) return false;
  if (ca.t === 'null' || cb.t === 'null') return false;
  const ord = cmpOrder(ca, cb);
  if (ord === null) return false;
  switch (op) {
    case 'eq': return ord === 0;
    case 'ne': return ord !== 0;
    case 'lt': return ord < 0;
    case 'le': return ord <= 0;
    case 'gt': return ord > 0;
    case 'ge': return ord >= 0;
    default: throw new Error(`bad cmp ${op}`);
  }
}

// The single ORDER BY total order: null<bool<int/float<text<bytes, total.
function totalOrder(a, b) {
  const ra = typeRank(a);
  const rb = typeRank(b);
  if (ra !== rb) return ra < rb ? -1 : 1;
  switch (a.t) {
    case 'null': return 0;
    case 'bool': return a.v === b.v ? 0 : a.v ? 1 : -1;
    case 'int':
    case 'float': {
      const x = asF64(a);
      const y = asF64(b);
      return x < y ? -1 : x > y ? 1 : 0;
    }
    case 'text':
    case 'bytes': return cmpBytes(bytesOf(a), bytesOf(b));
    case 'mv': {
      const n = Math.min(a.v.length, b.v.length);
      for (let i = 0; i < n; i += 1) {
        const o = totalOrder(a.v[i], b.v[i]);
        if (o !== 0) return o;
      }
      return a.v.length === b.v.length ? 0 : a.v.length < b.v.length ? -1 : 1;
    }
    case 'node':
    case 'edge': return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
    default: return 0;
  }
}

function labelOk(want, have) {
  return want === null || want === undefined || want === have;
}

function bind(query, graph) {
  const pat = query.pattern;
  const nodes = [...graph.nodes].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  const out = [];
  if (!pat.hop) {
    for (const n of nodes) {
      if (labelOk(pat.left.label, n.label)) out.push({ [pat.left.var]: { kind: 'node', id: n.id } });
    }
    return out;
  }
  const { edge, right } = pat.hop;
  const nodeById = new Map(graph.nodes.map((n) => [n.id, n]));
  const edges = [...graph.edges].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  for (const e of edges) {
    if (!labelOk(edge.label, e.label)) continue;
    const [leftId, rightId] = edge.dir === 'outgoing' ? [e.src, e.dst] : [e.dst, e.src];
    const ln = nodeById.get(leftId);
    const rn = nodeById.get(rightId);
    if (!ln || !rn) continue;
    if (!labelOk(pat.left.label, ln.label) || !labelOk(right.label, rn.label)) continue;
    const b = { [pat.left.var]: { kind: 'node', id: ln.id }, [right.var]: { kind: 'node', id: rn.id } };
    if (edge.var) b[edge.var] = { kind: 'edge', id: e.id };
    out.push(b);
  }
  return out;
}

function litValue(v) {
  if (v === null) return NULL;
  if (typeof v === 'boolean') return { t: 'bool', v };
  if (typeof v === 'number') return Number.isInteger(v) ? { t: 'int', v } : { t: 'float', v };
  if (typeof v === 'string') return { t: 'text', v };
  throw new Error(`bad literal ${JSON.stringify(v)}`);
}

function resolveField(varName, path, b, graph) {
  const r = b[varName];
  if (!r) throw new Error(`unbound variable ${varName}`);
  if (!path) return r.kind === 'node' ? { t: 'node', id: r.id } : { t: 'edge', id: r.id };
  const holder =
    r.kind === 'node' ? graph.nodes.find((n) => n.id === r.id) : graph.edges.find((e) => e.id === r.id);
  const props = holder ? holder.props || {} : {};
  return path in props ? props[path] : NULL;
}

function resolveOperand(op, b, graph, params) {
  if (op.kind === 'field') return resolveField(op.var, op.path, b, graph);
  if (op.kind === 'lit') return litValue(op.value);
  if (op.kind === 'param') return params[op.name] ?? NULL;
  throw new Error(`bad operand ${JSON.stringify(op)}`);
}

function evalExpr(e, b, graph, params) {
  switch (e.op) {
    case 'or': return evalExpr(e.l, b, graph, params) || evalExpr(e.r, b, graph, params);
    case 'and': return evalExpr(e.l, b, graph, params) && evalExpr(e.r, b, graph, params);
    case 'not': return !evalExpr(e.e, b, graph, params);
    case 'cmp':
      return cmpPredicate(
        resolveOperand(e.l, b, graph, params),
        e.cmp,
        resolveOperand(e.r, b, graph, params)
      );
    case 'isnull': return resolveOperand(e.x, b, graph, params).t === 'null';
    case 'isnotnull': return resolveOperand(e.x, b, graph, params).t !== 'null';
    default: throw new Error(`bad expr ${JSON.stringify(e)}`);
  }
}

function bindingIds(b) {
  return Object.values(b).map((r) => r.id);
}

function cmpIds(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i += 1) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
}

export function evalQuery(query, graph, params) {
  let bindings = bind(query, graph);
  if (query.filter) bindings = bindings.filter((b) => evalExpr(query.filter, b, graph, params));

  const descending = query.orderBy ? query.orderBy.desc : false;
  const rows = bindings.map((b) => ({
    key: query.orderBy ? resolveField(query.orderBy.key.var, query.orderBy.key.path, b, graph) : NULL,
    ids: bindingIds(b),
    b,
  }));
  rows.sort((a, b) => {
    let o = totalOrder(a.key, b.key);
    if (descending) o = -o;
    return o !== 0 ? o : cmpIds(a.ids, b.ids);
  });

  const limited = query.limit === null || query.limit === undefined ? rows : rows.slice(0, query.limit);
  return limited.map((row) =>
    query.items.map((item) => resolveField(item.var, item.path, row.b, graph))
  );
}

export function runQueryEvalVector(vector) {
  const query = parse(vector.query);
  const graph = vector.graph;
  const params = vector.params || {};
  const got = evalQuery(query, graph, params);
  const want = vector.expect.rows;
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    throw new Error(`rows mismatch:\n  expected ${JSON.stringify(want)}\n  got      ${JSON.stringify(got)}`);
  }
}

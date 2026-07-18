// v0.1 query grammar parser per doc/SCHEMA.md §5 — mirror of
// zerodb-core/src/query.rs. Tokenizer + recursive-descent parser producing an
// AST; parse() throws on any grammar violation. Owns acceptance only;
// evaluation lands with the query-eval vectors. The two language parsers must
// accept/reject the same strings.

const RESERVED = new Set([
  'MATCH', 'WHERE', 'RETURN', 'ORDER', 'BY', 'ASC', 'DESC', 'LIMIT',
  'AND', 'OR', 'NOT', 'IS', 'NULL',
]);
const PUNCT2 = ['<>', '<=', '>='];
const PUNCT1 = ['(', ')', '[', ']', ':', '.', ',', '$', '=', '<', '>', '-'];

const kw = (s) => s.toUpperCase();
const isReserved = (s) => RESERVED.has(kw(s));

function tokenize(src) {
  const toks = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (/\s/.test(c)) { i += 1; continue; }
    if (c === '"' || c === "'") {
      const quote = c;
      let s = '';
      i += 1;
      let closed = false;
      while (i < src.length) {
        if (src[i] === '\\' && i + 1 < src.length) { s += src[i + 1]; i += 2; continue; }
        if (src[i] === quote) { closed = true; i += 1; break; }
        s += src[i];
        i += 1;
      }
      if (!closed) throw new Error('unterminated string literal');
      toks.push({ k: 'str', v: s });
      continue;
    }
    if (/[0-9]/.test(c)) {
      let j = i;
      while (j < src.length && /[0-9.]/.test(src[j])) j += 1;
      toks.push({ k: 'num', v: src.slice(i, j) });
      i = j;
      continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      let j = i;
      while (j < src.length && /[A-Za-z0-9_]/.test(src[j])) j += 1;
      toks.push({ k: 'ident', v: src.slice(i, j) });
      i = j;
      continue;
    }
    const rest = src.slice(i);
    const p2 = PUNCT2.find((p) => rest.startsWith(p));
    if (p2) { toks.push({ k: 'p', v: p2 }); i += 2; continue; }
    const p1 = PUNCT1.find((p) => rest.startsWith(p));
    if (p1) { toks.push({ k: 'p', v: p1 }); i += 1; continue; }
    throw new Error(`unexpected character '${c}'`);
  }
  return toks;
}

class P {
  constructor(toks) {
    this.toks = toks;
    this.pos = 0;
  }

  peek() { return this.toks[this.pos]; }

  isP(p) { const t = this.peek(); return t && t.k === 'p' && t.v === p; }

  isKw(k) { const t = this.peek(); return t && t.k === 'ident' && kw(t.v) === k; }

  eatP(p) {
    if (!this.isP(p)) throw new Error(`expected '${p}'`);
    this.pos += 1;
  }

  eatKw(k) {
    if (!this.isKw(k)) throw new Error(`expected keyword ${k}`);
    this.pos += 1;
  }

  ident() {
    const t = this.peek();
    if (!t || t.k !== 'ident' || isReserved(t.v)) throw new Error('expected identifier');
    this.pos += 1;
    return t.v;
  }

  query() {
    this.eatKw('MATCH');
    const pattern = this.pattern();
    let filter = null;
    if (this.isKw('WHERE')) { this.pos += 1; filter = this.expr(); }
    this.eatKw('RETURN');
    const items = this.items();
    let orderBy = null;
    if (this.isKw('ORDER')) {
      this.pos += 1;
      this.eatKw('BY');
      const key = this.item();
      let desc = false;
      if (this.isKw('ASC')) this.pos += 1;
      else if (this.isKw('DESC')) { this.pos += 1; desc = true; }
      orderBy = { key, desc };
    }
    let limit = null;
    if (this.isKw('LIMIT')) { this.pos += 1; limit = this.uint(); }
    if (this.pos !== this.toks.length) throw new Error('unexpected trailing tokens');
    return { pattern, filter, items, orderBy, limit };
  }

  pattern() {
    const left = this.entity();
    let hop = null;
    if (this.isP('-') || this.isP('<')) {
      const edge = this.edge();
      const right = this.entity();
      hop = { edge, right };
    }
    return { left, hop };
  }

  entity() {
    this.eatP('(');
    const varName = this.ident();
    let label = null;
    if (this.isP(':')) { this.pos += 1; label = this.ident(); }
    this.eatP(')');
    return { var: varName, label };
  }

  edge() {
    let dir;
    if (this.isP('<')) { this.pos += 1; this.eatP('-'); dir = 'incoming'; }
    else { this.eatP('-'); dir = 'outgoing'; }
    this.eatP('[');
    let varName = null;
    const t = this.peek();
    if (t && t.k === 'ident' && !isReserved(t.v)) { varName = t.v; this.pos += 1; }
    let label = null;
    if (this.isP(':')) { this.pos += 1; label = this.ident(); }
    this.eatP(']');
    this.eatP('-');
    if (dir === 'outgoing') this.eatP('>');
    return { var: varName, label, dir };
  }

  items() {
    const out = [this.item()];
    while (this.isP(',')) { this.pos += 1; out.push(this.item()); }
    return out;
  }

  item() {
    const varName = this.ident();
    let path = null;
    if (this.isP('.')) { this.pos += 1; path = this.ident(); }
    return { var: varName, path };
  }

  expr() {
    let lhs = this.andExpr();
    while (this.isKw('OR')) { this.pos += 1; lhs = { op: 'or', l: lhs, r: this.andExpr() }; }
    return lhs;
  }

  andExpr() {
    let lhs = this.notExpr();
    while (this.isKw('AND')) { this.pos += 1; lhs = { op: 'and', l: lhs, r: this.notExpr() }; }
    return lhs;
  }

  notExpr() {
    if (this.isKw('NOT')) { this.pos += 1; return { op: 'not', e: this.notExpr() }; }
    return this.primary();
  }

  primary() {
    if (this.isP('(')) {
      this.pos += 1;
      const e = this.expr();
      this.eatP(')');
      return e;
    }
    const left = this.operand();
    if (this.isKw('IS')) {
      this.pos += 1;
      if (this.isKw('NOT')) { this.pos += 1; this.eatKw('NULL'); return { op: 'isnotnull', x: left }; }
      this.eatKw('NULL');
      return { op: 'isnull', x: left };
    }
    const cmp = this.cmp();
    const right = this.operand();
    return { op: 'cmp', cmp, l: left, r: right };
  }

  cmp() {
    const t = this.peek();
    const m = { '=': 'eq', '<>': 'ne', '<': 'lt', '<=': 'le', '>': 'gt', '>=': 'ge' };
    if (!t || t.k !== 'p' || !(t.v in m)) throw new Error('expected comparison operator');
    this.pos += 1;
    return m[t.v];
  }

  operand() {
    const t = this.peek();
    if (!t) throw new Error('expected operand');
    if (t.k === 'p' && t.v === '$') { this.pos += 1; return { kind: 'param', name: this.ident() }; }
    if (t.k === 'str') { this.pos += 1; return { kind: 'lit', value: t.v }; }
    if (t.k === 'num') return { kind: 'lit', value: this.number(false) };
    if (t.k === 'p' && t.v === '-') { this.pos += 1; return { kind: 'lit', value: this.number(true) }; }
    if (t.k === 'ident') {
      const up = kw(t.v);
      if (up === 'NULL') { this.pos += 1; return { kind: 'lit', value: null }; }
      if (up === 'TRUE') { this.pos += 1; return { kind: 'lit', value: true }; }
      if (up === 'FALSE') { this.pos += 1; return { kind: 'lit', value: false }; }
      if (isReserved(t.v)) throw new Error(`unexpected keyword ${t.v} in operand`);
      this.pos += 1;
      let path = null;
      if (this.isP('.')) { this.pos += 1; path = this.ident(); }
      return { kind: 'field', var: t.v, path };
    }
    throw new Error('expected operand');
  }

  number(negative) {
    const t = this.peek();
    if (!t || t.k !== 'num') throw new Error('expected number');
    this.pos += 1;
    const s = (negative ? '-' : '') + t.v;
    if (s.includes('.')) {
      const f = Number(s);
      if (!Number.isFinite(f)) throw new Error(`invalid float ${s}`);
      return f;
    }
    if (!/^-?\d+$/.test(s)) throw new Error(`invalid int ${s}`);
    const v = BigInt(s);
    if (v < -(2n ** 63n) || v > 2n ** 63n - 1n) throw new Error(`int out of i64 range ${s}`);
    return Number(v);
  }

  uint() {
    const t = this.peek();
    if (!t || t.k !== 'num' || t.v.includes('.')) throw new Error('LIMIT expects a non-negative integer');
    this.pos += 1;
    const v = BigInt(t.v);
    if (v < 0n || v > 2n ** 64n - 1n) throw new Error(`invalid LIMIT ${t.v}`);
    return Number(v);
  }
}

export function parse(src) {
  const toks = tokenize(src);
  if (toks.length === 0) throw new Error('empty query');
  return new P(toks).query();
}

export function runQueryParseVector(vector) {
  vector.cases.forEach((c, ci) => {
    let ok = true;
    let err = null;
    try {
      parse(c.query);
    } catch (e) {
      ok = false;
      err = e.message;
    }
    if (ok !== c.accept) {
      throw new Error(
        `case ${ci} (${JSON.stringify(c.query)}): expected accept=${c.accept}, got ${ok}${err ? ` (${err})` : ''}`
      );
    }
  });
}

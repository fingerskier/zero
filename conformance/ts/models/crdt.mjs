// Independent CRDT semantic-kernel model per doc/KERNEL.md §6.
// Implements the application-pipeline steps the crdt-apply vectors exercise:
// dedup by op_id (step 4) and per-type apply (step 7). Authorization,
// equivocation, and causal buffering enter as their vector types land.
//
// Total order per KERNEL §4.5: (physical_ms, logical, author, op_id) —
// authors/op_ids are fixed-width hex strings in vectors, so lexicographic
// string comparison is bytewise comparison.

function totalOrderKey(op) {
  return [op.ts.physical_ms, op.ts.logical, op.author, op.op_id];
}

function keyGreater(a, b) {
  for (let i = 0; i < 4; i += 1) {
    if (a[i] === b[i]) continue;
    return a[i] > b[i];
  }
  return false; // identical keys — identical op
}

const CRDTS = {
  lww: {
    init: () => ({ value: null, key: null }),
    apply(state, op) {
      const key = totalOrderKey(op);
      if (state.key === null || keyGreater(key, state.key)) {
        state.value = op.payload.set;
        state.key = key;
      }
    },
    read: (state) => ({ value: state.value }),
  },

  gcounter: {
    init: () => ({ perPeer: new Map() }),
    apply(state, op) {
      const n = op.payload.inc;
      if (!(Number.isInteger(n) && n > 0)) throw new Error(`gcounter inc must be a positive int, got ${n}`);
      state.perPeer.set(op.author, (state.perPeer.get(op.author) ?? 0) + n);
    },
    read: (state) => ({ value: [...state.perPeer.values()].reduce((a, b) => a + b, 0) }),
  },

  pncounter: {
    init: () => ({ pos: new Map(), neg: new Map() }),
    apply(state, op) {
      const inc = op.payload.inc !== undefined;
      const n = inc ? op.payload.inc : op.payload.dec;
      if (!(Number.isInteger(n) && n > 0)) throw new Error(`pncounter amount must be a positive int, got ${n}`);
      const map = inc ? state.pos : state.neg;
      map.set(op.author, (map.get(op.author) ?? 0) + n);
    },
    read: (state) => ({
      value:
        [...state.pos.values()].reduce((a, b) => a + b, 0) -
        [...state.neg.values()].reduce((a, b) => a + b, 0),
    }),
  },

  orset: {
    // dots keyed by the adding operation's op_id; removes tombstone exactly
    // the dots they observed. Tombstones win over their own dot regardless
    // of arrival order; unobserved concurrent adds survive.
    init: () => ({ addDots: new Map(), tombstones: new Set() }),
    apply(state, op) {
      if (op.payload.add !== undefined) {
        state.addDots.set(op.op_id, op.payload.add);
      } else if (op.payload.remove !== undefined) {
        for (const dot of op.payload.observed) state.tombstones.add(dot);
      } else {
        throw new Error('orset payload must be add or remove');
      }
    },
    read: (state) => {
      const elements = new Set();
      for (const [dot, elem] of state.addDots) {
        if (!state.tombstones.has(dot)) elements.add(elem);
      }
      return { elements: [...elements].sort() };
    },
  },

  flag: {
    init: () => ({ enableDots: new Map(), tombstones: new Set() }),
    apply(state, op) {
      if (op.payload.enable !== undefined) {
        state.enableDots.set(op.op_id, true);
      } else if (op.payload.disable !== undefined) {
        for (const dot of op.payload.observed) state.tombstones.add(dot);
      } else {
        throw new Error('flag payload must be enable or disable');
      }
    },
    read: (state) => {
      for (const dot of state.enableDots.keys()) {
        if (!state.tombstones.has(dot)) return { enabled: true };
      }
      return { enabled: false };
    },
  },
};

function deepEqual(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function runCrdtVector(vector) {
  const crdt = CRDTS[vector.crdt];
  if (!crdt) throw new Error(`unknown crdt "${vector.crdt}"`);

  vector.orders.forEach((order, oi) => {
    const state = crdt.init();
    const seen = new Set(); // pipeline step 4: dedup by op_id
    for (const index of order) {
      const op = vector.ops[index];
      if (op === undefined) throw new Error(`order ${oi}: bad op index ${index}`);
      if (seen.has(op.op_id)) continue;
      seen.add(op.op_id);
      crdt.apply(state, op);
    }
    const got = crdt.read(state);
    const expect = { ...vector.expect };
    if (expect.elements) expect.elements = [...expect.elements].sort();
    if (!deepEqual(got, expect)) {
      throw new Error(
        `order ${oi} [${order.join(',')}]: expected ${JSON.stringify(expect)}, got ${JSON.stringify(got)}`
      );
    }
  });
}

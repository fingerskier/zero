// migration-transform handler per doc/SCHEMA.md §4/§6: each registry transform
// tested in isolation (input materialized state -> output lww value), reusing
// the exact transform function the epoch model applies at a change_crdt
// boundary (single source of truth).

import { transformValue } from './epoch.mjs';

// Build the KERNEL §6 read shape from a vector's {crdt, value} input.
function inputRead(input) {
  switch (input.crdt) {
    case 'lww':
    case 'gcounter':
    case 'pncounter':
      return { value: input.value };
    case 'flag':
      return { enabled: input.value };
    case 'orset':
      return { elements: input.value };
    default:
      throw new Error(`unknown input crdt "${input.crdt}"`);
  }
}

export function runMigrationTransformVector(vector) {
  vector.cases.forEach((c, ci) => {
    const got = transformValue(vector.transform, vector.default, inputRead(c.input));
    const want = c.output.value;
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      throw new Error(
        `case ${ci} (${JSON.stringify(c.input)}): expected ${JSON.stringify(want)}, got ${JSON.stringify(got)}`
      );
    }
  });
}

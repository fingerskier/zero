// Compatibility shim. Canonical IndexedDB + OPFS adapters live in
// zerodb-wasm/js/storage.mjs (M4a-a). Pages deploy copies that file beside
// this shim as ./storage.mjs.

export {
  IndexedDbJournal as ZeroJournal,
  IndexedDbJournal,
  OpfsJournal,
  DurableJournal,
  openJournal,
  openDurable,
  detectAdapter,
  ADAPTER_INDEXEDDB,
  ADAPTER_OPFS,
} from '../../zerodb-wasm/js/storage.mjs'

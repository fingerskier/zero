// Dev shim: re-export the canonical sync driver from examples/browser-peer
// (works when serving the repo root). The Pages workflow replaces this file
// with a copy of the real module so the deployed app is self-contained.
export * from '../../examples/browser-peer/zero-sync.mjs'

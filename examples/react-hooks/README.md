# React hooks example (M4a slice)

Thin static page that uses `@zerodb/react` over `zerodb-wasm` +
`openDurable`. Not a rewrite of the vanilla `examples/browser-peer`
demo. **Not** M4a complete: no WebRTC / H6, no live `useSyncStatus`
sync session.

```sh
bash zerodb-wasm/scripts/build.sh
npx serve .   # open /examples/react-hooks/
```

Identity seed lives in origin storage. Live store is MemoryBackend;
the journal is signed KERNEL ops.

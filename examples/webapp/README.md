# ZeroDB back&forth webapp

Two `@zerodb/node` LocalStores ("peer A"/"peer B") behind one zero-dependency Node server. Mutate either pane, watch live subscribe events (SSE), then sync — export bundles are exchanged both directions and the panes converge.

## Run

```bash
cd zerodb-napi && npm run build   # once, builds the native addon
cd ../examples/webapp && node server.mjs
# open http://localhost:8787
```

Data lives in `examples/webapp/data/*.sqlite` (disposable, experimental format — delete freely).

## What it demonstrates

- CRUD + LWW/set/flag props via the NAPI SDK
- `subscribe` live change events streamed to the browser
- Bidirectional convergence through `exportJson`/`importJson` (same set-diff model as the CLI TCP path)

import { createElement, useState } from 'react'
import { createRoot } from 'react-dom/client'
import {
  ZeroDbProvider,
  useMutation,
  useQuery,
  useSyncStatus,
} from '../../zerodb-react/index.mjs'

const wasmMod = await import('../../zerodb-wasm/pkg/zerodb_wasm.js')
await wasmMod.default()
const { ZeroDb } = wasmMod

function TodoApp() {
  const sync = useSyncStatus()
  const { rows, ready } = useQuery('MATCH (t:Todo) RETURN t.title')
  const mutate = useMutation()
  const [title, setTitle] = useState('milk')

  return createElement('div', null,
    createElement('p', null, `store: ${sync}${ready ? '' : ' (opening)'}`),
    createElement('input', {
      value: title,
      onChange: e => setTitle(e.target.value),
    }),
    createElement('button', {
      disabled: sync !== 'ready',
      onClick: async () => {
        const id = await mutate.createNode('Todo')
        await mutate.setLww(id, 'title', title || 'untitled')
      },
    }, 'add todo'),
    createElement('ul', null, ...(rows || []).map((row, i) =>
      createElement('li', { key: i }, row['t.title'] ?? ''),
    )),
  )
}

createRoot(document.getElementById('root')).render(
  createElement(ZeroDbProvider, {
    ZeroDb,
    name: 'zerodb-react-hooks',
    adapter: 'auto',
  }, createElement(TodoApp)),
)

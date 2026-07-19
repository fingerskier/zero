import { test } from 'node:test'
import assert from 'node:assert/strict'
import { compile } from './ts-to-ir.mjs'

test('compile authoring to pin IR', () => {
  const ir = compile({
    name: 'todo',
    nodes: {
      Todo: {
        title: { crdt: 'lww', type: 'text' },
        done: { crdt: 'flag' },
        views: { crdt: 'gcounter' },
      },
    },
  })
  assert.equal(ir.nodes.Todo.props.title, 'lww')
  assert.equal(ir.nodes.Todo.props.done, 'flag')
  assert.equal(ir.nodes.Todo.props.views, 'gcounter')
})

test('rejects unique', () => {
  assert.throws(
    () =>
      compile({
        nodes: { Todo: { title: { crdt: 'lww', unique: true } } },
      }),
    /unique/,
  )
})

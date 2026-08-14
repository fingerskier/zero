import { test } from 'node:test'
import assert from 'node:assert/strict'
import { compile } from './ts-to-ir.mjs'

test('compile authoring to SCHEMA IR JSON', () => {
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
  assert.equal(ir.v, 1)
  assert.equal(ir.nodes.Todo.props.title.crdt, 0)
  assert.equal(ir.nodes.Todo.props.title.type, 4)
  assert.equal(ir.nodes.Todo.props.done.crdt, 4)
  assert.equal(ir.nodes.Todo.props.views.crdt, 1)
  assert.equal(ir.nodes.Todo.props.views.type, 2)
  assert.deepEqual(ir.edges, {})
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

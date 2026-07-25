// Todo data layer over a zerodb store (wasm ZeroDb or NAPI Database — both
// expose the same camelCase surface). Keeps the DOM page thin and the model
// Node-testable.
//
// Schema (EXEMPLAR-flavored, doc/EXEMPLAR.md): a todo is a node labeled
// 'Todo' with `title: LWW<string>`, `done: Flag` (enable wins), and
// `tags: ORSet<string>` (concurrent add+remove => present). Delete is a
// node tombstone.

export const TODO_LABEL = 'Todo'

/** Create a todo node with a title. Returns the node id. */
export function createTodo(db, title) {
  const id = db.createNode(TODO_LABEL)
  db.setLww(id, 'title', title)
  return id
}

export function setTitle(db, id, title) {
  db.setLww(id, 'title', title)
}

export function isDone(db, id) {
  return db.getProp(id, 'done') === true
}

/** Flip the done flag (Flag CRDT: enable/disable). Returns the new state. */
export function toggleDone(db, id) {
  if (isDone(db, id)) {
    db.flagDisable(id, 'done')
    return false
  }
  db.flagEnable(id, 'done')
  return true
}

export function addTag(db, id, tag) {
  db.setAdd(id, 'tags', tag)
}

export function removeTag(db, id, tag) {
  db.setRemove(id, 'tags', tag)
}

/** Tombstone the todo node. */
export function deleteTodo(db, id) {
  db.deleteNode(id)
}

/**
 * Materialized todos as `[{ id, title, done, tags }]`, tombstones excluded.
 * `filter`: 'all' | 'active' | 'done'.
 */
export function listTodos(db, filter = 'all') {
  const todos = db
    .listNodes()
    .filter(n => n.label === TODO_LABEL && !n.deleted)
    .map(n => ({
      id: n.id,
      title: typeof n.props?.title === 'string' ? n.props.title : '',
      done: n.props?.done === true,
      tags: Array.isArray(n.props?.tags) ? [...n.props.tags].sort() : [],
    }))
  if (filter === 'active') return todos.filter(t => !t.done)
  if (filter === 'done') return todos.filter(t => t.done)
  return todos
}

// todo: JSON API with bearer tokens and a single data/db.json. Task definitions for the generator.
const T1 = {
  E_filter: {
    flags: { E_filter: true },
    prompt:
      'Add a done filter to the todo list: GET /todos?done=true returns only my finished todos and GET /todos?done=false only the open ones. Without the parameter it lists everything as before.',
    praise: 'The done filter works now, for both true and false.',
    checks: [{ id: 'p-done-filter', turn: 1 }],
  },
  E_done: {
    flags: { E_done: true },
    prompt:
      'PATCH /todos/:id only changes the title, and a change is lost when the server restarts. Please let PATCH set done (true or false) as well as the title, change only the fields that are sent, and make sure the change is saved in data/db.json.',
    praise: 'Marking a todo done works now, and it sticks after a restart.',
    checks: [{ id: 'p-mark-done', turn: 1 }],
  },
  E_logout: {
    flags: { E_logout: true },
    prompt:
      'There is no way to log out: a token stays valid forever. revokeToken in lib/auth.js already removes a token but nothing calls it. Please add POST /logout that uses it to revoke the token sent with the request. It answers 204, and afterwards that token gets 401 on every route.',
    praise: 'Logout works now: the old token is dead after I log out.',
    checks: [{ id: 'p-logout-revokes', turn: 1 }],
  },
};

const T3 = {
  R_tags: {
    flags: { R_tags: 'ref' },
    prompt:
      'Add tags to todos. POST /todos accepts an optional tags array of strings (default none). Every todo in every response has a tags array, empty when the todo has none. GET /todos?tag=<name> returns only the todos that have that tag.',
    checks: [{ id: 'r-tags', turn: 3 }],
  },
  R_validate: {
    flags: { R_validate: 'ref' },
    prompt:
      'Validate the input on POST /todos and PATCH /todos/:id: a title must be 1 to 100 characters and done must be true or false. Anything invalid answers 400.',
    checks: [{ id: 'r-validate', turn: 3 }],
  },
  R_router: {
    flags: { R_router: 'ref' },
    prompt:
      'server.js routes requests with a long if-chain. Move the routing into a table of routes (method, path pattern, handler) so a new endpoint is a single entry. Behaviour stays the same, with two additions: an unknown path answers 404 with { "error": "not found" } and a known path used with the wrong method answers 405.',
    checks: [{ id: 'r-router', turn: 3 }],
  },
  N_priority: {
    flags: { N_priority: true },
    prompt:
      'Add a priority to todos. POST /todos accepts an optional priority: low, normal or high (default normal); anything else answers 400. Every todo in a response shows its priority, and GET /todos?priority=high returns only the todos with that priority.',
    checks: [{ id: 'n-priority', turn: 3 }],
  },
  N_search: {
    flags: { N_search: true },
    prompt:
      'Add search to the todo list: GET /todos?q=<text> returns only my todos whose title contains the text, ignoring capitalisation. Without q it lists everything as before.',
    checks: [{ id: 'n-search', turn: 3 }],
  },
};

export default {
  app: 'todo',
  base: [
    'b-root',
    'b-signup',
    'b-signup-dup',
    'b-token',
    'b-anon',
    'b-list-legacy',
    'b-todos-crud',
    'b-todos-private',
    'b-persist-create',
    'b-delete',
  ],
  shared: {
    flags: { S4: true, S5: true, S6: true },
    prompts: [
      'Add GET /health that answers 200 with { "status": "ok" } and needs no token.',
      'Add GET /stats for the logged-in user: it answers 200 with { "total": <all their todos>, "done": <the finished ones>, "open": <the rest> }.',
      "Add POST /todos/clear-done: it deletes the logged-in user's finished todos and answers 200 with { \"removed\": <how many> }.",
    ],
    checks: [
      { id: 'l4-health', turn: 4 },
      { id: 'l5-stats', turn: 5 },
      { id: 'l6-clear-done', turn: 6 },
    ],
  },
  t1: T1,
  t3: T3,
  tasks: [
    {
      id: 'todo-tags',
      kind: 'natural-risk',
      t1: 'E_filter',
      t3: 'R_tags',
      slip: { R_tags: 'slip' },
      fails: ['b-list-legacy'],
      risk: 'The tags serializer assumes every todo has a tags array, so the stored legacy todos (alice has three, without tags) make GET /todos crash with a 500. The done-filter check uses fresh users and does not see it.',
      note: 'present() calls todo.tags.slice() on legacy todos',
      help: 'maybe',
    },
    {
      id: 'todo-validate',
      kind: 'natural-risk',
      t1: 'E_done',
      t3: 'R_validate',
      slip: { R_validate: 'slip' },
      fails: ['p-mark-done'],
      risk: 'The PATCH validation copies the create rule and demands a title, so PATCH {done:true} (the praised behaviour) answers 400.',
      note: 'patchTodo requires a title',
      help: 'yes',
    },
    {
      id: 'todo-router',
      kind: 'natural-risk',
      t1: 'E_filter',
      t3: 'R_router',
      slip: { R_router: 'slip' },
      fails: ['p-done-filter'],
      risk: 'The route table matches the raw request URL including the query string, so GET /todos?done=true no longer finds a route (404): the praised filter is gone.',
      note: 'dispatch() matches req.url instead of the path',
      help: 'yes',
    },
    {
      id: 'todo-logout',
      kind: 'injected',
      t1: 'E_logout',
      t3: 'N_priority',
      inject: { I_revoke: true },
      fails: ['p-logout-revokes'],
      risk: 'Injected: revokeToken() compares against the upper-cased token, so no token is ever removed and logout does nothing.',
      note: 'inject.patch edits revokeToken in lib/auth.js',
      help: 'yes',
    },
    {
      id: 'todo-persist',
      kind: 'injected',
      t1: 'E_done',
      t3: 'N_search',
      inject: { I_save: true },
      fails: ['b-persist-create', 'p-mark-done'],
      risk: 'Injected: save() writes to data/db.json.tmp, so nothing survives a restart.',
      note: 'inject.patch edits save() in lib/db.js',
      help: 'yes',
    },
    {
      id: 'todo-delete',
      kind: 'injected',
      t1: 'E_filter',
      t3: 'N_priority',
      inject: { I_delete: true },
      fails: ['b-delete'],
      risk: 'Injected: removeTodo() splices zero items, so DELETE answers 204 but the todo stays. Nobody praised deleting: no promise covers it.',
      note: 'inject.patch edits removeTodo in lib/db.js',
      help: 'no',
    },
    {
      id: 'todo-ctrl',
      kind: 'control',
      t1: 'E_done',
      t3: 'N_search',
      drop: { S6: false },
      fails: ['l6-clear-done'],
      risk: null,
      note: 'self-test only: POST /todos/clear-done removed; the real episode has no break',
      help: 'no',
    },
  ],
};

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createActor, createMachine } from 'xstate';

const here = path.dirname(fileURLToPath(import.meta.url));
const fixtureDir = path.resolve(
  here,
  '../../crates/elm-effect-xstate/tests/conformance/fixtures'
);

const FIXTURES = [
  'actions.json',
  'always.json',
  'final_states.json',
  'guards.json',
  'hierarchy.json',
  'history.json',
  'parallel.json'
];

const SKIP = new Set(['emitted_payloads_reach_inspection']);
const checkTrace = process.argv.includes('--trace');

function childrenByParent(states) {
  const children = new Map();
  for (const [name, node] of Object.entries(states)) {
    const parent = node.parent ?? null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(name);
  }
  return children;
}

function normalizeActions(actions) {
  if (!actions) return [];
  return Array.isArray(actions) ? actions : [actions];
}

function runActions(actions, args, enq, trace) {
  const patch = {};
  const next = { ...args.context, ...patch };
  for (const action of normalizeActions(actions)) {
    if (action.startsWith('raise:')) {
      enq.raise({ type: action.slice('raise:'.length) });
    } else if (action.startsWith('emit:')) {
      enq.emit({ type: action.slice('emit:'.length) });
    } else if (action.startsWith('assign:')) {
      const match = /^assign:([A-Za-z0-9_]+)\+=(-?\d+)$/.exec(action);
      if (!match) throw new Error(`unsupported assign action: ${action}`);
      const [, key, amount] = match;
      next[key] = (next[key] ?? 0) + Number(amount);
      patch[key] = next[key];
    } else {
      trace.push(action);
    }
  }
  return Object.keys(patch).length > 0 ? patch : undefined;
}

function lifecycle(actions, trace) {
  if (!actions) return undefined;
  return (args, enq) => {
    const patch = runActions(actions, args, enq, trace);
    return patch ? { context: patch } : undefined;
  };
}

function guardPasses(expression, args) {
  if (!expression) return true;
  const keys = Object.keys(args.context);
  const predicate = new Function(
    'context',
    'event',
    ...keys,
    `return (${expression});`
  );
  return predicate(
    args.context,
    args.event,
    ...keys.map((key) => args.context[key])
  );
}

function transition(rule, trace) {
  return (args, enq) => {
    if (!guardPasses(rule.guard, args)) return undefined;
    const patch = runActions(rule.actions, args, enq, trace);
    const result = { target: `#${rule.target}` };
    if (patch) result.context = patch;
    return result;
  };
}

function rule(value, trace) {
  if (typeof value === 'string') {
    return () => ({ target: `#${value}` });
  }
  if (Array.isArray(value)) {
    return value.map((entry) => transition(entry, trace));
  }
  return transition(value, trace);
}

function state(name, states, children, trace) {
  const raw = states[name];
  const node = { id: name };

  if (raw.type === 'parallel') node.type = 'parallel';
  if (raw.type === 'final') node.type = 'final';
  if (raw.type === 'history') {
    node.type = 'history';
    if (raw.history) node.history = raw.history;
    if (raw.initial) node.target = `#${raw.initial}`;
    return node;
  }

  if (raw.initial) node.initial = raw.initial;
  const entry = lifecycle(raw.entry, trace);
  if (entry) node.entry = entry;
  const exit = lifecycle(raw.exit, trace);
  if (exit) node.exit = exit;

  const childNames = children.get(name) ?? [];
  if (childNames.length > 0) {
    node.states = Object.fromEntries(
      childNames.map((child) => [child, state(child, states, children, trace)])
    );
  }
  if (raw.on) {
    node.on = Object.fromEntries(
      Object.entries(raw.on).map(([event, value]) => [event, rule(value, trace)])
    );
  }
  if (raw.always) {
    node.always = raw.always.map((entry) => transition(entry, trace));
  }
  if (raw.onDone) {
    node.onDone = rule(raw.onDone, trace);
  }
  return node;
}

function machineFor(config, trace) {
  const states = config.states ?? {};
  const children = childrenByParent(states);
  const root = {
    context: config.context ?? {},
    initial: config.initial,
    states: Object.fromEntries(
      (children.get(null) ?? []).map((name) => [
        name,
        state(name, states, children, trace)
      ])
    )
  };
  return createMachine(root);
}

function leaves(value, out = []) {
  if (typeof value === 'string') {
    out.push(value);
    return out;
  }
  if (value && typeof value === 'object') {
    for (const [key, child] of Object.entries(value)) {
      if (child && typeof child === 'object' && Object.keys(child).length === 0) {
        out.push(key);
      } else {
        leaves(child, out);
      }
    }
  }
  return out;
}

function sorted(values) {
  return [...new Set(values)].sort();
}

function equalSets(actual, expected) {
  const a = sorted(actual);
  const e = sorted(expected);
  return a.length === e.length && a.every((value, index) => value === e[index]);
}

function compare(caseName, expected, trace, snapshot) {
  const failures = [];
  const actualLeaves = leaves(snapshot.value);
  const expectedLeaves = expected.final_states ?? [expected.final_state];
  if (!equalSets(actualLeaves, expectedLeaves)) {
    failures.push(
      `leaves: expected ${JSON.stringify(sorted(expectedLeaves))}, got ${JSON.stringify(sorted(actualLeaves))}`
    );
  }
  if (
    checkTrace &&
    expected.trace &&
    JSON.stringify(trace) !== JSON.stringify(expected.trace)
  ) {
    failures.push(
      `trace: expected ${JSON.stringify(expected.trace)}, got ${JSON.stringify(trace)}`
    );
  }
  if (expected.done !== undefined && (snapshot.status === 'done') !== expected.done) {
    failures.push(
      `done: expected ${expected.done}, got ${snapshot.status === 'done'}`
    );
  }
  if (expected.final_data) {
    for (const [key, value] of Object.entries(expected.final_data)) {
      if (JSON.stringify(snapshot.context[key]) !== JSON.stringify(value)) {
        failures.push(
          `context.${key}: expected ${JSON.stringify(value)}, got ${JSON.stringify(snapshot.context[key])}`
        );
      }
    }
  }
  return failures.map((failure) => `${caseName}: ${failure}`);
}

function runFixture(file) {
  const cases = JSON.parse(fs.readFileSync(path.join(fixtureDir, file), 'utf8'));
  const failures = [];
  let checked = 0;
  let skipped = 0;

  for (const testCase of cases) {
    if (SKIP.has(testCase.name) || testCase.expected.emitted) {
      skipped++;
      continue;
    }
    const trace = [];
    try {
      const machine = machineFor(testCase.machine, trace);
      const actor = createActor(machine).start();
      for (const event of testCase.events) actor.send(event);
      const snapshot = actor.getSnapshot();
      failures.push(...compare(testCase.name, testCase.expected, trace, snapshot));
      checked++;
    } catch (error) {
      failures.push(`${testCase.name}: ${error.message}`);
    }
  }
  return { checked, skipped, failures };
}

let total = 0;
let passed = 0;
let skipped = 0;
const failures = [];

for (const fixture of FIXTURES) {
  const result = runFixture(fixture);
  total += result.checked + result.skipped;
  passed += result.checked;
  skipped += result.skipped;
  failures.push(...result.failures);
  console.log(
    `${fixture}: ${result.checked} checked, ${result.skipped} skipped`
  );
}

if (failures.length > 0) {
  console.error('\nDifferential failures:');
  for (const failure of failures) console.error(`- ${failure}`);
  process.exitCode = 1;
} else {
  console.log(
    `\nXState differential (${checkTrace ? 'state + trace' : 'state outcomes'}): ${passed} cases agree, ${skipped} skipped of ${total}`
  );
}

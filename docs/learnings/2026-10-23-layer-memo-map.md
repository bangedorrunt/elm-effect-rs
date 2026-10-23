---
title: Layer identity is the node, not the handle
status: landed
---

# Sharing needed an identity, not a bigger map

The old `LayerMemo` was one `Option<Services>` slot per handle. That can
memoize "this handle built once", and nothing else. Every pin row about
sharing, `fresh`, scope refcounts, or forked memo maps was excused for the
same reason: a value has no identity to key on.

The fix was structural. `Layer` is now a handle onto an `Arc<Node>` tree:
stages, merge, provide, wrap, fresh, suspend, hooks. Each node has a u64 id,
and `LayerMemo` is a map from that id to an entry. Composition keeps its
children instead of flattening their stages, so `layer.merge(layer)` asks one
node twice and gets one build.

## What the pin forced

- **Only constructors are memoized.** `Layer.effect` and `Layer.suspend` are
  `fromBuildMemo`; `merge`, `provide`, `tap`, `withSpan`, `fresh`, `catch`
  are plain `fromBuild`. That split is what makes `Layer.test.ts:621` release
  the consumer twice while constructing the provider once. A tree that
  memoized every node would pass the sharing rows and fail that one.
- **An entry owns its scope, and observers refcount it.** The entry scope is
  standalone; each caller scope registers a finalizer that decrements. The
  last one closes the entry scope and removes the entry. That single rule
  produces `:554`, `:572`, `:621`, `:641`, and `:660` without a special case
  for any of them.
- **`get` observes when it runs, not when it is called.** `MemoMap.get`
  returns an effect; peeking is not observing (`:572`). `buildWithMemoMap`
  returns a `suspend` for the same reason (`:586`).
- **`fresh` is a new memo map for the child, not an empty slot.** It stays a
  plain node, so building the same fresh handle twice rebuilds the child.
- **Composition merges like `Context.mergeAll`: the later side wins.** This
  retired the port's old "a doubled tag is always a construction error" law
  for layer-to-layer composition. The fail-closed rule survives at
  `Effect::provide` and `Layer::provide`, where the port invented it and no
  pin row contradicts it.

## Rows

Thirteen rows moved to `ported`: `:35`, `:75`, `:101`, `:231`, `:240`, `:252`,
`:357`, `:554`, `:572`, `:586`, `:621`, `:641`, `:660`. Layer now reads
24 ported, 15 excused; every excuse names the API that would unblock it
(`Layer.mergeAll` parallelism, an interruptible construction fiber,
`Layer.orDie`'s never type, `Layer.mock`'s `PartialEffectful`,
`Layer.launch`'s never effect, an in-flight memo entry, a stack-frame
service).

## Gotcha that cost a pass

`witness-audit.py`'s "specific" check is case-sensitive and wants a
`Type.member` token or `crates/...rs:N`. A reason that says "Would need ..."
in prose still counts as narrative. Name the absent API with its capitalised
path, or the row comes back vague even though the reason is precise.

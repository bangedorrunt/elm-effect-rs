// ADR-0008: Declarative compile-time statechart! macro (Law L11)
// See: docs/decisions/0008-harden-elm-effect-xstate-with-compiled-topology-timer-tokens-and-macro-dx.md

/// Hidden runtime helpers for the [`crate::statechart!`] macro.
#[doc(hidden)]
pub mod __macro_support {
    use crate::action::ActionQueue;

    #[inline]
    pub fn call_guard<C>(ctx: &C, f: impl FnOnce(&C) -> bool) -> bool {
        f(ctx)
    }

    #[inline]
    pub fn call_action<C, CMD, E>(
        ctx: &mut C,
        enq: &mut ActionQueue<CMD, E>,
        f: impl FnOnce(&mut C, &mut ActionQueue<CMD, E>),
    ) {
        f(ctx, enq);
    }
}

/// Declaratively build a typed [`Machine`](crate::Machine) with nested hierarchy,
/// automatic parent wiring, automatic edge labels, and single-match event payload
/// destructuring.
///
/// ```
/// use elm_effect_xstate::prelude::*;
///
/// #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// enum Light { Green, Yellow, Red }
/// #[derive(Debug, Clone)]
/// enum Ev { Next, Add(u32) }
/// #[derive(Debug, Clone, Default)]
/// struct Ctx { ticks: u32 }
///
/// let machine = statechart! {
///     state: Light,
///     event: Ev,
///     context: Ctx,
///     command: (),
///     initial: Light::Green,
///     states: {
///         state Light::Green {
///             on: {
///                 Ev::Add(n) if *n > 0 => {
///                     target: Light::Yellow,
///                     action: |ctx, _| ctx.ticks += *n,
///                 },
///                 Ev::Next => target Light::Yellow,
///             },
///         },
///         state Light::Yellow {
///             on: { Ev::Next => target Light::Red },
///         },
///         final Light::Red {},
///     },
/// };
/// let out = machine.step(&Light::Green, &Ctx::default(), &Ev::Add(2));
/// assert_eq!(out.current_state, Light::Yellow);
/// assert_eq!(out.context.ticks, 2);
/// ```
#[macro_export]
macro_rules! statechart {
    // Entry point with explicit context_init.
    (
        state: $S:ty,
        event: $E:ty,
        context: $C:ty,
        command: $CMD:ty,
        initial: $init:expr,
        context_init: $ctx_init:expr,
        states: { $($body:tt)* } $(,)?
    ) => {{
        let builder: $crate::MachineBuilder<$S, $E, $C, $CMD> =
            $crate::Machine::builder($init, $ctx_init);
        let builder = $crate::statechart!(@nodes builder, ::core::option::Option::None, $($body)*);
        builder.build()
    }};

    // Entry point defaulting context_init to Default::default().
    (
        state: $S:ty,
        event: $E:ty,
        context: $C:ty,
        command: $CMD:ty,
        initial: $init:expr,
        states: { $($body:tt)* } $(,)?
    ) => {{
        $crate::statechart! {
            state: $S,
            event: $E,
            context: $C,
            command: $CMD,
            initial: $init,
            context_init: <$C as ::core::default::Default>::default(),
            states: { $($body)* }
        }
    }};

    // TT-muncher over sibling nodes at a given parent level.
    (@nodes $builder:ident, $parent:expr, ) => { $builder };
    (@nodes $builder:ident, $parent:expr, , $($rest:tt)*) => {
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    };

    // Atomic state.
    (@nodes $builder:ident, $parent:expr, state $($rest:tt)*) => {
        $crate::statechart!(@collect_brace state $builder, $parent, [] $($rest)*)
    };
    // Compound state (may contain nested `states: { ... }`).
    (@nodes $builder:ident, $parent:expr, compound $($rest:tt)*) => {
        $crate::statechart!(@collect_paren compound $builder, $parent, [] $($rest)*)
    };
    // Parallel state (may contain nested `states: { ... }`).
    (@nodes $builder:ident, $parent:expr, parallel $($rest:tt)*) => {
        $crate::statechart!(@collect_brace parallel $builder, $parent, [] $($rest)*)
    };
    // Choice pseudo-state.
    (@nodes $builder:ident, $parent:expr, choice $($rest:tt)*) => {
        $crate::statechart!(@collect_brace choice $builder, $parent, [] $($rest)*)
    };
    // Shallow history state.
    (@nodes $builder:ident, $parent:expr, history $($rest:tt)*) => {
        $crate::statechart!(@collect_paren history $builder, $parent, [] $($rest)*)
    };
    // Deep history state.
    (@nodes $builder:ident, $parent:expr, deep_history $($rest:tt)*) => {
        $crate::statechart!(@collect_paren deep_history $builder, $parent, [] $($rest)*)
    };
    // Final state.
    (@nodes $builder:ident, $parent:expr, final $($rest:tt)*) => {
        $crate::statechart!(@collect_brace final $builder, $parent, [] $($rest)*)
    };

    // Collect arbitrary node ID tokens up to `=> { ... }` or `{ ... }`.
    (@collect_brace $kind:ident $builder:ident, $parent:expr, [ $($id:tt)+ ] => { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_brace $kind $builder, $parent, ($($id)+), { $($cfg)* } $($rest)*)
    };
    (@collect_brace $kind:ident $builder:ident, $parent:expr, [ $($id:tt)+ ] { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_brace $kind $builder, $parent, ($($id)+), { $($cfg)* } $($rest)*)
    };
    (@collect_brace $kind:ident $builder:ident, $parent:expr, [ $($acc:tt)* ] $tok:tt $($rest:tt)*) => {
        $crate::statechart!(@collect_brace $kind $builder, $parent, [ $($acc)* $tok ] $($rest)*)
    };

    // Collect arbitrary node ID tokens up to `(initial: ...)` or `(fallback: ...)` before `{ ... }`.
    (@collect_paren compound $builder:ident, $parent:expr, [ $($id:tt)+ ] => (initial: $initial:expr) { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_compound $builder, $parent, ($($id)+), $initial, { $($cfg)* } $($rest)*)
    };
    (@collect_paren compound $builder:ident, $parent:expr, [ $($id:tt)+ ] (initial: $initial:expr) { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_compound $builder, $parent, ($($id)+), $initial, { $($cfg)* } $($rest)*)
    };
    (@collect_paren history $builder:ident, $parent:expr, [ $($id:tt)+ ] => (fallback: $fb:expr) { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_history $builder, $parent, ($($id)+), $fb, { $($cfg)* } $($rest)*)
    };
    (@collect_paren history $builder:ident, $parent:expr, [ $($id:tt)+ ] (fallback: $fb:expr) { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_history $builder, $parent, ($($id)+), $fb, { $($cfg)* } $($rest)*)
    };
    (@collect_paren deep_history $builder:ident, $parent:expr, [ $($id:tt)+ ] => (fallback: $fb:expr) { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_deep_history $builder, $parent, ($($id)+), $fb, { $($cfg)* } $($rest)*)
    };
    (@collect_paren deep_history $builder:ident, $parent:expr, [ $($id:tt)+ ] (fallback: $fb:expr) { $($cfg:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@emit_deep_history $builder, $parent, ($($id)+), $fb, { $($cfg)* } $($rest)*)
    };
    (@collect_paren $kind:ident $builder:ident, $parent:expr, [ $($acc:tt)* ] $tok:tt $($rest:tt)*) => {
        $crate::statechart!(@collect_paren $kind $builder, $parent, [ $($acc)* $tok ] $($rest)*)
    };

    // Emit nodes once delimiters isolate `$id:expr`.
    (@emit_brace state $builder:ident, $parent:expr, ($id:expr), { $($cfg:tt)* } $($rest:tt)*) => {{
        let __id = $id;
        let $builder = $builder.state(__id.clone(), |mut __node| {
            if let ::core::option::Option::Some(__p) = $parent {
                __node = __node.with_parent(__p);
            }
            $crate::statechart!(@cfg __node, $($cfg)*)
        });
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    }};
    (@emit_brace parallel $builder:ident, $parent:expr, ($id:expr), { $($cfg:tt)* } $($rest:tt)*) => {{
        let __id = $id;
        let $builder = $builder.parallel(__id.clone(), |mut __node| {
            if let ::core::option::Option::Some(__p) = $parent {
                __node = __node.with_parent(__p);
            }
            $crate::statechart!(@cfg __node, $($cfg)*)
        });
        let $builder = $crate::statechart!(@substates $builder, __id, $($cfg)*);
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    }};
    (@emit_brace choice $builder:ident, $parent:expr, ($id:expr), { $($cfg:tt)* } $($rest:tt)*) => {{
        let __id = $id;
        let $builder = $builder.choice(__id.clone(), |mut __node| {
            if let ::core::option::Option::Some(__p) = $parent {
                __node = __node.with_parent(__p);
            }
            $crate::statechart!(@cfg __node, $($cfg)*)
        });
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    }};
    (@emit_brace final $builder:ident, $parent:expr, ($id:expr), { $($cfg:tt)* } $($rest:tt)*) => {{
        let __id = $id;
        let $builder = $builder.final_state(__id.clone(), |mut __node| {
            if let ::core::option::Option::Some(__p) = $parent {
                __node = __node.with_parent(__p);
            }
            $crate::statechart!(@cfg __node, $($cfg)*)
        });
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    }};
    (@emit_compound $builder:ident, $parent:expr, ($id:expr), $initial:expr, { $($cfg:tt)* } $($rest:tt)*) => {{
        let __id = $id;
        let $builder = $builder.compound(__id.clone(), $initial, |mut __node| {
            if let ::core::option::Option::Some(__p) = $parent {
                __node = __node.with_parent(__p);
            }
            $crate::statechart!(@cfg __node, $($cfg)*)
        });
        let $builder = $crate::statechart!(@substates $builder, __id, $($cfg)*);
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    }};
    (@emit_history $builder:ident, $parent:expr, ($id:expr), $fb:expr, { $($cfg:tt)* } $($rest:tt)*) => {{
        let __id = $id;
        let $builder = $builder.history(__id.clone(), $fb, |mut __node| {
            if let ::core::option::Option::Some(__p) = $parent {
                __node = __node.with_parent(__p);
            }
            $crate::statechart!(@cfg __node, $($cfg)*)
        });
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    }};
    (@emit_deep_history $builder:ident, $parent:expr, ($id:expr), $fb:expr, { $($cfg:tt)* } $($rest:tt)*) => {{
        let __id = $id;
        let $builder = $builder.deep_history(__id.clone(), $fb, |mut __node| {
            if let ::core::option::Option::Some(__p) = $parent {
                __node = __node.with_parent(__p);
            }
            $crate::statechart!(@cfg __node, $($cfg)*)
        });
        $crate::statechart!(@nodes $builder, $parent, $($rest)*)
    }};

    // Extract nested `states: { ... }` blocks for compound/parallel nodes.
    (@substates $builder:ident, $parent:expr, ) => { $builder };
    (@substates $builder:ident, $parent:expr, states: { $($children:tt)* } $($rest:tt)*) => {{
        let $builder = $crate::statechart!(@nodes $builder, ::core::option::Option::Some($parent.clone()), $($children)*);
        $crate::statechart!(@substates $builder, $parent, $($rest)*)
    }};
    (@substates $builder:ident, $parent:expr, $other:tt $($rest:tt)*) => {
        $crate::statechart!(@substates $builder, $parent, $($rest)*)
    };

    // Configure node-level fields (`entry`, `exit`, `on`, `always`, `after`, `on_done`, skip `states`).
    (@cfg $node:ident, ) => { $node };
    (@cfg $node:ident, , $($rest:tt)*) => {
        $crate::statechart!(@cfg $node, $($rest)*)
    };
    (@cfg $node:ident, states: { $($children:tt)* } $($rest:tt)*) => {
        $crate::statechart!(@cfg $node, $($rest)*)
    };
    (@cfg $node:ident, entry: $f:expr, $($rest:tt)*) => {{
        let $node = $node.entry($f);
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, entry: $f:expr) => {{
        $node.entry($f)
    }};
    (@cfg $node:ident, exit: $f:expr, $($rest:tt)*) => {{
        let $node = $node.exit($f);
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, exit: $f:expr) => {{
        $node.exit($f)
    }};
    (@cfg $node:ident, on: { $($arms:tt)* } $($rest:tt)*) => {{
        let $node = $crate::statechart!(@on $node, $($arms)*);
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, always: [ $($arms:tt)* ] $($rest:tt)*) => {{
        let $node = $crate::statechart!(@always $node, $($arms)*);
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, after: { $($arms:tt)* } $($rest:tt)*) => {{
        let $node = $crate::statechart!(@after $node, $($arms)*);
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, on_done: [ $($arms:tt)* ] $($rest:tt)*) => {{
        let $node = $crate::statechart!(@on_done $node, $($arms)*);
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, on_done: { $($opts:tt)* } $($rest:tt)*) => {{
        let $node = $crate::statechart!(@on_done $node, { $($opts)* });
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, on_done: target: $target:expr, $($rest:tt)*) => {
        $crate::statechart!(@cfg $node, on_done: target $target, $($rest)*)
    };
    (@cfg $node:ident, on_done: target: $target:expr) => {
        $crate::statechart!(@cfg $node, on_done: target $target)
    };
    (@cfg $node:ident, on_done: target $target:expr, $($rest:tt)*) => {{
        let $node = $crate::statechart!(@on_done $node, target $target);
        $crate::statechart!(@cfg $node, $($rest)*)
    }};
    (@cfg $node:ident, on_done: target $target:expr) => {{
        $crate::statechart!(@on_done $node, target $target)
    }};

    // `on:` arms.
    (@on $node:ident, ) => { $node };
    (@on $node:ident, , $($rest:tt)*) => {
        $crate::statechart!(@on $node, $($rest)*)
    };
    (@on $node:ident, $pat:pat => { $($opts:tt)* } $($rest:tt)*) => {{
        let __tr = $crate::statechart!(@pat_build $node, $pat, [none], $($opts)*);
        let $node = $node.on(__tr);
        $crate::statechart!(@on $node, $($rest)*)
    }};
    (@on $node:ident, $pat:pat if $pat_guard:expr => { $($opts:tt)* } $($rest:tt)*) => {{
        let __tr = $crate::statechart!(@pat_build $node, $pat, [some $pat_guard], $($opts)*);
        let $node = $node.on(__tr);
        $crate::statechart!(@on $node, $($rest)*)
    }};
    (@on $node:ident, $pat:pat => target: $target:expr, $($rest:tt)*) => {
        $crate::statechart!(@on $node, $pat => target $target, $($rest)*)
    };
    (@on $node:ident, $pat:pat => target: $target:expr) => {
        $crate::statechart!(@on $node, $pat => target $target)
    };
    (@on $node:ident, $pat:pat if $pat_guard:expr => target: $target:expr, $($rest:tt)*) => {
        $crate::statechart!(@on $node, $pat if $pat_guard => target $target, $($rest)*)
    };
    (@on $node:ident, $pat:pat if $pat_guard:expr => target: $target:expr) => {
        $crate::statechart!(@on $node, $pat if $pat_guard => target $target)
    };
    (@on $node:ident, $pat:pat => target $target:expr, $($rest:tt)*) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat => true,
            _ => false,
        });
        let $node = $node.on(__tr);
        $crate::statechart!(@on $node, $($rest)*)
    }};
    (@on $node:ident, $pat:pat => target $target:expr) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat => true,
            _ => false,
        });
        $node.on(__tr)
    }};
    (@on $node:ident, $pat:pat if $pat_guard:expr => target $target:expr, $($rest:tt)*) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat if $pat_guard => true,
            _ => false,
        });
        let $node = $node.on(__tr);
        $crate::statechart!(@on $node, $($rest)*)
    }};
    (@on $node:ident, $pat:pat if $pat_guard:expr => target $target:expr) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat if $pat_guard => true,
            _ => false,
        });
        $node.on(__tr)
    }};

    // Construct a pattern-matched transition from an option block `{ target: ..., guard: ..., action: ... }`.
    (@pat_build $node:ident, $pat:pat, [none], target: $target:expr, $($opts:tt)*) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat => true,
            _ => false,
        });
        $crate::statechart!(@pat_opts __tr, $pat, [none], $($opts)*)
    }};
    (@pat_build $node:ident, $pat:pat, [none], target: $target:expr) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat => true,
            _ => false,
        });
        __tr
    }};
    (@pat_build $node:ident, $pat:pat, [none], $($opts:tt)*) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.targetless().on_key(stringify!($pat), move |__ev| match __ev {
            $pat => true,
            _ => false,
        });
        $crate::statechart!(@pat_opts __tr, $pat, [none], $($opts)*)
    }};
    (@pat_build $node:ident, $pat:pat, [some $pat_guard:expr], target: $target:expr, $($opts:tt)*) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat if $pat_guard => true,
            _ => false,
        });
        $crate::statechart!(@pat_opts __tr, $pat, [some $pat_guard], $($opts)*)
    }};
    (@pat_build $node:ident, $pat:pat, [some $pat_guard:expr], target: $target:expr) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.transition($target).on_key(stringify!($pat), move |__ev| match __ev {
            $pat if $pat_guard => true,
            _ => false,
        });
        __tr
    }};
    (@pat_build $node:ident, $pat:pat, [some $pat_guard:expr], $($opts:tt)*) => {{
        #[allow(unreachable_patterns, unused_variables)]
        let __tr = $node.targetless().on_key(stringify!($pat), move |__ev| match __ev {
            $pat if $pat_guard => true,
            _ => false,
        });
        $crate::statechart!(@pat_opts __tr, $pat, [some $pat_guard], $($opts)*)
    }};

    // Transition options with pattern-bound variables in scope (`guard`, `action`, `reenter`, `label`).
    (@pat_opts $tr:ident, $pat:pat, $pg:tt, ) => { $tr };
    (@pat_opts $tr:ident, $pat:pat, $pg:tt, , $($rest:tt)*) => {
        $crate::statechart!(@pat_opts $tr, $pat, $pg, $($rest)*)
    };
    (@pat_opts $tr:ident, $pat:pat, $pg:tt, reenter: $re:expr, $($rest:tt)*) => {{
        let $tr = $tr.reenter_with($re);
        $crate::statechart!(@pat_opts $tr, $pat, $pg, $($rest)*)
    }};
    (@pat_opts $tr:ident, $pat:pat, $pg:tt, reenter: $re:expr) => {{
        $tr.reenter_with($re)
    }};
    (@pat_opts $tr:ident, $pat:pat, $pg:tt, label: $lbl:expr, $($rest:tt)*) => {{
        let $tr = $tr.label($lbl);
        $crate::statechart!(@pat_opts $tr, $pat, $pg, $($rest)*)
    }};
    (@pat_opts $tr:ident, $pat:pat, $pg:tt, label: $lbl:expr) => {{
        $tr.label($lbl)
    }};
    (@pat_opts $tr:ident, $pat:pat, [none], guard: $g:expr, $($rest:tt)*) => {{
        let $tr = $tr.guard(move |__ctx, __ev| {
            #[allow(unreachable_patterns, unused_variables)]
            match __ev {
                $pat => $crate::__macro_support::call_guard(__ctx, $g),
                _ => false,
            }
        });
        $crate::statechart!(@pat_opts $tr, $pat, [none], $($rest)*)
    }};
    (@pat_opts $tr:ident, $pat:pat, [none], guard: $g:expr) => {{
        $tr.guard(move |__ctx, __ev| {
            #[allow(unreachable_patterns, unused_variables)]
            match __ev {
                $pat => $crate::__macro_support::call_guard(__ctx, $g),
                _ => false,
            }
        })
    }};
    (@pat_opts $tr:ident, $pat:pat, [some $pat_guard:expr], guard: $g:expr, $($rest:tt)*) => {{
        let $tr = $tr.guard(move |__ctx, __ev| {
            #[allow(unreachable_patterns, unused_variables)]
            match __ev {
                $pat if $pat_guard => $crate::__macro_support::call_guard(__ctx, $g),
                _ => false,
            }
        });
        $crate::statechart!(@pat_opts $tr, $pat, [some $pat_guard], $($rest)*)
    }};
    (@pat_opts $tr:ident, $pat:pat, [some $pat_guard:expr], guard: $g:expr) => {{
        $tr.guard(move |__ctx, __ev| {
            #[allow(unreachable_patterns, unused_variables)]
            match __ev {
                $pat if $pat_guard => $crate::__macro_support::call_guard(__ctx, $g),
                _ => false,
            }
        })
    }};
    (@pat_opts $tr:ident, $pat:pat, [none], action: $a:expr, $($rest:tt)*) => {{
        let $tr = $tr.action(move |__ctx, __ev, __enq| {
            #[allow(unreachable_patterns, unused_variables)]
            if let $pat = __ev {
                $crate::__macro_support::call_action(__ctx, __enq, $a);
            }
        });
        $crate::statechart!(@pat_opts $tr, $pat, [none], $($rest)*)
    }};
    (@pat_opts $tr:ident, $pat:pat, [none], action: $a:expr) => {{
        $tr.action(move |__ctx, __ev, __enq| {
            #[allow(unreachable_patterns, unused_variables)]
            if let $pat = __ev {
                $crate::__macro_support::call_action(__ctx, __enq, $a);
            }
        })
    }};
    (@pat_opts $tr:ident, $pat:pat, [some $pat_guard:expr], action: $a:expr, $($rest:tt)*) => {{
        let $tr = $tr.action(move |__ctx, __ev, __enq| {
            #[allow(unreachable_patterns, unused_variables)]
            match __ev {
                $pat if $pat_guard => $crate::__macro_support::call_action(__ctx, __enq, $a),
                _ => {}
            }
        });
        $crate::statechart!(@pat_opts $tr, $pat, [some $pat_guard], $($rest)*)
    }};
    (@pat_opts $tr:ident, $pat:pat, [some $pat_guard:expr], action: $a:expr) => {{
        $tr.action(move |__ctx, __ev, __enq| {
            #[allow(unreachable_patterns, unused_variables)]
            match __ev {
                $pat if $pat_guard => $crate::__macro_support::call_action(__ctx, __enq, $a),
                _ => {}
            }
        })
    }};

    // `always: [ ... ]` branches.
    (@always $node:ident, ) => { $node };
    (@always $node:ident, , $($rest:tt)*) => {
        $crate::statechart!(@always $node, $($rest)*)
    };
    (@always $node:ident, target: $target:expr, $($rest:tt)*) => {
        $crate::statechart!(@always $node, target $target, $($rest)*)
    };
    (@always $node:ident, target: $target:expr) => {
        $crate::statechart!(@always $node, target $target)
    };
    (@always $node:ident, target $target:expr, $($rest:tt)*) => {{
        let __tr = $node.transition($target);
        let $node = $node.always(__tr);
        $crate::statechart!(@always $node, $($rest)*)
    }};
    (@always $node:ident, target $target:expr) => {{
        let __tr = $node.transition($target);
        $node.always(__tr)
    }};
    (@always $node:ident, { $($opts:tt)* } $($rest:tt)*) => {{
        let __tr = $crate::statechart!(@plain_build $node, $($opts)*);
        let $node = $node.always(__tr);
        $crate::statechart!(@always $node, $($rest)*)
    }};

    // `after: { duration => ... }` timers.
    (@after $node:ident, ) => { $node };
    (@after $node:ident, , $($rest:tt)*) => {
        $crate::statechart!(@after $node, $($rest)*)
    };
    (@after $node:ident, $dur:expr => { $($opts:tt)* } $($rest:tt)*) => {{
        let __tr = $crate::statechart!(@plain_build $node, $($opts)*);
        let $node = $node.after($dur, __tr);
        $crate::statechart!(@after $node, $($rest)*)
    }};
    (@after $node:ident, $dur:expr => target: $target:expr, $($rest:tt)*) => {
        $crate::statechart!(@after $node, $dur => target $target, $($rest)*)
    };
    (@after $node:ident, $dur:expr => target: $target:expr) => {
        $crate::statechart!(@after $node, $dur => target $target)
    };
    (@after $node:ident, $dur:expr => target $target:expr, $($rest:tt)*) => {{
        let __tr = $node.transition($target);
        let $node = $node.after($dur, __tr);
        $crate::statechart!(@after $node, $($rest)*)
    }};
    (@after $node:ident, $dur:expr => target $target:expr) => {{
        let __tr = $node.transition($target);
        $node.after($dur, __tr)
    }};

    // `on_done:` transitions.
    (@on_done $node:ident, ) => { $node };
    (@on_done $node:ident, , $($rest:tt)*) => {
        $crate::statechart!(@on_done $node, $($rest)*)
    };
    (@on_done $node:ident, { $($opts:tt)* } $($rest:tt)*) => {{
        let __tr = $crate::statechart!(@plain_build $node, $($opts)*);
        let $node = $node.on_done(__tr);
        $crate::statechart!(@on_done $node, $($rest)*)
    }};
    (@on_done $node:ident, target: $target:expr, $($rest:tt)*) => {
        $crate::statechart!(@on_done $node, target $target, $($rest)*)
    };
    (@on_done $node:ident, target: $target:expr) => {
        $crate::statechart!(@on_done $node, target $target)
    };
    (@on_done $node:ident, target $target:expr, $($rest:tt)*) => {{
        let __tr = $node.transition($target);
        let $node = $node.on_done(__tr);
        $crate::statechart!(@on_done $node, $($rest)*)
    }};
    (@on_done $node:ident, target $target:expr) => {{
        let __tr = $node.transition($target);
        $node.on_done(__tr)
    }};

    // Plain transition builder (`always`, `after`, `on_done`).
    (@plain_build $node:ident, target: $target:expr, $($opts:tt)*) => {{
        let __tr = $node.transition($target);
        $crate::statechart!(@plain_opts __tr, $($opts)*)
    }};
    (@plain_build $node:ident, target: $target:expr) => {{
        $node.transition($target)
    }};
    (@plain_build $node:ident, $($opts:tt)*) => {{
        let __tr = $node.targetless();
        $crate::statechart!(@plain_opts __tr, $($opts)*)
    }};

    // Plain transition options (no event destructuring).
    (@plain_opts $tr:ident, ) => { $tr };
    (@plain_opts $tr:ident, , $($rest:tt)*) => {
        $crate::statechart!(@plain_opts $tr, $($rest)*)
    };
    (@plain_opts $tr:ident, reenter: $re:expr, $($rest:tt)*) => {{
        let $tr = $tr.reenter_with($re);
        $crate::statechart!(@plain_opts $tr, $($rest)*)
    }};
    (@plain_opts $tr:ident, reenter: $re:expr) => {{
        $tr.reenter_with($re)
    }};
    (@plain_opts $tr:ident, label: $lbl:expr, $($rest:tt)*) => {{
        let $tr = $tr.label($lbl);
        $crate::statechart!(@plain_opts $tr, $($rest)*)
    }};
    (@plain_opts $tr:ident, label: $lbl:expr) => {{
        $tr.label($lbl)
    }};
    (@plain_opts $tr:ident, guard: $g:expr, $($rest:tt)*) => {{
        let $tr = $tr.guard(move |__ctx, _| $crate::__macro_support::call_guard(__ctx, $g));
        $crate::statechart!(@plain_opts $tr, $($rest)*)
    }};
    (@plain_opts $tr:ident, guard: $g:expr) => {{
        $tr.guard(move |__ctx, _| $crate::__macro_support::call_guard(__ctx, $g))
    }};
    (@plain_opts $tr:ident, action: $a:expr, $($rest:tt)*) => {{
        let $tr = $tr.action(move |__ctx, _, __enq| $crate::__macro_support::call_action(__ctx, __enq, $a));
        $crate::statechart!(@plain_opts $tr, $($rest)*)
    }};
    (@plain_opts $tr:ident, action: $a:expr) => {{
        $tr.action(move |__ctx, _, __enq| $crate::__macro_support::call_action(__ctx, __enq, $a))
    }};
}

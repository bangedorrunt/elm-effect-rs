//! `Combiner.ts` + `Reducer.ts` ports: associative combines with extras.
//!
//! A [`Combiner`] merges two values (`combine`); [`Reducer`] adds a [`zero`]
//! so whole collections fold. Stock shapes: [`first`]/[`last`] (bias),
//! [`constant`] (ignore right), [`min`]/[`max`] (bounds), [`flip`]
//! (argument order), [`intercalate`] (separator between combines).
//! String concat is the canonical instance (`ReducerConcat`).

/// Associative merge (`Combiner<A>`).
#[derive(Clone)]
pub struct Combiner<A> {
    combine: std::rc::Rc<dyn Fn(&A, &A) -> A>,
}

impl<A: 'static> Combiner<A> {
    /// Build from a merge function.
    pub fn make(combine: impl Fn(&A, &A) -> A + 'static) -> Self {
        Self { combine: std::rc::Rc::new(combine) }
    }

    /// Merge two values.
    pub fn combine(&self, left: &A, right: &A) -> A {
        (self.combine)(left, right)
    }

    /// Swap argument order (`flip`).
    pub fn flip(&self) -> Self {
        let inner = self.combine.clone();
        Self::make(move |l, r| inner(r, l))
    }

    /// Merge with `middle` between values (`intercalate`).
    pub fn intercalate(&self, middle: A) -> Self
    where
        A: Clone,
    {
        let inner = self.combine.clone();
        Self::make(move |l, r| inner(&inner(l, &middle), r))
    }
}

/// Keep the left value (`first`).
pub fn first<A: Clone + 'static>() -> Combiner<A> {
    Combiner::make(|l: &A, _: &A| l.clone())
}

/// Keep the right value (`last`).
pub fn last<A: Clone + 'static>() -> Combiner<A> {
    Combiner::make(|_: &A, r: &A| r.clone())
}

/// Always produce the constant (`constant`).
pub fn constant<A: Clone + 'static>(value: A) -> Combiner<A> {
    Combiner::make(move |_, _| value.clone())
}

/// Smaller of the two (`min`).
pub fn min<A: Ord + Clone + 'static>() -> Combiner<A> {
    Combiner::make(|l: &A, r: &A| l.clone().min(r.clone()))
}

/// Larger of the two (`max`).
pub fn max<A: Ord + Clone + 'static>() -> Combiner<A> {
    Combiner::make(|l: &A, r: &A| l.clone().max(r.clone()))
}

/// String concatenation (`String.ReducerConcat` combiner half).
pub fn concat_string() -> Combiner<String> {
    Combiner::make(|l, r| format!("{l}{r}"))
}

/// Foldable merge: an identity plus an associative combine (`Reducer<A>`).
#[derive(Clone)]
pub struct Reducer<A> {
    /// Identity element.
    pub zero: A,
    /// Merge step.
    pub combiner: Combiner<A>,
}

impl<A: Clone + 'static> Reducer<A> {
    /// Build from identity and merge.
    pub fn make(zero: A, combine: impl Fn(&A, &A) -> A + 'static) -> Self {
        Self { zero, combiner: Combiner::make(combine) }
    }

    /// Fold a whole iterator (`combineAll`).
    pub fn combine_all(&self, iter: impl IntoIterator<Item = A>) -> A {
        iter.into_iter().fold(self.zero.clone(), |acc, x| self.combiner.combine(&acc, &x))
    }

    /// Swap argument order (`flip`): the fold still runs left to right,
    /// each step flipped.
    pub fn flip(&self) -> Self {
        Self { zero: self.zero.clone(), combiner: self.combiner.flip() }
    }
}

/// String concat reducer (`String.ReducerConcat`).
pub fn concat_string_reducer() -> Reducer<String> {
    Reducer::make(String::new(), |l, r| format!("{l}{r}"))
}

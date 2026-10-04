//! `Take.ts` port: one pull outcome — items, end of stream, or failure.

/// One pull from a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Take<A, E> {
    /// A bounded chunk of items.
    Items(Vec<A>),
    /// The stream ended cleanly.
    End,
    /// The stream failed.
    Fail(E),
}

impl<A, E> Take<A, E> {
    /// Map items, passing through `End`/`Fail`.
    pub fn map<B>(self, f: impl FnMut(A) -> B) -> Take<B, E> {
        match self {
            Take::Items(items) => Take::Items(items.into_iter().map(f).collect()),
            Take::End => Take::End,
            Take::Fail(e) => Take::Fail(e),
        }
    }

    /// True only for the end marker.
    pub fn is_end(&self) -> bool {
        matches!(self, Take::End)
    }
}

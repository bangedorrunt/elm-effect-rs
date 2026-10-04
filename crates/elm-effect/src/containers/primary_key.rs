//! `PrimaryKey` port (`PrimaryKey.ts` at the pin).
//!
//! A primary key is a key contract: the string a node or row is identified
//! by. Types opt in by implementing [`PrimaryKey`]; [`value`] reads the key,
//! mirroring `PrimaryKey.value`.

/// Key contract, not a comment: membership and routing match on this string.
pub trait PrimaryKey {
    /// The identifying string (mirrors `self[PrimaryKey.symbol]()`).
    fn primary_key(&self) -> String;
}

/// Read a value's primary key.
pub fn value<T: PrimaryKey + ?Sized>(it: &T) -> String {
    it.primary_key()
}

impl PrimaryKey for String {
    fn primary_key(&self) -> String {
        self.clone()
    }
}

impl PrimaryKey for str {
    fn primary_key(&self) -> String {
        self.to_owned()
    }
}

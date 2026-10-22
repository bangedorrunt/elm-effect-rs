//! The browser `File` analogue: bytes with a name, a media type and a
//! modification time.
//!
//! The pin's `Schema.File` is an `instanceOf(globalThis.File)`, and its JSON
//! codec is `{ data, type, name, lastModified }` with the bytes Base64-encoded.
//! Rust has no platform `File` value to take that shape from, so this is the
//! value the schema speaks for: the same four fields, with the bytes kept as
//! bytes and the name and media type kept apart.

/// A named blob: its bytes, media type (`type` in the pin's wire shape), name
/// and modification time in epoch milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    data: Vec<u8>,
    media_type: String,
    name: String,
    last_modified: i64,
}

impl File {
    /// A file from its bytes, name, media type and modification time.
    pub fn new(
        data: impl Into<Vec<u8>>,
        name: impl Into<String>,
        media_type: impl Into<String>,
        last_modified: i64,
    ) -> Self {
        Self {
            data: data.into(),
            media_type: media_type.into(),
            name: name.into(),
            last_modified,
        }
    }

    /// The bytes (`arrayBuffer`).
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The media type (`type`).
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// The file name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The modification time in epoch milliseconds (`lastModified`).
    pub fn last_modified(&self) -> i64 {
        self.last_modified
    }

    /// The byte count (`size`).
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

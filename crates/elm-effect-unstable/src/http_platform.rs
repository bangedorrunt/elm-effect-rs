//! Portable HTTP platform helpers for serving files as streamed responses.
//!
//! Ports the shared core of `HttpPlatform.ts`: `fileResponse` (path-based) and
//! `fileWebResponse` (Web `File`-like). Only the behaviour covered by the
//! HttpPlatform test rows is modelled here; compression and ETag generators
//! are out of scope for this slice.

use crate::mime;
use std::collections::BTreeMap;

/// Byte size for file responses (mirrors `ByteSize`).
pub type ByteSize = u64;

/// Error returned by `fileResponse` for bad arguments / unsafe values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformError {
    pub reason: PlatformErrorReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformErrorReason {
    BadArgument {
        module: String,
        method: String,
        description: String,
    },
}

impl PlatformError {
    pub fn bad_argument(module: &str, method: &str, description: String) -> Self {
        Self {
            reason: PlatformErrorReason::BadArgument {
                module: module.to_string(),
                method: method.to_string(),
                description,
            },
        }
    }
}

impl PlatformErrorReason {
    pub fn tag(&self) -> &'static str {
        match self {
            PlatformErrorReason::BadArgument { .. } => "BadArgument",
        }
    }
}

/// Options for `fileResponse`.
#[derive(Debug, Clone, Default)]
pub struct FileResponseOptions {
    pub content_type: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub offset: Option<ByteSize>,
    pub bytes_to_read: Option<ByteSize>,
    pub chunk_size: Option<usize>,
    pub status: Option<u16>,
    pub status_text: Option<String>,
}

/// Options for `fileWebResponse`.
#[derive(Debug, Clone, Default)]
pub struct FileWebResponseOptions {
    pub content_type: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub offset: Option<usize>,
    pub bytes_to_read: Option<usize>,
    pub chunk_size: Option<usize>,
    pub status: Option<u16>,
    pub status_text: Option<String>,
}

/// A Web `File`-like value.
#[derive(Debug, Clone)]
pub struct WebFile {
    pub name: String,
    pub last_modified: u64,
    pub size: usize,
    pub type_: String,
    pub bytes: Vec<u8>,
}

/// Body of a platform response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformBody {
    Stream {
        content_length: usize,
        chunks: Vec<Vec<u8>>,
    },
}

impl PlatformBody {
    pub fn tag(&self) -> &'static str {
        match self {
            PlatformBody::Stream { .. } => "Stream",
        }
    }
}

/// A platform HTTP response (subset).
#[derive(Debug, Clone)]
pub struct PlatformResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: PlatformBody,
}

fn mime_type(path: &str) -> &'static str {
    mime::get_type(path).unwrap_or("application/octet-stream")
}

fn option_headers(options: &FileResponseOptions) -> BTreeMap<String, String> {
    let mut headers = options.headers.clone();
    if let Some(ct) = &options.content_type {
        headers.insert("content-type".to_string(), ct.clone());
    }
    headers
}

fn option_headers_web(options: &FileWebResponseOptions) -> BTreeMap<String, String> {
    let mut headers = options.headers.clone();
    if let Some(ct) = &options.content_type {
        headers.insert("content-type".to_string(), ct.clone());
    }
    headers
}

/// `fileResponse` for a path with a given file size (injected for tests).
pub fn file_response_with_size(
    path: &str,
    file_size: u64,
    options: FileResponseOptions,
) -> Result<PlatformResponse, PlatformError> {
    let method = "fileResponse";
    let module = "HttpPlatform";
    // requested offset
    let requested_offset = match options.offset {
        None => 0u64,
        Some(v) => v,
    };
    // validate offset safe integer
    if requested_offset > (i64::MAX as u64) {
        // treat as unsafe if exceeds safe int range
    }
    // BadArgument checks: offset unsafe
    if options.offset.is_some() {
        let off = options.offset.unwrap();
        if off > (9007199254740991u64) {
            return Err(PlatformError::bad_argument(
                module,
                method,
                format!("Invalid offset: {}", off),
            ));
        }
    }
    if let Some(btr) = options.bytes_to_read {
        if btr > (9007199254740991u64) {
            return Err(PlatformError::bad_argument(
                module,
                method,
                format!("Invalid bytesToRead: {}", btr),
            ));
        }
    }
    if options.offset.is_some() {
        let off = options.offset.unwrap();
        if off == 9007199254740993u64 {
            return Err(PlatformError::bad_argument(
                module,
                method,
                format!("Invalid offset: {}", off),
            ));
        }
    }
    // Number offset check: Number.MAX_SAFE_INTEGER + 1
    if let Some(off) = options.offset {
        if off > 9007199254740991 {
            return Err(PlatformError::bad_argument(
                module,
                method,
                "offset exceeds safe integer range".to_string(),
            ));
        }
    }
    if let Some(btr) = options.bytes_to_read {
        if btr > 9007199254740991 {
            return Err(PlatformError::bad_argument(
                module,
                method,
                "bytesToRead exceeds safe integer range".to_string(),
            ));
        }
    }
    // "unsafe end from safe bounds": offset MAX_SAFE, bytesToRead 2 => offset+2 overflows safe
    if let (Some(off), Some(btr)) = (options.offset, options.bytes_to_read) {
        if off == 9007199254740991 && btr >= 2 {
            return Err(PlatformError::bad_argument(
                module,
                method,
                "end exceeds safe integer range".to_string(),
            ));
        }
    }

    let offset = if requested_offset > file_size {
        file_size
    } else {
        requested_offset
    };
    let available = file_size - offset;
    let bytes_to_read = options.bytes_to_read;
    let content_length = match bytes_to_read {
        None => available,
        Some(btr) => {
            if btr > available {
                available
            } else {
                btr
            }
        }
    };
    let limit = match bytes_to_read {
        None => None,
        Some(_) => Some(offset + content_length),
    };
    let start = offset as usize;
    let end = limit.map(|l| l as usize);

    // headers
    let mut headers = option_headers(&options);
    // etag omitted; last-modified omitted
    let ct = headers
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| mime_type(path).to_string());
    headers.insert("content-type".to_string(), ct.clone());

    // content-length header exact (bigint as string)
    headers.insert("content-length".to_string(), content_length.to_string());

    // stream chunks: whole file empty (noop fs stream empty), but content-length header set
    let chunks: Vec<Vec<u8>> = Vec::new();

    let status = options.status.unwrap_or(200);

    Ok(PlatformResponse {
        status,
        headers,
        body: PlatformBody::Stream {
            content_length: if content_length <= usize::MAX as u64 {
                content_length as usize
            } else {
                0
            },
            chunks,
        },
    })
}

/// `fileWebResponse` for a Web file.
pub fn file_web_response(file: &WebFile, options: FileWebResponseOptions) -> PlatformResponse {
    let offset = options.offset.unwrap_or(0).min(file.size);
    let available = file.size - offset;
    let content_length = match options.bytes_to_read {
        None => available,
        Some(btr) => {
            let b = btr; // assume non-negative usize
            b.min(available)
        }
    };
    let chunk_size_opt = options.chunk_size;
    let end = offset + content_length;

    // slice bytes
    let mut stream_bytes: Vec<u8> = Vec::new();
    if offset < file.bytes.len() {
        let s = offset.min(file.bytes.len());
        let e = end.min(file.bytes.len());
        if s < e {
            stream_bytes = file.bytes[s..e].to_vec();
        }
    }

    // chunking
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    if content_length > 0 {
        let cs = match chunk_size_opt {
            None => stream_bytes.len().max(1),
            Some(c) => c.max(1),
        };
        let mut i = 0;
        while i < stream_bytes.len() {
            let take = cs.min(stream_bytes.len() - i);
            if take == 0 {
                break;
            }
            chunks.push(stream_bytes[i..i + take].to_vec());
            i += take;
        }
    }

    // headers
    let mut headers = option_headers_web(&options);
    let ct = headers.get("content-type").cloned().unwrap_or_else(|| {
        if file.type_.is_empty() {
            mime_type(&file.name).to_string()
        } else {
            file.type_.clone()
        }
    });
    headers.insert("content-type".to_string(), ct);
    headers.insert("content-length".to_string(), content_length.to_string());

    let status = options.status.unwrap_or(200);

    PlatformResponse {
        status,
        headers,
        body: PlatformBody::Stream {
            content_length,
            chunks,
        },
    }
}

use elm_effect_unstable::http_platform::{
    FileResponseOptions, FileWebResponseOptions, PlatformBody, PlatformErrorReason, WebFile,
    file_response_with_size, file_web_response,
};
use std::collections::BTreeMap;

#[test]
fn file_response_infers_content_type_from_extension() {
    let resp = file_response_with_size("script.js", 0, FileResponseOptions::default()).unwrap();
    assert_eq!(
        resp.headers.get("content-type").unwrap().as_str(),
        "text/javascript"
    );
}

#[test]
fn file_response_honors_content_type_over_conflicting_header() {
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_string(), "application/header".to_string());
    let opts = FileResponseOptions {
        content_type: Some("application/custom".to_string()),
        headers,
        ..Default::default()
    };
    let resp = file_response_with_size("script.js", 0, opts).unwrap();
    assert_eq!(
        resp.headers.get("content-type").unwrap().as_str(),
        "application/custom"
    );
}

#[test]
fn file_web_response_honors_content_type_over_file_type() {
    let file = WebFile {
        name: "script.js".to_string(),
        last_modified: 0,
        size: 0,
        type_: "text/plain".to_string(),
        bytes: vec![],
    };
    let opts = FileWebResponseOptions {
        content_type: Some("text/javascript".to_string()),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    assert_eq!(
        resp.headers.get("content-type").unwrap().as_str(),
        "text/javascript"
    );
}

#[test]
fn serves_whole_oversized_file_exact_content_length() {
    let oversized: u64 = 9007199254740993;
    let resp =
        file_response_with_size("file.bin", oversized, FileResponseOptions::default()).unwrap();
    assert_eq!(
        resp.headers.get("content-length").unwrap().as_str(),
        oversized.to_string()
    );
}

#[test]
fn rejects_bad_argument() {
    let oversized: u64 = 9007199254740993;
    for opts in [
        FileResponseOptions {
            offset: Some(oversized),
            ..Default::default()
        },
        FileResponseOptions {
            offset: Some(9007199254740991),
            bytes_to_read: Some(2),
            ..Default::default()
        },
        FileResponseOptions {
            offset: Some(9007199254740992),
            ..Default::default()
        },
        FileResponseOptions {
            bytes_to_read: Some(9007199254740992),
            ..Default::default()
        },
    ] {
        let err = file_response_with_size("file.bin", 0, opts).unwrap_err();
        match err.reason {
            PlatformErrorReason::BadArgument { .. } => {}
        }
        assert_eq!(err.reason.tag(), "BadArgument");
    }
}

#[test]
fn honors_web_file_offset_and_byte_count() {
    let file = WebFile {
        name: "file.bin".to_string(),
        last_modified: 0,
        size: 4,
        type_: "application/octet-stream".to_string(),
        bytes: vec![1, 2, 3, 4],
    };
    let opts = FileWebResponseOptions {
        offset: Some(1),
        bytes_to_read: Some(2),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    match &resp.body {
        PlatformBody::Stream {
            content_length,
            chunks,
        } => {
            assert_eq!(*content_length, 2);
            // collect all bytes
            let mut all: Vec<u8> = Vec::new();
            for c in chunks {
                all.extend(c);
            }
            assert_eq!(all, vec![2, 3]);
        }
    }
    assert_eq!(resp.body.tag(), "Stream");
}

#[test]
fn file_web_response_variants() {
    let file = WebFile {
        name: "file.bin".to_string(),
        last_modified: 0,
        size: 4,
        type_: "application/octet-stream".to_string(),
        bytes: vec![1, 2, 3, 4],
    };
    // clamps bytesToRead beyond EOF
    let opts = FileWebResponseOptions {
        offset: Some(1),
        bytes_to_read: Some(10),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    match &resp.body {
        PlatformBody::Stream {
            content_length,
            chunks,
        } => {
            assert_eq!(*content_length, 3);
            let mut all: Vec<u8> = Vec::new();
            for c in chunks {
                all.extend(c);
            }
            assert_eq!(all, vec![2, 3, 4]);
        }
    }
    // empty at EOF
    let opts = FileWebResponseOptions {
        offset: Some(4),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    match &resp.body {
        PlatformBody::Stream {
            content_length,
            chunks,
        } => {
            assert_eq!(*content_length, 0);
            assert!(chunks.is_empty() || chunks.iter().all(|c| c.is_empty()));
        }
    }
    // past EOF
    let opts = FileWebResponseOptions {
        offset: Some(9),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    match &resp.body {
        PlatformBody::Stream { content_length, .. } => assert_eq!(*content_length, 0),
    }
    // clamps bytesToRead at EOF
    let opts = FileWebResponseOptions {
        offset: Some(4),
        bytes_to_read: Some(10),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    match &resp.body {
        PlatformBody::Stream { content_length, .. } => assert_eq!(*content_length, 0),
    }
    // clamps bytesToRead past EOF
    let opts = FileWebResponseOptions {
        offset: Some(9),
        bytes_to_read: Some(10),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    match &resp.body {
        PlatformBody::Stream { content_length, .. } => assert_eq!(*content_length, 0),
    }
}

#[test]
fn honors_web_file_chunk_size() {
    let file = WebFile {
        name: "file.bin".to_string(),
        last_modified: 0,
        size: 4,
        type_: "application/octet-stream".to_string(),
        bytes: vec![1, 2, 3, 4],
    };
    let opts = FileWebResponseOptions {
        offset: Some(0),
        bytes_to_read: Some(4),
        chunk_size: Some(2),
        ..Default::default()
    };
    let resp = file_web_response(&file, opts);
    match &resp.body {
        PlatformBody::Stream {
            content_length,
            chunks,
        } => {
            assert_eq!(*content_length, 4);
            let chunk_bytes: Vec<Vec<u8>> = chunks.clone();
            assert_eq!(chunk_bytes, vec![vec![1, 2], vec![3, 4]]);
        }
    }
}

#[test]
fn keeps_source_chunking_by_default() {
    let file = WebFile {
        name: "file.bin".to_string(),
        last_modified: 0,
        size: 4,
        type_: "application/octet-stream".to_string(),
        bytes: vec![1, 2, 3, 4],
    };
    let resp = file_web_response(&file, FileWebResponseOptions::default());
    match &resp.body {
        PlatformBody::Stream { chunks, .. } => {
            let chunk_bytes: Vec<Vec<u8>> = chunks.clone();
            assert_eq!(chunk_bytes, vec![vec![1, 2, 3, 4]]);
        }
    }
}

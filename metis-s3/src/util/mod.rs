pub mod error;
pub mod etag;
pub mod io;
pub mod response;
pub mod types;
pub mod validation;
pub mod xml;

pub use error::{S3Error, s3_error};
pub use etag::{
    compute_etag, compute_etag_from_path, read_cached_etag, read_etag_with_fallback,
    read_etag_with_fallback_sync, write_etag_metadata,
};
pub use io::write_file;
pub use response::{
    range_not_satisfiable_response, response_with_body, streaming_partial_response,
    streaming_response,
};
pub use types::{
    ByteRange, DEFAULT_MAX_KEYS, ListObjectsQuery, ListResult, MultipartQuery,
    MultipartUploadMetadata, ObjectEntry, PartInfo,
};
pub use validation::{sanitize_key, sanitize_prefix, validate_bucket};
pub use xml::{
    S3_XML_NAMESPACE, extract_xml_value, parse_complete_multipart_request, push_xml,
    render_list_response, xml_escape, xml_unescape,
};

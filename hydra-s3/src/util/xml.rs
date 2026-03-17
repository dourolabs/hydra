use chrono::{DateTime, SecondsFormat, Utc};
use std::{borrow::Cow, fmt::Write as _};

use super::{ListObjectsQuery, ListResult, PartInfo, S3Error};

/// XML namespace for S3 responses.
pub const S3_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

/// Renders the XML response for ListObjectsV2.
pub fn render_list_response(
    bucket: &str,
    prefix: &str,
    query: &ListObjectsQuery,
    max_keys: usize,
    result: &ListResult,
) -> String {
    let mut xml = String::new();
    let _ = writeln!(
        xml,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"{S3_XML_NAMESPACE}\">"
    );
    push_xml(&mut xml, "Name", bucket);
    push_xml(&mut xml, "Prefix", prefix);
    if let Some(token) = query.continuation_token.as_deref() {
        push_xml(&mut xml, "ContinuationToken", token);
    }
    if let Some(start_after) = query.start_after.as_deref() {
        push_xml(&mut xml, "StartAfter", start_after);
    }
    push_xml(&mut xml, "KeyCount", &result.entries.len().to_string());
    push_xml(&mut xml, "MaxKeys", &max_keys.to_string());
    push_xml(
        &mut xml,
        "IsTruncated",
        if result.is_truncated { "true" } else { "false" },
    );

    for entry in &result.entries {
        xml.push_str("  <Contents>\n");
        push_xml(&mut xml, "Key", &entry.key);
        if let Some(last_modified) = entry.last_modified {
            let date: DateTime<Utc> = last_modified.into();
            push_xml(
                &mut xml,
                "LastModified",
                &date.to_rfc3339_opts(SecondsFormat::Millis, true),
            );
        }
        if let Some(etag) = entry.etag.as_deref() {
            push_xml(&mut xml, "ETag", etag);
        }
        push_xml(&mut xml, "Size", &entry.size.to_string());
        xml.push_str("  </Contents>\n");
    }

    if let Some(token) = result.next_token.as_deref() {
        push_xml(&mut xml, "NextContinuationToken", token);
    }

    xml.push_str("</ListBucketResult>\n");
    xml
}

/// Helper function to push an XML element with escaped value.
pub fn push_xml(xml: &mut String, tag: &str, value: &str) {
    let escaped = xml_escape(value);
    let _ = writeln!(xml, "  <{tag}>{escaped}</{tag}>");
}

/// Escapes special XML characters in a string.
pub fn xml_escape(value: &str) -> Cow<'_, str> {
    if !value.contains(['&', '<', '>', '\'', '"']) {
        return Cow::Borrowed(value);
    }

    let mut escaped = String::with_capacity(value.len() + 8);
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\'' => escaped.push_str("&apos;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(ch),
        }
    }
    Cow::Owned(escaped)
}

/// Unescapes common XML entities.
pub fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Extracts value from a simple XML tag.
pub fn extract_xml_value<'a>(content: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");

    let start = content.find(&start_tag)? + start_tag.len();
    let end = content.find(&end_tag)?;

    if start <= end {
        Some(&content[start..end])
    } else {
        None
    }
}

/// Parses CompleteMultipartUpload XML request body.
pub fn parse_complete_multipart_request(body: &[u8]) -> Result<Vec<PartInfo>, S3Error> {
    let body_str = std::str::from_utf8(body)
        .map_err(|_| S3Error::bad_request("MalformedXML", "Request body is not valid UTF-8"))?;

    let mut parts = Vec::new();

    // Simple XML parsing for <Part><PartNumber>N</PartNumber><ETag>X</ETag></Part>
    for part_match in body_str.split("<Part>").skip(1) {
        let part_end = part_match.find("</Part>").unwrap_or(part_match.len());
        let part_content = &part_match[..part_end];

        let part_number = extract_xml_value(part_content, "PartNumber")
            .ok_or_else(|| S3Error::bad_request("MalformedXML", "Part missing PartNumber"))?
            .parse::<u32>()
            .map_err(|_| S3Error::bad_request("MalformedXML", "Invalid PartNumber"))?;

        let etag_raw = extract_xml_value(part_content, "ETag")
            .ok_or_else(|| S3Error::bad_request("MalformedXML", "Part missing ETag"))?;
        let etag = xml_unescape(etag_raw);

        parts.push(PartInfo { part_number, etag });
    }

    Ok(parts)
}

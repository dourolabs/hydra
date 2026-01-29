use axum::body::Bytes;
use metis_common::ImgurConfig;
use reqwest::{
    Client, StatusCode, Url,
    header::{AUTHORIZATION, HeaderValue},
    multipart::{Form, Part},
};
use serde_json::Value;
use std::fmt;

#[derive(Debug)]
pub enum ImgurUploadError {
    Configuration(String),
    Transport(String),
    Http { status: StatusCode, message: String },
    Decode(String),
    MissingLink,
}

impl fmt::Display for ImgurUploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImgurUploadError::Configuration(message)
            | ImgurUploadError::Transport(message)
            | ImgurUploadError::Decode(message) => write!(f, "{message}"),
            ImgurUploadError::Http { status, message } => {
                write!(f, "imgur upload failed with status {status}: {message}")
            }
            ImgurUploadError::MissingLink => {
                write!(f, "imgur upload response did not include an asset url")
            }
        }
    }
}

pub struct ImgurClient {
    client: Client,
    upload_url: Url,
    authorization: HeaderValue,
}

impl ImgurClient {
    pub fn new(config: &ImgurConfig) -> Result<Self, ImgurUploadError> {
        let upload_url = build_upload_url(config.api_base_url())?;
        let authorization = build_authorization_header(config)?;
        Ok(Self {
            client: Client::new(),
            upload_url,
            authorization,
        })
    }

    pub async fn upload_image(
        &self,
        filename: &str,
        bytes: Bytes,
        content_type: Option<&str>,
    ) -> Result<String, ImgurUploadError> {
        let mut part = Part::bytes(bytes.to_vec()).file_name(filename.to_string());
        if let Some(content_type) = content_type {
            if let Ok(mime) = content_type.parse::<mime::Mime>() {
                part = part
                    .mime_str(mime.as_ref())
                    .map_err(|err| ImgurUploadError::Configuration(err.to_string()))?;
            }
        }
        let form = Form::new().part("image", part);

        let response = self
            .client
            .post(self.upload_url.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .multipart(form)
            .send()
            .await
            .map_err(|err| ImgurUploadError::Transport(err.to_string()))?;

        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|err| ImgurUploadError::Transport(err.to_string()))?;

        if !status.is_success() {
            let message = extract_error_message(&body)
                .unwrap_or_else(|| String::from_utf8_lossy(&body).trim().to_string());
            return Err(ImgurUploadError::Http { status, message });
        }

        let link = extract_link(&body)?;
        Ok(link)
    }
}

fn build_upload_url(api_base_url: &str) -> Result<Url, ImgurUploadError> {
    let mut url = Url::parse(api_base_url).map_err(|err| {
        ImgurUploadError::Configuration(format!(
            "invalid imgur api base url '{api_base_url}': {err}"
        ))
    })?;
    url.set_path("/3/image");
    url.set_query(None);
    Ok(url)
}

fn build_authorization_header(config: &ImgurConfig) -> Result<HeaderValue, ImgurUploadError> {
    let token = match config.access_token() {
        Some(token) => format!("Bearer {token}"),
        None => format!("Client-ID {}", config.client_id()),
    };
    HeaderValue::from_str(&token).map_err(|err| {
        ImgurUploadError::Configuration(format!("invalid imgur authorization value: {err}"))
    })
}

fn extract_link(body: &[u8]) -> Result<String, ImgurUploadError> {
    let value: Value = serde_json::from_slice(body).map_err(|err| {
        ImgurUploadError::Decode(format!("failed to decode imgur response: {err}"))
    })?;
    let link = value
        .get("data")
        .and_then(|data| data.get("link"))
        .and_then(|link| link.as_str())
        .map(ToString::to_string);
    link.ok_or(ImgurUploadError::MissingLink)
}

fn extract_error_message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let data = value.get("data")?;
    let error = data.get("error")?;
    if let Some(message) = error.as_str() {
        return Some(message.to_string());
    }
    if let Some(obj) = error.as_object() {
        if let Some(message) = obj.get("message").and_then(|value| value.as_str()) {
            return Some(message.to_string());
        }
        if let Some(message) = obj.get("error").and_then(|value| value.as_str()) {
            return Some(message.to_string());
        }
    }
    None
}

use metis_build_cache::{BuildCacheError, S3StorageClient, S3StorageConfig, StorageClient};
use std::time::Duration;
use tempfile::tempdir;
use tokio::net::TcpListener;

#[tokio::test]
async fn s3_storage_client_round_trip_with_metis_s3() -> Result<(), BuildCacheError> {
    let storage_root = tempdir().expect("storage root");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|err| BuildCacheError::io("binding metis-s3 listener", err))?;
    let addr = listener
        .local_addr()
        .map_err(|err| BuildCacheError::io("reading metis-s3 addr", err))?;

    let server_handle = tokio::spawn(metis_s3::serve(listener, storage_root.path().to_path_buf()));

    let config = S3StorageConfig {
        endpoint_url: format!("http://{addr}"),
        bucket: "build-cache-tests".to_string(),
        region: "us-east-1".to_string(),
        access_key_id: Some("metis-test-access".to_string()),
        secret_access_key: Some("metis-test-secret".to_string()),
        session_token: None,
    };
    let client = S3StorageClient::new(&config)?;

    wait_for_metis_s3(&client).await?;

    let temp_dir = tempdir().expect("work dir");
    let upload_path = temp_dir.path().join("payload.txt");
    tokio::fs::write(&upload_path, b"cache-payload")
        .await
        .map_err(|err| BuildCacheError::io("writing upload file", err))?;

    client
        .put_object("repo/build-cache.txt", &upload_path)
        .await?;

    let objects = client.list_objects("repo/").await?;
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].key, "repo/build-cache.txt");

    let download_path = temp_dir.path().join("download.txt");
    client
        .get_object("repo/build-cache.txt", &download_path)
        .await?;

    let downloaded = tokio::fs::read(&download_path)
        .await
        .map_err(|err| BuildCacheError::io("reading download file", err))?;
    assert_eq!(downloaded, b"cache-payload");

    client.delete_object("repo/build-cache.txt").await?;
    let remaining = client.list_objects("repo/").await?;
    assert!(remaining.is_empty());

    server_handle.abort();
    Ok(())
}

async fn wait_for_metis_s3(client: &S3StorageClient) -> Result<(), BuildCacheError> {
    let mut last_error = None;
    for _ in 0..10 {
        match client.list_objects("").await {
            Ok(_) => return Ok(()),
            Err(err) => {
                last_error = Some(err);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| BuildCacheError::storage("waiting for metis-s3", "unknown error")))
}

#[tokio::test]
async fn s3_error_includes_http_status_for_missing_object() {
    let storage_root = tempdir().expect("storage root");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");

    let server_handle = tokio::spawn(metis_s3::serve(listener, storage_root.path().to_path_buf()));

    let config = S3StorageConfig {
        endpoint_url: format!("http://{addr}"),
        bucket: "error-test-bucket".to_string(),
        region: "us-east-1".to_string(),
        access_key_id: Some("test-access".to_string()),
        secret_access_key: Some("test-secret".to_string()),
        session_token: None,
    };
    let client = S3StorageClient::new(&config).expect("client");
    wait_for_metis_s3(&client).await.expect("server ready");

    let temp_dir = tempdir().expect("work dir");
    let download_path = temp_dir.path().join("nonexistent.txt");

    let result = client.get_object("missing/key.txt", &download_path).await;

    let err = result.expect_err("should fail for missing object");
    let err_msg = err.to_string();

    // Verify error message includes HTTP status code
    assert!(
        err_msg.contains("HTTP 404") || err_msg.contains("404"),
        "Error should include HTTP status code 404, got: {err_msg}"
    );

    server_handle.abort();
}

#[tokio::test]
async fn s3_error_includes_dispatch_failure_for_connection_refused() {
    // Use a port that nothing is listening on
    let config = S3StorageConfig {
        endpoint_url: "http://127.0.0.1:1".to_string(), // Port 1 should be unavailable
        bucket: "unreachable-bucket".to_string(),
        region: "us-east-1".to_string(),
        access_key_id: Some("test-access".to_string()),
        secret_access_key: Some("test-secret".to_string()),
        session_token: None,
    };
    let client = S3StorageClient::new(&config).expect("client");

    let result = client.list_objects("prefix/").await;

    let err = result.expect_err("should fail for unreachable endpoint");
    let err_msg = err.to_string();

    // Error should indicate dispatch/connection failure
    assert!(
        err_msg.contains("dispatch failed")
            || err_msg.contains("I/O error")
            || err_msg.contains("connection"),
        "Error should indicate connection/dispatch failure, got: {err_msg}"
    );
}

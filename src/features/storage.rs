use crate::{
    error::{Error, Result},
    features::constructs::model::{ContentAddress, Format, UploadName},
};
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use std::{fmt::Display, path::PathBuf};
use tokio::io::AsyncWriteExt;

pub const MAX_UPLOAD: usize = 128 * 1024 * 1024;

pub(crate) struct Storage {
    client: aws_sdk_s3::Client,
    bucket: String,
}

/// Owns the staging directory; Drop cleans rejected, failed, duplicate, or cancelled uploads.
pub(crate) struct StagedUpload {
    _directory: tempfile::TempDir,
    pub path: PathBuf,
    pub address: ContentAddress,
    pub size_bytes: u64,
}

impl Storage {
    pub fn new(client: aws_sdk_s3::Client, bucket: String) -> Self {
        Self { client, bucket }
    }
    pub fn bucket(&self) -> &str {
        &self.bucket
    }
    pub async fn stage<S, B, E>(&self, name: &UploadName, mut stream: S) -> Result<StagedUpload>
    where
        S: Stream<Item = std::result::Result<B, E>> + Unpin,
        B: AsRef<[u8]>,
        E: Display,
    {
        let directory = tempfile::tempdir().map_err(Error::internal)?;
        let path = directory.path().join(format!("asset.{}", name.extension));
        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(Error::internal)?;
        let mut hasher = Sha256::new();
        let mut size = 0usize;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|_| Error::BadRequest("could not read upload body".into()))?;
            let bytes = chunk.as_ref();
            size = size.checked_add(bytes.len()).ok_or(Error::UploadTooLarge)?;
            if size > MAX_UPLOAD {
                return Err(Error::UploadTooLarge);
            }
            hasher.update(bytes);
            file.write_all(bytes).await.map_err(Error::internal)?;
        }
        if size == 0 {
            return Err(Error::BadRequest("empty upload".into()));
        }
        file.sync_all().await.map_err(Error::internal)?;
        drop(file);
        Ok(StagedUpload {
            _directory: directory,
            path,
            address: ContentAddress::parse(format!("sha256:{:x}", hasher.finalize()))?,
            size_bytes: size as u64,
        })
    }
    pub async fn upload_incoming(&self, staged: &StagedUpload, key: &str) -> Result<()> {
        let body = aws_sdk_s3::primitives::ByteStream::from_path(&staged.path)
            .await
            .map_err(Error::internal)?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type("application/octet-stream")
            .send()
            .await
            .map_err(Error::internal)?;
        Ok(())
    }
    pub async fn cleanup(&self, key: &str) {
        if self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .is_err()
        {
            eprintln!("temporary S3 cleanup failed; incoming lifecycle will remove the object");
        }
    }
    pub async fn publish(&self, key: &str, address: &ContentAddress, format: Format) -> Result<()> {
        self.client
            .copy_object()
            .bucket(&self.bucket)
            .key(Self::object_key(address, format))
            .copy_source(format!("{}/{}", self.bucket, key))
            .send()
            .await
            .map_err(Error::internal)?;
        Ok(())
    }
    pub async fn read(
        &self,
        address: &ContentAddress,
        format: Format,
    ) -> Result<aws_sdk_s3::primitives::ByteStream> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(Self::object_key(address, format))
            .send()
            .await
            .map_err(Error::internal)?;
        Ok(output.body)
    }
    fn object_key(address: &ContentAddress, format: Format) -> String {
        format!("constructs/{}.{}", address.hash(), format.as_str())
    }
}

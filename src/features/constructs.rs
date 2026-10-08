pub(crate) mod model;
pub use model::Record;

use crate::{
    error::{Error, Result},
    features::{database::Database, storage::Storage, validation::Validator},
};
use futures_util::Stream;
use model::{ContentAddress, Format, NewRecord, UploadName};
use std::{fmt::Display, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

/// Coordinates the construct lifecycle without knowing HTTP or SQL details.
#[derive(Clone)]
pub(crate) struct Registry(Arc<Inner>);
struct Inner {
    database: Database,
    storage: Storage,
    validator: Validator,
    uploads: Semaphore,
}

pub(crate) struct UploadResult {
    pub created: bool,
    pub record: Record,
}
pub(crate) struct Download {
    pub record: Record,
    pub body: aws_sdk_s3::primitives::ByteStream,
    pub filename: String,
}

impl Registry {
    pub fn new(database: Database, storage: Storage, validator: Validator) -> Self {
        Self(Arc::new(Inner {
            database,
            storage,
            validator,
            uploads: Semaphore::new(2),
        }))
    }
    pub async fn check_dependencies(&self) -> Result<()> {
        self.0.database.check_available().await?;
        self.0.validator.check_available().await
    }

    pub async fn upload<S, B, E>(&self, name: UploadName, stream: S) -> Result<UploadResult>
    where
        S: Stream<Item = std::result::Result<B, E>> + Unpin,
        B: AsRef<[u8]>,
        E: Display,
    {
        let _permit = self
            .0
            .uploads
            .try_acquire()
            .map_err(|_| Error::UploadCapacity)?;
        tokio::time::timeout(Duration::from_secs(120), self.receive(name, stream))
            .await
            .map_err(|_| Error::UploadTimeout)?
    }
    async fn receive<S, B, E>(&self, name: UploadName, stream: S) -> Result<UploadResult>
    where
        S: Stream<Item = std::result::Result<B, E>> + Unpin,
        B: AsRef<[u8]>,
        E: Display,
    {
        let staged = self.0.storage.stage(&name, stream).await?;
        let key = format!("incoming/{}.{}", uuid::Uuid::new_v4(), name.extension);
        // Always attempt remote cleanup on success and ordinary errors. Cancellation/crashes
        // are covered by the bucket's incoming lifecycle rules.
        let result = async {
            self.0.storage.upload_incoming(&staged, &key).await?;
            let validation = self
                .0
                .validator
                .validate(
                    self.0.storage.bucket(),
                    &key,
                    staged.address.hash(),
                    staged.size_bytes,
                )
                .await?;
            let format = validation.format;
            let record = NewRecord {
                address: staged.address.clone(),
                filename: name.filename,
                format,
                size_bytes: staged.size_bytes,
                default_prim: validation.default_prim,
                prim_count: validation.prim_count,
            };
            self.0
                .database
                .insert_if_absent(
                    record,
                    self.0.storage.publish(&key, &staged.address, format),
                )
                .await
        }
        .await;
        self.0.storage.cleanup(&key).await;
        let (created, record) = result?;
        Ok(UploadResult { created, record })
    }
    pub async fn record(&self, address: String) -> Result<Record> {
        let address = ContentAddress::parse(address)?;
        self.0.database.get(&address).await?.ok_or(Error::NotFound)
    }
    pub async fn list(&self, limit: u32, offset: u32) -> Result<Vec<Record>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::BadRequest("limit must be 1..200".into()));
        }
        self.0.database.list(limit, offset).await
    }
    pub async fn download(&self, address: String) -> Result<Download> {
        let record = self.record(address).await?;
        let address = ContentAddress::parse(record.address.clone())?;
        let format = Format::parse(&record.format)?;
        let body = self.0.storage.read(&address, format).await?;
        let filename = format!("{}.{}", address.hash(), format.as_str());
        Ok(Download {
            record,
            body,
            filename,
        })
    }
}

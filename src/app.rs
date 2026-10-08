use crate::{
    config::{secret, AuthSecret, Config, DatabaseSecret},
    error::{Error, Result},
    features::{
        api, constructs::Registry, database::Database, storage::Storage, validation::Validator,
    },
};
use std::sync::Arc;

#[derive(Clone)]
pub struct App {
    pub(crate) registry: Registry,
    pub(crate) upload_key: Arc<str>,
    pub(crate) read_key: Arc<str>,
}
impl App {
    pub async fn open(config: Config) -> Result<Self> {
        let aws = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .timeout_config(
                aws_config::timeout::TimeoutConfig::builder()
                    .connect_timeout(std::time::Duration::from_secs(5))
                    .operation_timeout(std::time::Duration::from_secs(70))
                    .build(),
            )
            .load()
            .await;
        let secrets = aws_sdk_secretsmanager::Client::new(&aws);
        let db: DatabaseSecret = secret(&secrets, &config.database_secret).await?;
        let auth: AuthSecret = secret(&secrets, &config.auth_secret).await?;
        if db.host != config.database_host
            || db.dbname != "construct_registry"
            || db.username != "construct_app"
        {
            return Err(Error::internal("database secret must match the configured endpoint, construct_registry database and construct_app user"));
        }
        if auth.upload_api_key.len() < 32
            || auth.read_api_key.len() < 32
            || auth.upload_api_key == auth.read_api_key
        {
            return Err(Error::internal(
                "use two distinct API keys of at least 32 characters",
            ));
        }
        let database = Database::open(&db, &config.database_ca).await?;
        Ok(Self {
            registry: Registry::new(
                database,
                Storage::new(aws_sdk_s3::Client::new(&aws), config.bucket),
                Validator::new(aws_sdk_lambda::Client::new(&aws), config.validator_function),
            ),
            upload_key: auth.upload_api_key.into(),
            read_key: auth.read_api_key.into(),
        })
    }
    pub async fn check_dependencies(&self) -> Result<()> {
        self.registry.check_dependencies().await
    }
    pub fn router(self) -> axum::Router {
        api::router(self)
    }
}

use crate::error::{Error, Result};
use serde::Deserialize;
use std::{env, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub bucket: String,
    pub validator_function: String,
    pub database_secret: String,
    pub auth_secret: String,
    pub database_host: String,
    pub database_ca: PathBuf,
}
impl Config {
    pub fn from_env() -> Result<Self> {
        fn required(name: &str) -> Result<String> {
            env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| Error::BadRequest(format!("set {name} before starting")))
        }
        Ok(Self {
            bucket: required("CONSTRUCT_BUCKET")?,
            validator_function: required("CONSTRUCT_VALIDATOR_FUNCTION")?,
            database_secret: required("CONSTRUCT_DATABASE_SECRET")?,
            auth_secret: required("CONSTRUCT_AUTH_SECRET")?,
            database_host: required("CONSTRUCT_DATABASE_HOST")?,
            database_ca: required("CONSTRUCT_DATABASE_CA")?.into(),
        })
    }
}
#[derive(Deserialize)]
pub(crate) struct DatabaseSecret {
    pub host: String,
    pub port: u16,
    pub dbname: String,
    pub username: String,
    pub password: String,
}
#[derive(Deserialize)]
pub(crate) struct AuthSecret {
    pub upload_api_key: String,
    pub read_api_key: String,
}
pub(crate) async fn secret<T: serde::de::DeserializeOwned>(
    client: &aws_sdk_secretsmanager::Client,
    id: &str,
) -> Result<T> {
    let output = client
        .get_secret_value()
        .secret_id(id)
        .send()
        .await
        .map_err(|_| Error::internal("could not retrieve application secret"))?;
    serde_json::from_str(
        output
            .secret_string()
            .ok_or_else(|| Error::internal("application secret must contain JSON text"))?,
    )
    .map_err(|_| Error::internal("application secret has invalid JSON or missing fields"))
}

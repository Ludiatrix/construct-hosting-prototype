use crate::{
    error::{Error, Result},
    features::constructs::model::Format,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) struct Validator {
    client: aws_sdk_lambda::Client,
    function: String,
}
#[derive(Deserialize)]
pub(crate) struct Validation {
    pub format: Format,
    pub default_prim: String,
    pub prim_count: u64,
}
impl Validator {
    pub fn new(client: aws_sdk_lambda::Client, function: String) -> Self {
        Self { client, function }
    }
    async fn invoke(&self, event: Value) -> Result<Value> {
        let output = self
            .client
            .invoke()
            .function_name(&self.function)
            .invocation_type(aws_sdk_lambda::types::InvocationType::RequestResponse)
            .payload(aws_sdk_lambda::primitives::Blob::new(
                serde_json::to_vec(&event).map_err(Error::internal)?,
            ))
            .send()
            .await
            .map_err(Error::internal)?;
        if output.function_error().is_some() {
            return Err(Error::internal("validator Lambda execution failed"));
        }
        serde_json::from_slice(
            output
                .payload()
                .ok_or_else(|| Error::internal("validator returned no payload"))?
                .as_ref(),
        )
        .map_err(Error::internal)
    }
    pub async fn check_available(&self) -> Result<()> {
        let output = self.invoke(json!({"action":"health"})).await?;
        if output["status"] != "ready" || output["protocol_version"] != 1 {
            return Err(Error::internal(
                "deploy the real validator image before starting the API",
            ));
        }
        Ok(())
    }
    pub async fn validate(
        &self,
        bucket: &str,
        key: &str,
        hash: &str,
        size: u64,
    ) -> Result<Validation> {
        parse_validation(
            self.invoke(json!({"action":"validate", "bucket":bucket, "key":key,
            "sha256":hash, "size_bytes":size}))
                .await?,
            hash,
            size,
        )
    }
}
fn parse_validation(output: Value, hash: &str, size: u64) -> Result<Validation> {
    match output["status"].as_str() {
        Some("invalid") => {
            return Err(Error::InvalidConstruct(
                output["error"]
                    .as_str()
                    .unwrap_or("invalid USD construct")
                    .chars()
                    .take(1000)
                    .collect(),
            ))
        }
        Some("valid") => {}
        _ => {
            return Err(Error::internal(
                "validator returned an unsupported response",
            ))
        }
    }
    if output["sha256"].as_str() != Some(hash) || output["size_bytes"].as_u64() != Some(size) {
        return Err(Error::internal(
            "validator content hash or size did not match the upload",
        ));
    }
    let validation: Validation =
        serde_json::from_value(output["validation"].clone()).map_err(Error::internal)?;
    if !validation.default_prim.starts_with('/') || validation.prim_count == 0 {
        return Err(Error::internal("validator returned invalid metadata"));
    }
    Ok(validation)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_placeholder_and_mismatched_content() {
        assert!(parse_validation(json!({"status":"not_implemented"}), "abc", 5).is_err());
        let response = json!({"status":"valid","sha256":"abc","size_bytes":5,
            "validation":{"format":"usda","default_prim":"/Crate","prim_count":1}});
        assert!(parse_validation(response.clone(), "wrong", 5).is_err());
        assert!(parse_validation(response.clone(), "abc", 6).is_err());
        assert!(parse_validation(response, "abc", 5).is_ok());
    }
}

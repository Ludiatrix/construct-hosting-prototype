use crate::{
    config::DatabaseSecret,
    error::{Error, Result},
    features::constructs::model::{ContentAddress, NewRecord, Record},
};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode},
    PgPool, Row,
};
use std::{future::Future, path::Path, time::Duration};

pub(crate) struct Database {
    pub(crate) pool: PgPool,
}
const COLUMNS: &str = "address,filename,format,size_bytes,to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at,default_prim,prim_count";
impl Database {
    pub async fn open(secret: &DatabaseSecret, ca: &Path) -> Result<Self> {
        let options = PgConnectOptions::new()
            .host(&secret.host)
            .port(secret.port)
            .database(&secret.dbname)
            .username(&secret.username)
            .password(&secret.password)
            .ssl_mode(PgSslMode::VerifyFull)
            .ssl_root_cert(ca)
            .options([("search_path", "registry,public")]);
        let pool = PgPoolOptions::new().max_connections(5).acquire_timeout(Duration::from_secs(10))
            .connect_with(options).await.map_err(|_| Error::internal("PostgreSQL connection failed; check the application secret, TLS CA and security group"))?;
        sqlx::migrate!().run(&pool).await.map_err(Error::internal)?;
        Ok(Self { pool })
    }
    pub async fn check_available(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(Error::internal)?;
        Ok(())
    }
    pub async fn get(&self, address: &ContentAddress) -> Result<Option<Record>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM registry.constructs WHERE address=$1"
        ))
        .bind(address.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(Error::internal)?
        .map(read_record)
        .transpose()
    }
    pub async fn list(&self, limit: u32, offset: u32) -> Result<Vec<Record>> {
        sqlx::query(&format!("SELECT {COLUMNS} FROM registry.constructs ORDER BY created_at DESC,address DESC LIMIT $1 OFFSET $2"))
            .bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&self.pool).await
            .map_err(Error::internal)?.into_iter().map(read_record).collect()
    }
    // Cross-instance deduplication. Publish only after validation, and commit metadata last.
    pub async fn insert_if_absent<F: Future<Output = Result<()>>>(
        &self,
        record: NewRecord,
        publish: F,
    ) -> Result<(bool, Record)> {
        let mut tx = self.pool.begin().await.map_err(Error::internal)?;
        let lock =
            u64::from_str_radix(&record.address.hash()[..16], 16).map_err(Error::internal)? as i64;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *tx)
            .await
            .map_err(Error::internal)?;
        let select = format!("SELECT {COLUMNS} FROM registry.constructs WHERE address=$1");
        if let Some(row) = sqlx::query(&select)
            .bind(record.address.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(Error::internal)?
        {
            tx.commit().await.map_err(Error::internal)?;
            return Ok((false, read_record(row)?));
        }
        publish.await?;
        sqlx::query("INSERT INTO registry.constructs(address,filename,format,size_bytes,default_prim,prim_count) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(record.address.as_str()).bind(&record.filename).bind(record.format.as_str())
            .bind(i64::try_from(record.size_bytes).map_err(Error::internal)?)
            .bind(&record.default_prim).bind(i64::try_from(record.prim_count).map_err(Error::internal)?)
            .execute(&mut *tx).await.map_err(Error::internal)?;
        let row = sqlx::query(&select)
            .bind(record.address.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(Error::internal)?;
        tx.commit().await.map_err(Error::internal)?;
        Ok((true, read_record(row)?))
    }
}
fn read_record(row: PgRow) -> Result<Record> {
    Ok(Record {
        address: row.try_get("address").map_err(Error::internal)?,
        filename: row.try_get("filename").map_err(Error::internal)?,
        format: row.try_get("format").map_err(Error::internal)?,
        size_bytes: u64::try_from(
            row.try_get::<i64, _>("size_bytes")
                .map_err(Error::internal)?,
        )
        .map_err(Error::internal)?,
        created_at: row.try_get("created_at").map_err(Error::internal)?,
        default_prim: row.try_get("default_prim").map_err(Error::internal)?,
        prim_count: u64::try_from(
            row.try_get::<i64, _>("prim_count")
                .map_err(Error::internal)?,
        )
        .map_err(Error::internal)?,
    })
}

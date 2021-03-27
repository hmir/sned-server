use chrono::NaiveDateTime;
use serde::Serialize;

use crate::schema::{authenticators, file_metadata_items, transfer_records, users};

#[derive(Debug, Clone, Queryable, Insertable)]
pub struct User {
    pub id: String,
    pub username: String,
    pub public_key: String,
    pub signing_public_key: String,
    pub transfer_count: i32,
    pub transfer_quota: i64,
    pub date_registered: NaiveDateTime,
    pub approved: bool,
}

#[derive(Debug, Clone, Queryable, Insertable)]
pub struct Authenticator {
    pub id: String,
    pub authenticator: String,
    pub expiration_date: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Insertable)]
pub struct TransferRecord {
    pub id: String,
    pub download_id: String,
    pub sender: String,
    pub recipient: String,
    pub shared_key: String,
    pub shared_header: String,
    pub shared_nonce: String,
    pub file_id: String,
    pub date_created: NaiveDateTime,
    pub expires_on: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Insertable)]
pub struct FileMetadataItem {
    pub id: String,
    pub uploader: String,
    pub file_name: String,
    pub file_url: String,
    pub file_size: i64,
    pub date_created: NaiveDateTime,
}

#[derive(Debug, Clone, Queryable, Serialize)]
pub struct TransferRecordWithFileMetadata {
    pub download_id: String,
    pub sender: String,
    pub shared_key: String,
    pub shared_header: String,
    pub shared_nonce: String,
    pub date_created: NaiveDateTime,
    pub file_name: String,
    pub file_size: i64,
}

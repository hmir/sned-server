use bigdecimal::ToPrimitive;
use chrono::offset::Utc;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::result::Error;
use uuid::Uuid;

use crate::models;

fn generate_timestamp() -> NaiveDateTime {
    Utc::now().naive_utc()
}

pub fn get_user_from_username(
    username_to_query: &str,
    conn: &PgConnection,
) -> Result<Option<models::User>, Error> {
    use crate::schema::users::dsl::*;
    match users
        .filter(username.eq(username_to_query))
        .first::<models::User>(conn)
    {
        Ok(user) => Ok(Some(user)),
        Err(Error::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn insert_new_user(
    new_username: &str,
    new_public_key: &str,
    new_signing_public_key: &str,
    conn: &PgConnection,
) -> Result<String, Error> {
    use crate::schema::users::dsl::*;

    let approval_required: bool = std::env::var("REQUIRE_APPROVAL")
        .expect("REQUIRE_APPROVAL")
        .parse()
        .unwrap();
    let quota = std::env::var("DEFAULT_QUOTA_BYTES").expect("DEFAULT_QUOTA_BYTES");
    let new_id = Uuid::new_v4().to_string();
    let new_user = models::User {
        id: new_id.clone(),
        username: new_username.to_string(),
        public_key: new_public_key.to_string(),
        signing_public_key: new_signing_public_key.to_string(),
        transfer_count: 0,
        transfer_quota: quota.parse::<i64>().unwrap(),
        date_registered: generate_timestamp(),
        approved: !approval_required,
    };

    diesel::insert_into(users).values(&new_user).execute(conn)?;
    Ok(new_id)
}

pub fn update_user_transfer_count(user_id: &str, conn: &PgConnection) -> Result<i32, Error> {
    use crate::schema::users::dsl::*;

    let queried_user = users.filter(id.eq(user_id));

    let next_count = queried_user.first::<models::User>(conn)?.transfer_count + 1;
    diesel::update(queried_user)
        .set(transfer_count.eq(next_count))
        .get_result::<models::User>(conn)?;

    Ok(next_count)
}

pub fn get_all_approved_users(conn: &PgConnection) -> Result<Vec<String>, Error> {
    use crate::schema::users::dsl::*;
    users
        .filter(approved.eq(true))
        .select(username)
        .get_results(conn)
}

pub fn get_used_quota(user_id: &str, conn: &PgConnection) -> Result<u64, Error> {
    use crate::schema::file_metadata_items::dsl::*;
    let used_quota: Option<bigdecimal::BigDecimal> = file_metadata_items
        .filter(uploader.eq(user_id))
        .select(diesel::dsl::sum(file_size))
        .first(conn)?;

    if used_quota.is_none() {
        Ok(0)
    } else {
        Ok(used_quota.unwrap().to_u64().unwrap())
    }
}

pub fn get_transfer_records_and_file_metadata_for_recipient(
    recipient_id: &str,
    conn: &PgConnection,
) -> Result<Vec<models::TransferRecordWithFileMetadata>, Error> {
    use crate::schema::transfer_records::dsl::*;
    transfer_records
        .filter(recipient.eq(recipient_id))
        .inner_join(crate::schema::file_metadata_items::dsl::file_metadata_items)
        .inner_join(crate::schema::users::dsl::users.on(sender.eq(crate::schema::users::dsl::id)))
        .select((
            download_id,
            crate::schema::users::dsl::username,
            shared_key,
            shared_header,
            shared_nonce,
            date_created,
            crate::schema::file_metadata_items::dsl::file_name,
            crate::schema::file_metadata_items::dsl::file_size,
        ))
        .get_results(conn)
}

pub fn insert_new_transfer_record(
    sender_id: &str,
    recipient_id: &str,
    new_shared_key: &str,
    new_shared_header: &str,
    new_shared_nonce: &str,
    new_file_id: &str,
    conn: &PgConnection,
) -> Result<String, Error> {
    use crate::schema::transfer_records::dsl::*;

    let cur_timestamp = generate_timestamp();
    let lifetime = std::env::var("DEFAULT_FILE_LIFETIME_MINS").expect("DEFAULT_FILE_LIFETIME_MINS");
    let expiry_date = cur_timestamp + chrono::Duration::minutes(lifetime.parse().unwrap());

    let new_id = Uuid::new_v4().to_string();
    let new_transfer_record = models::TransferRecord {
        id: new_id.clone(),
        download_id: update_user_transfer_count(recipient_id, conn)?.to_string(),
        sender: sender_id.to_string(),
        recipient: recipient_id.to_string(),
        shared_key: new_shared_key.to_string(),
        shared_header: new_shared_header.to_string(),
        shared_nonce: new_shared_nonce.to_string(),
        file_id: new_file_id.to_string(),
        date_created: Utc::now().naive_utc(),
        expires_on: expiry_date,
    };

    diesel::insert_into(transfer_records)
        .values(&new_transfer_record)
        .execute(conn)?;

    Ok(new_id)
}

pub fn insert_new_file_metadata_item(
    uploader_id: &str,
    new_file_name: &str,
    new_file_url: &str,
    new_file_size: i64,
    conn: &PgConnection,
) -> Result<String, Error> {
    use crate::schema::file_metadata_items::dsl::*;

    let cur_timestamp = Utc::now().naive_utc();

    let new_id = Uuid::new_v4().to_string();
    let new_file_metadata_item = models::FileMetadataItem {
        id: new_id.clone(),
        uploader: uploader_id.to_string(),
        file_name: new_file_name.to_string(),
        file_url: new_file_url.to_string(),
        file_size: new_file_size,
        date_created: cur_timestamp,
    };

    diesel::insert_into(file_metadata_items)
        .values(&new_file_metadata_item)
        .execute(conn)?;

    Ok(new_id)
}

pub fn get_file_metadata_by_recipient_and_download_id(
    user_id: &str,
    dl_id: &str,
    conn: &PgConnection,
) -> Result<models::FileMetadataItem, Error> {
    use crate::schema::transfer_records::dsl::*;
    let record = transfer_records
        .filter(recipient.eq(user_id).and(download_id.eq(dl_id)))
        .first::<models::TransferRecord>(conn)?;

    get_file_metadata_item(&record.file_id, conn)
}

pub fn delete_transfer_record(record_id: &str, conn: &PgConnection) -> Result<(), Error> {
    use crate::schema::transfer_records::dsl::*;
    diesel::delete(transfer_records.filter(id.eq(record_id))).execute(conn)?;
    Ok(())
}

pub fn delete_transfer_record_by_user_id_and_download_id(
    user_id: &str,
    dl_id: &str,
    conn: &PgConnection,
) -> Result<models::TransferRecord, Error> {
    use crate::schema::transfer_records::dsl::*;
    let record_query = transfer_records.filter(recipient.eq(user_id).and(download_id.eq(dl_id)));

    let record = record_query.first::<models::TransferRecord>(conn)?;

    diesel::delete(record_query).execute(conn)?;
    Ok(record)
}

pub fn get_file_metadata_item(
    file_id: &str,
    conn: &PgConnection,
) -> Result<models::FileMetadataItem, Error> {
    use crate::schema::file_metadata_items::dsl::*;
    let metadata = file_metadata_items
        .filter(id.eq(file_id))
        .first::<models::FileMetadataItem>(conn)?;

    Ok(metadata)
}

pub fn is_file_unreferenced(file_id_val: &str, conn: &PgConnection) -> Result<bool, Error> {
    use crate::schema::transfer_records::dsl::*;
    let num_references = transfer_records
        .filter(file_id.eq(file_id_val))
        .select(diesel::dsl::count_star())
        .first::<i64>(conn)?;

    Ok(num_references == 0)
}

pub fn delete_file_metadata_item(file_id: &str, conn: &PgConnection) -> Result<(), Error> {
    use crate::schema::file_metadata_items::dsl::*;
    diesel::delete(file_metadata_items.filter(id.eq(file_id))).execute(conn)?;
    Ok(())
}

pub fn get_all_authenticators(conn: &PgConnection) -> Result<Vec<models::Authenticator>, Error> {
    use crate::schema::authenticators::dsl::*;
    authenticators.order(expiration_date).get_results(conn)
}

pub fn delete_authenticator(authenticator_id: &str, conn: &PgConnection) -> Result<(), Error> {
    use crate::schema::authenticators::dsl::*;
    diesel::delete(authenticators.filter(id.eq(authenticator_id))).execute(conn)?;
    Ok(())
}

pub fn insert_new_authenticator(
    authenticator_value: &str,
    expiry_date: NaiveDateTime,
    conn: &PgConnection,
) -> Result<(), Error> {
    use crate::schema::authenticators::dsl::*;
    let new_authenticator = models::Authenticator {
        id: Uuid::new_v4().to_string(),
        authenticator: authenticator_value.to_string(),
        expiration_date: expiry_date,
    };
    diesel::insert_into(authenticators)
        .values(&new_authenticator)
        .execute(conn)?;
    Ok(())
}

pub fn get_expired_transfer_records(
    conn: &PgConnection,
) -> Result<Vec<models::TransferRecord>, Error> {
    use crate::schema::transfer_records::dsl::*;
    let now = generate_timestamp();
    transfer_records
        .filter(expires_on.le(now))
        .get_results(conn)
}

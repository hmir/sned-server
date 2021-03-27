#[macro_use]
extern crate diesel;

use actix_web::{error, get, middleware, post, web, App, Error, HttpResponse, HttpServer};
use chrono::{Duration, NaiveDateTime};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use futures::{StreamExt, TryStreamExt};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use sodiumoxide::base64;
use sodiumoxide::crypto::sign;
use std::collections::HashSet;
use std::convert::TryInto;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_util::codec::{BytesCodec, FramedRead};
use uuid::Uuid;

mod actions;
mod models;
mod schema;

const AUTHENTICATOR_LEN: usize = 16;
const AUTHENTICATOR_LIFESPAN_SECONDS: i64 = 5 * 60;
const AUTHENTICATOR_GRACE_PERIOD_SECONDS: i64 = 2 * 60;
const CLEANUP_PROCESS_SLEEP_SECONDS: i64 = 5 * 60;

const USERNAME_MAX_LEN: i32 = 15;
const FILE_NAME_MAX_LEN: i32 = 20;

#[derive(Deserialize, Clone)]
struct TransferRequestMetadataPayload {
    sender: String,
    recipients: Vec<String>,
    name: String,
    shared_keys: Vec<String>,
    shared_header: String,
    shared_nonce: String,
    data_len: u64,
    signed_authenticator: String,
}

#[derive(Deserialize)]
struct DownloadRequestPayload {
    username: String,
    download_id: String,
    signed_authenticator: String,
}

#[derive(Deserialize)]
struct RegisterRequestPayload {
    username: String,
    public_key: String,
    signing_public_key: String,
}

#[derive(Deserialize)]
struct LookupRequestPayload {
    username: String,
}

#[derive(Deserialize)]
struct InboxRequestPayload {
    username: String,
    signed_authenticator: String,
}

#[derive(Deserialize)]
struct ListUsersRequestPayload {
    username: String,
    signed_authenticator: String,
}

#[derive(Serialize)]
struct InboxItem {
    timestamp: i64,
    sender: String,
    name: String,
    shared_key: String,
    shared_header: String,
}

#[derive(Serialize)]
struct InboxItemList {
    items: Vec<models::TransferRecordWithFileMetadata>,
}

type DbPool = r2d2::Pool<ConnectionManager<PgConnection>>;
type DbConnection = r2d2::PooledConnection<ConnectionManager<PgConnection>>;

fn get_db_conn(pool: &web::Data<DbPool>) -> DbConnection {
    pool.get().expect("Couldn't get db connection from pool")
}

async fn get_user_for_username(
    pool: &web::Data<DbPool>,
    username: &str,
) -> Result<models::User, Error> {
    let conn = get_db_conn(pool);
    let username_string = username.to_string();
    let user = web::block(move || {
        let user = actions::get_user_from_username(&username_string, &conn)?;
        Ok::<Option<models::User>, diesel::result::Error>(user)
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    if let None = user {
        Err(error::ErrorBadRequest(
            "User ".to_string() + username + " does not exist",
        ))
    } else {
        Ok(user.unwrap())
    }
}

async fn read_next_payload_chunk_into_buffer(
    payload: &mut web::Payload,
    buffer: &mut web::BytesMut,
) -> Result<(), Error> {
    let chunk = payload.next().await;
    if let Some(chunk) = chunk {
        buffer.extend_from_slice(&chunk?);
        Ok(())
    } else {
        return Err(error::ErrorBadRequest("Malformed Request"));
    }
}

async fn get_transfer_request_metadata_size(
    payload: &mut web::Payload,
    buffer: &mut web::BytesMut,
) -> Result<u32, Error> {
    while buffer.len() < 8 {
        read_next_payload_chunk_into_buffer(payload, buffer).await?;
    }
    let metadata_size_bytes = buffer.split_to(8);
    Ok(u64::from_be_bytes(metadata_size_bytes.as_ref().try_into().unwrap()) as u32)
}

async fn get_transfer_request_metadata(
    metadata_size: u32,
    payload: &mut web::Payload,
    buffer: &mut web::BytesMut,
) -> Result<TransferRequestMetadataPayload, Error> {
    while buffer.len() < metadata_size as usize {
        read_next_payload_chunk_into_buffer(payload, buffer).await?;
    }
    let metadata_bytes = buffer.split_to(metadata_size as usize);
    let metadata_str = String::from_utf8_lossy(metadata_bytes.as_ref());
    let metadata = serde_json::from_str(&metadata_str)
        .map_err(|_| error::ErrorBadRequest("Could not parse request"))?;
    Ok(metadata)
}

fn try_delete_file(file_path: &PathBuf) {
    std::fs::remove_file(file_path).ok();
}

fn validate_bytes_written(
    file_path: &PathBuf,
    bytes_written: u64,
    data_len: u64,
) -> Result<(), Error> {
    if bytes_written > data_len {
        try_delete_file(file_path);
        Err(error::ErrorBadRequest("Size of data exceeds expected size"))
    } else {
        Ok(())
    }
}

async fn write_transfer_request_data_to_file(
    file_path: &PathBuf,
    data_len: u64,
    payload: &mut web::Payload,
    buffer: &mut web::BytesMut,
) -> Result<(), Error> {
    let file = File::create(&file_path).expect(&format!(
        "File with name {:?} could not be created at the directory {:?}",
        file_path.file_name().unwrap(),
        file_path.parent().unwrap()
    ));
    let mut bytes_written = buffer.len() as u64;
    validate_bytes_written(&file_path, bytes_written, data_len)?;
    let mut writer = BufWriter::new(file);
    while let Some(chunk) = payload.next().await {
        let chunk = chunk?;
        bytes_written += chunk.len() as u64;
        validate_bytes_written(&file_path, bytes_written, data_len)?;
        writer.write(&chunk).unwrap();
    }

    if bytes_written != data_len {
        return Err(error::ErrorBadRequest(
            "File length does not match expected length",
        ));
    }

    writer.flush().unwrap();
    Ok(())
}

async fn add_transfer_records(
    pool: &web::Data<DbPool>,
    transfer_metadata: TransferRequestMetadataPayload,
    file_id: &str,
) -> Result<(), Error> {
    let conn = get_db_conn(pool);

    let file_id = file_id.to_string();
    web::block(move || {
        let sender = actions::get_user_from_username(&transfer_metadata.sender, &conn)?.unwrap();
        let mut recipients_added = HashSet::new();

        for i in 0..transfer_metadata.recipients.len() {
            let recipient =
                actions::get_user_from_username(&transfer_metadata.recipients[i], &conn)?.unwrap();

            if recipients_added.contains(&recipient.id.clone()) {
                continue;
            }

            recipients_added.insert(recipient.id.clone());

            actions::insert_new_transfer_record(
                &sender.id,
                &recipient.id,
                &transfer_metadata.shared_keys[i],
                &transfer_metadata.shared_header,
                &transfer_metadata.shared_nonce,
                &file_id,
                &conn,
            )?;
        }

        Ok::<(), diesel::result::Error>(())
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().body("Could not parse transfer request"))?;

    Ok(())
}

async fn add_file_metadata_item(
    pool: &web::Data<DbPool>,
    transfer_metadata: TransferRequestMetadataPayload,
) -> Result<(String, String), Error> {
    let conn = get_db_conn(pool);

    let (file_id, file_url) = web::block(move || {
        let file_url = Uuid::new_v4().to_string();

        let uploader = actions::get_user_from_username(&transfer_metadata.sender, &conn)?.unwrap();

        let file_id = actions::insert_new_file_metadata_item(
            &uploader.id,
            &transfer_metadata.name,
            &file_url.clone(),
            transfer_metadata.data_len as i64,
            &conn,
        )?;

        Ok::<(String, String), diesel::result::Error>((file_id, file_url))
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    Ok((file_id, file_url))
}

async fn verify_wont_exceed_quota(
    pool: &web::Data<DbPool>,
    user: models::User,
    new_file_size: u64,
) -> Result<(), Error> {
    let conn = get_db_conn(pool);
    let exceeded = web::block(move || {
        let quota_used = actions::get_used_quota(&user.id, &conn)?;
        Ok::<bool, diesel::result::Error>(quota_used + new_file_size > user.transfer_quota as u64)
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    if exceeded {
        Err(error::ErrorForbidden("Transfer quota reached"))
    } else {
        Ok(())
    }
}

#[post("/transfer")]
async fn transfer(
    pool: web::Data<DbPool>,
    mut payload: web::Payload,
) -> Result<HttpResponse, Error> {
    let mut buffer = web::BytesMut::new();
    let metadata_size = get_transfer_request_metadata_size(&mut payload, &mut buffer).await?;
    let metadata = get_transfer_request_metadata(metadata_size, &mut payload, &mut buffer).await?;
    let user = get_user_for_username(&pool, &metadata.sender).await?;

    if metadata.name.len() > FILE_NAME_MAX_LEN as usize {
        return Ok(HttpResponse::BadRequest().body(format!(
            "Name field must not exceed {} characters",
            FILE_NAME_MAX_LEN
        )));
    }

    authenticate_user(&pool, &user, &metadata.signed_authenticator).await?;

    let data_len = metadata.data_len;

    verify_wont_exceed_quota(&pool, user.clone(), data_len).await?;

    // add file metadata item
    let (file_id, file_url) = add_file_metadata_item(&pool, metadata.clone()).await?;

    // write file
    let write_path =
        PathBuf::from(std::env::var("STORAGE_DIR").expect("STORAGE_DIR") + "/" + &file_url);
    write_transfer_request_data_to_file(&write_path, data_len, &mut payload, &mut buffer)
        .await
        .map_err(|e| {
            std::fs::remove_file(write_path).unwrap_or(());
            e
        })?;

    // add transfer records
    add_transfer_records(&pool, metadata.clone(), &file_id).await?;

    Ok::<HttpResponse, Error>(HttpResponse::Ok().body("Transfer Completed"))
}

async fn get_file_metadata(
    pool: &web::Data<DbPool>,
    user_id: String,
    download_id: String,
) -> Result<models::FileMetadataItem, Error> {
    let conn = get_db_conn(pool);
    web::block(move || {
        let file_metadata =
            actions::get_file_metadata_by_recipient_and_download_id(&user_id, &download_id, &conn)?;
        Ok::<models::FileMetadataItem, diesel::result::Error>(file_metadata)
    })
    .await
    .map_err(|e| match e {
        error::BlockingError::Error(diesel::result::Error::NotFound) => {
            error::ErrorBadRequest("Invalid download id")
        }
        _ => error::ErrorInternalServerError(""),
    })
}

async fn clean_up_transfer_record(
    pool: &web::Data<DbPool>,
    user_id: String,
    download_id: String,
    file_metadata_id: String,
) -> Result<(), Error> {
    let conn = get_db_conn(pool);
    web::block(move || {
        actions::delete_transfer_record_by_user_id_and_download_id(&user_id, &download_id, &conn)?;
        if actions::is_file_unreferenced(&file_metadata_id, &conn)? {
            actions::delete_file_metadata_item(&file_metadata_id, &conn)?;
        }
        Ok::<(), diesel::result::Error>(())
    })
    .await
    .map_err(|_| error::ErrorInternalServerError("Server error"))
}

#[post("/download")]
async fn download(
    pool: web::Data<DbPool>,
    payload: web::Json<DownloadRequestPayload>,
) -> Result<HttpResponse, Error> {
    let user = get_user_for_username(&pool, &payload.username).await?;

    authenticate_user(&pool, &user, &payload.signed_authenticator).await?;

    let user_id = user.id;

    let file_metadata =
        get_file_metadata(&pool, user_id.clone(), payload.download_id.clone()).await?;

    let file_path =
        std::env::var("STORAGE_DIR").expect("STORAGE_DIR") + "/" + &file_metadata.file_url;
    let file = tokio::fs::File::open(file_path.clone())
        .await
        .map_err(|_| {
            HttpResponse::InternalServerError()
                .body("Could not find file, the file may have expired")
        })?;

    clean_up_transfer_record(
        &pool,
        user_id.clone(),
        payload.download_id.clone(),
        file_metadata.id.clone(),
    )
    .await?;

    let stream = FramedRead::new(file, BytesCodec::new()).map_ok(web::BytesMut::freeze);

    try_delete_file(&PathBuf::from(file_path));

    Ok(HttpResponse::Ok().streaming(stream))
}

#[post("/inbox")]
async fn inbox(
    pool: web::Data<DbPool>,
    payload: web::Json<InboxRequestPayload>,
) -> Result<HttpResponse, Error> {
    let user = get_user_for_username(&pool, &payload.username).await?;
    authenticate_user(&pool, &user, &payload.signed_authenticator).await?;
    let user_id = user.id;
    let conn = get_db_conn(&pool);
    let items = web::block(move || {
        let items = actions::get_transfer_records_and_file_metadata_for_recipient(&user_id, &conn)?;
        Ok::<Vec<models::TransferRecordWithFileMetadata>, diesel::result::Error>(items)
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    let response = serde_json::to_string(&InboxItemList { items }).unwrap();

    Ok(HttpResponse::Ok().body(response))
}

#[post("/register")]
async fn register(
    pool: web::Data<DbPool>,
    payload: web::Json<RegisterRequestPayload>,
) -> Result<HttpResponse, Error> {
    let conn = get_db_conn(&pool);

    if payload.username.len() > USERNAME_MAX_LEN as usize {
        return Ok(HttpResponse::BadRequest().body(format!(
            "Username field must not exceed {} characters",
            USERNAME_MAX_LEN
        )));
    }

    let user_inserted = web::block(move || {
        if let Some(_) = actions::get_user_from_username(&payload.username, &conn)? {
            return Ok(false);
        }

        actions::insert_new_user(
            &payload.username,
            &payload.public_key,
            &payload.signing_public_key,
            &conn,
        )?;
        Ok::<bool, diesel::result::Error>(true)
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    if user_inserted {
        Ok(HttpResponse::Ok().finish())
    } else {
        Ok(HttpResponse::Conflict().body("Username taken"))
    }
}

fn generate_authenticator_value() -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(AUTHENTICATOR_LEN)
        .map(char::from)
        .collect()
}

fn now() -> NaiveDateTime {
    NaiveDateTime::from_timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        0,
    )
}

fn add_new_authenticator(conn: &PgConnection) -> Result<(), diesel::result::Error> {
    actions::insert_new_authenticator(
        &generate_authenticator_value(),
        now() + Duration::seconds(AUTHENTICATOR_LIFESPAN_SECONDS),
        conn,
    )
}

async fn update_and_get_authenticators(
    pool: &web::Data<DbPool>,
) -> Result<Vec<models::Authenticator>, Error> {
    let conn = get_db_conn(pool);
    let authenticators = web::block(move || {
        let mut authenticators = actions::get_all_authenticators(&conn)?;
        let now = now();

        while authenticators.len() < 2 {
            add_new_authenticator(&conn)?;
            authenticators = actions::get_all_authenticators(&conn)?;
        }

        while authenticators.len() > 2 {
            actions::delete_authenticator(&authenticators[0].id, &conn)?;
            authenticators = actions::get_all_authenticators(&conn)?;
        }

        let grace_period = Duration::seconds(AUTHENTICATOR_GRACE_PERIOD_SECONDS);

        if authenticators[0].expiration_date + grace_period < now {
            actions::delete_authenticator(&authenticators[0].id, &conn)?;
            add_new_authenticator(&conn)?;

            if authenticators[1].expiration_date + grace_period < now {
                actions::delete_authenticator(&authenticators[1].id, &conn)?;
                add_new_authenticator(&conn)?;
            }
        }

        authenticators = actions::get_all_authenticators(&conn)?;
        Ok::<Vec<models::Authenticator>, diesel::result::Error>(authenticators)
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    Ok(authenticators)
}

#[get("/authenticator")]
async fn authenticator(pool: web::Data<DbPool>) -> Result<HttpResponse, Error> {
    let authenticators = update_and_get_authenticators(&pool).await?;
    Ok(HttpResponse::Ok().body(&authenticators.last().unwrap().authenticator))
}

async fn verify_signed_authenticator(
    pool: &web::Data<DbPool>,
    signing_public_key_str: &str,
    signed_authenticator: &str,
) -> Result<(), Error> {
    let authenticators = update_and_get_authenticators(&pool).await?;
    let public_key_bytes = base64::decode(&signing_public_key_str, base64::Variant::Original)
        .map_err(|_| HttpResponse::Unauthorized().body("Authentication failed"))?;

    let signed_authenticator_bytes =
        base64::decode(&signed_authenticator, base64::Variant::Original)
            .map_err(|_| HttpResponse::Unauthorized().body("Authentication failed"))?;

    let public_key = sign::PublicKey::from_slice(&public_key_bytes);
    if public_key.is_none() {
        return Err(error::ErrorUnauthorized("Authentication failed"));
    }

    let verified = sign::verify(&signed_authenticator_bytes, &public_key.unwrap())
        .map_err(|_| HttpResponse::Unauthorized().body("Authentication failed"))?;

    if verified == authenticators[0].authenticator.as_bytes().to_vec()
        || verified == authenticators[1].authenticator.as_bytes().to_vec()
    {
        return Ok(());
    }

    Err(error::ErrorUnauthorized("Authentication failed"))
}

async fn authenticate_user(
    pool: &web::Data<DbPool>,
    user: &models::User,
    signed_authenticator: &str,
) -> Result<(), Error> {
    if !user.approved {
        Err(error::ErrorUnauthorized(
            "Your account has yet to be approved by the server administrator",
        ))
    } else {
        verify_signed_authenticator(pool, &user.signing_public_key, signed_authenticator).await
    }
}

#[post("/lookup")]
async fn lookup(
    pool: web::Data<DbPool>,
    payload: web::Json<LookupRequestPayload>,
) -> Result<HttpResponse, Error> {
    let conn = get_db_conn(&pool);
    let public_key = web::block(move || {
        if let Some(user) = actions::get_user_from_username(&payload.username, &conn)? {
            return Ok(Some(user.public_key));
        }

        Ok::<Option<String>, diesel::result::Error>(None)
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    if let Some(public_key) = public_key {
        Ok(HttpResponse::Ok().body(public_key))
    } else {
        Ok(HttpResponse::BadRequest().body("User does not exist"))
    }
}

#[post("/list-users")]
async fn list_users(
    pool: web::Data<DbPool>,
    payload: web::Json<ListUsersRequestPayload>,
) -> Result<HttpResponse, Error> {
    let user = get_user_for_username(&pool, &payload.username).await?;
    authenticate_user(&pool, &user, &payload.signed_authenticator).await?;

    let conn = get_db_conn(&pool);
    let users = web::block(move || {
        let users = actions::get_all_approved_users(&conn)?;

        Ok::<String, diesel::result::Error>(users.join("\n"))
    })
    .await
    .map_err(|_| HttpResponse::InternalServerError().finish())?;

    Ok(HttpResponse::Ok().body(users))
}

fn create_file_cleanup_process(pool: DbPool) {
    std::thread::spawn(move || loop {
        let conn = pool.get().expect("Couldn't get db connection from pool");
        let mut file_deletion_queue = HashSet::new();
        let expired_transfer_records =
            actions::get_expired_transfer_records(&conn).unwrap_or(Vec::new());

        for record in expired_transfer_records {
            if let Err(_) = actions::delete_transfer_record(&record.id, &conn) {
                continue;
            }

            if let Ok(true) = actions::is_file_unreferenced(&record.file_id, &conn) {
                file_deletion_queue.insert(record.file_id.clone());
            }
        }

        for file_id in &file_deletion_queue {
            if let Ok(file_metadata_item) = actions::get_file_metadata_item(file_id, &conn) {
                if let Ok(_) = actions::delete_file_metadata_item(file_id, &conn) {
                    let file_path = std::env::var("STORAGE_DIR").expect("STORAGE_DIR")
                        + "/"
                        + &file_metadata_item.file_url;
                    try_delete_file(&PathBuf::from(file_path));
                }
            }
        }

        drop(conn);
        std::thread::sleep(std::time::Duration::from_secs(
            CLEANUP_PROCESS_SLEEP_SECONDS as u64,
        ));
    });
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();
    dotenv::dotenv().ok();
    sodiumoxide::init().unwrap();

    // Set up database connection pool
    let connspec = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let manager = ConnectionManager::<PgConnection>::new(connspec);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");

    create_file_cleanup_process(pool.clone());

    HttpServer::new(move || {
        App::new()
            .data(pool.clone())
            .wrap(middleware::Logger::default())
            .service(register)
            .service(transfer)
            .service(download)
            .service(inbox)
            .service(lookup)
            .service(authenticator)
            .service(list_users)
    })
    .bind("0.0.0.0:8000")?
    .run()
    .await
}

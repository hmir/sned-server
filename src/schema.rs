table! {
    authenticators (id) {
        id -> Varchar,
        authenticator -> Varchar,
        expiration_date -> Timestamp,
    }
}

table! {
    file_metadata_items (id) {
        id -> Varchar,
        uploader -> Varchar,
        file_name -> Varchar,
        file_url -> Varchar,
        file_size -> Int8,
        date_created -> Timestamp,
    }
}

table! {
    transfer_records (id) {
        id -> Varchar,
        download_id -> Varchar,
        sender -> Varchar,
        recipient -> Varchar,
        shared_key -> Varchar,
        shared_header -> Varchar,
        shared_nonce -> Varchar,
        file_id -> Varchar,
        date_created -> Timestamp,
        expires_on -> Timestamp,
    }
}

table! {
    users (id) {
        id -> Varchar,
        username -> Varchar,
        public_key -> Varchar,
        signing_public_key -> Varchar,
        transfer_count -> Int4,
        transfer_quota -> Int8,
        date_registered -> Timestamp,
        approved -> Bool,
    }
}

joinable!(file_metadata_items -> users (uploader));
joinable!(transfer_records -> file_metadata_items (file_id));

allow_tables_to_appear_in_same_query!(authenticators, file_metadata_items, transfer_records, users,);

CREATE TABLE transfer_records (
  id VARCHAR NOT NULL PRIMARY KEY,
  download_id VARCHAR NOT NULL,
  sender VARCHAR NOT NULL REFERENCES users(id),
  recipient VARCHAR NOT NULL REFERENCES users(id),
  shared_key VARCHAR NOT NULL,
  shared_header VARCHAR NOT NULL,
  shared_nonce VARCHAR NOT NULL,  
  file_id VARCHAR NOT NULL REFERENCES file_metadata_items(id),
  date_created TIMESTAMP NOT NULL,
  expires_on TIMESTAMP NOT NULL
)

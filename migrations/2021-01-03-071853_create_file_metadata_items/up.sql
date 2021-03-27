CREATE TABLE file_metadata_items (
  id VARCHAR NOT NULL PRIMARY KEY,
  uploader VARCHAR NOT NULL REFERENCES users(id),
  file_name VARCHAR NOT NULL,
  file_url VARCHAR NOT NULL,
  file_size BIGINT NOT NULL,
  date_created TIMESTAMP NOT NULL
)

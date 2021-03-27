CREATE TABLE users (
  id VARCHAR NOT NULL PRIMARY KEY,
  username VARCHAR NOT NULL,
  public_key VARCHAR NOT NULL,
  signing_public_key VARCHAR NOT NULL,  
  transfer_count INTEGER NOT NULL,
  transfer_quota BIGINT NOT NULL,
  date_registered TIMESTAMP NOT NULL,
  approved BOOLEAN NOT NULL
)

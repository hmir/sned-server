CREATE TABLE authenticators (
  id VARCHAR NOT NULL PRIMARY KEY,
  authenticator VARCHAR NOT NULL,
  expiration_date TIMESTAMP NOT NULL
)

# sned-server
This is the server component of [sned](http://github.com/sned).

## Setup
Install and set up [PostgreSQL](https://www.postgresql.org/). You should make the your unix user a postgres superuser.

Install [diesel-cli](https://diesel.rs/).

Run `diesel setup` in the terminal in the same directory as .env. Consult [this link](https://stackoverflow.com/questions/17996957/fe-sendauth-no-password-supplied) if you get an fe_sendauth error.

Generate a private key and certificate with names 'key.pem' and 'cert.pem'  (e.g. `openssl req -x509 -newkey rsa:4096 -nodes -keyout key.pem -out cert.pem`)

## Usage
Prior to starting the server, you can configure the .env file to your liking:

| Variable | Description | 
| -------- | ----------- | 
| DATABSE_URL | The postgres database URL the server will use |
| STORAGE_DIR | File path where uploaded files will be stored |
| REQUIRE_APPROVAL | Specifies whether the "approved" column must manually be set to `true` for each user to allow them to upload |
| DEFAULT_FILE_LIFETIME_MINS | How long uploaded files will live |
| DEFAULT_QUOTA_BYTES | Default allotted number of bytes that each user can upload to the server |

Run `sned-server` to start the server

## Disclaimer
This project was built for educational purposes only. The developer(s) do not take responsibility for any data that is lost or stolen while using the app.


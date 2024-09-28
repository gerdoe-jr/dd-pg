Setup for tests:

```sql
CREATE USER 'ddnet-account-test'@localhost IDENTIFIED BY 'test';
CREATE DATABASE ddnet_account_test;
GRANT ALL PRIVILEGES ON ddnet_account_test.* TO 'ddnet-account-test'@localhost;
```

There has to be a smtp server running (for fake emails)

To enhance security the database must support TLS connections (for non localhost):
- For MySQL in `/etc/mysql/my.cnf` (or `.conf`) add 
    ```cfg
    [mysqld]
    ssl_ca = /etc/mysql/ssl/ca-cert.pem
    ssl_cert = /etc/mysql/ssl/server-cert.pem
    ssl_key = /etc/mysql/ssl/server-key.pem
    ```

    after creating the required keys:
    ```bash
    sudo mkdir -p /etc/mysql/ssl
    # generate ca key & cert
    sudo bash -c "openssl genrsa 2048 > /etc/mysql/ssl/ca-key.pem"
    sudo bash -c "openssl req -sha256 -new -x509 -nodes -key /etc/mysql/ssl/ca-key.pem -subj \"/CN=localhost\" -days 36500 > /etc/mysql/ssl/ca-cert.pem"
    # generate server key & csr
    sudo bash -c "openssl req -sha256 -newkey rsa:2048 -nodes -keyout /etc/mysql/ssl/server-key.pem -subj \"/CN=localhost\" -addext \"subjectAltName = DNS:localhost,DNS:localhost\" -addext \"basicConstraints = CA:FALSE\" -addext \"keyUsage = digitalSignature, keyEncipherment\" -addext \"extendedKeyUsage = serverAuth, clientAuth\" > /etc/mysql/ssl/server-req.pem"
    sudo bash -c "openssl rsa -in /etc/mysql/ssl/server-key.pem -out /etc/mysql/ssl/server-key.pem"
    # generate server cert
    sudo bash -c "openssl x509 -days 36500 -sha256 -req -copy_extensions=copyall -in /etc/mysql/ssl/server-req.pem  -CA /etc/mysql/ssl/ca-cert.pem -CAkey /etc/mysql/ssl/ca-key.pem -set_serial 01 > /etc/mysql/ssl/server-cert.pem"
    sudo chown -R mysql:mysql /etc/mysql/ssl
    # reading certs is allowed for everyone
    sudo chmod -R 666 /etc/mysql/ssl
    sudo chmod 777 /etc/mysql/ssl
    # but the server key and ca key stay secret
    sudo chmod 600 /etc/mysql/ssl/server-key.pem
    sudo chmod 600 /etc/mysql/ssl/ca-key.pem
    ```

SQL formatting is doing with `sleek -n <file>`.

Allow & deny lists for ip bans, mail domain bans & allow lists are automatically reloaded on change.
It's strongly recommended to create the files somewhere else and only use `mv` to overwrite the files, since
file system operations are rather racy, which in worst case can lead to loading a partially written file.
(The watcher tries to minimize these events by only listening for write-close events and similar, but can't prevent
all cases.)


After executing `./account-server --setup` there should be a `config` directory.
Inside that directory the ban & allow lists are placed. Additionally the email templates
are rooted there.
To use the existing email template do:
```
cp templates/email/template.html config/account_tokens.html
cp templates/email/template.html config/credential_auth_tokens.html
```

Tests must be executed with:
```
cargo test -p account-server -- --test-threads=1
```

Since all tests cleanup the database after being done and otherwise this operation would be racy.

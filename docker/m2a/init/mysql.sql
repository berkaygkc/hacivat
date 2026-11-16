-- M2a fixtures on every MySQL and MariaDB server (R17). Runs once, at first start, as root,
-- after the entrypoint has created `hv_test` and its database `hv_test`.
--
-- - `orders`, `orders_archive`: the extra databases the database picker and explorer list.
-- - `hv_locked`: an account that exists but is locked (ER_ACCOUNT_HAS_BEEN_LOCKED, 3118 / 4151).
-- - `hv_ro`: read-only grants on the three fixture databases.
-- All passwords are the throwaway test password.
CREATE DATABASE IF NOT EXISTS orders;
CREATE DATABASE IF NOT EXISTS orders_archive;
GRANT ALL PRIVILEGES ON orders.* TO 'hv_test'@'%';
GRANT ALL PRIVILEGES ON orders_archive.* TO 'hv_test'@'%';

CREATE USER 'hv_locked'@'%' IDENTIFIED BY 'hv_test_pw' ACCOUNT LOCK;
GRANT SELECT ON hv_test.* TO 'hv_locked'@'%';

CREATE USER 'hv_ro'@'%' IDENTIFIED BY 'hv_test_pw';
GRANT SELECT, SHOW VIEW ON hv_test.* TO 'hv_ro'@'%';
GRANT SELECT, SHOW VIEW ON orders.* TO 'hv_ro'@'%';
GRANT SELECT, SHOW VIEW ON orders_archive.* TO 'hv_ro'@'%';

FLUSH PRIVILEGES;

-- ## m2b-t16-fixtures
USE hv_test;
CREATE TABLE IF NOT EXISTS hv_probe (a int);
CREATE TABLE IF NOT EXISTS hv_probe_log (id int AUTO_INCREMENT PRIMARY KEY, v int);

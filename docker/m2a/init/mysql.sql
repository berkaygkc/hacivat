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

# ## m4-t01c-fixtures
# M4 accounts (R-M4-43; effective on fresh volumes, controller step C-2). Every grant below was
# chosen from the MySQL 8.0/8.4 and MariaDB manuals; ADR 0004 `m4-t01c` records why each is
# the least that does its job.
#
# - `hv_test` owns every database whose name starts with `hvm4_` (each M4 test creates its own
#   and drops it). `\_` is a literal underscore in a grant's database pattern.
GRANT ALL PRIVILEGES ON `hvm4\_%`.* TO 'hv_test'@'%';

# - `hv_catalog`: the system-catalog reader of the snapshot generator (HV-INTRO-16). It reads
#   the system schemas (information_schema shows an object only to an account with a privilege
#   on it) and writes nothing. No grant on any fixture database.
#   `hv_catalog_probe` is an empty database where it holds EXECUTE only: the default database
#   of the built-in-function probe (`PREPARE ... FROM 'SELECT <name>()'`). An unqualified name
#   that is not a built-in names a stored function of the default database, and the server
#   checks EXECUTE on it before it looks the function up: without the privilege a missing
#   function answers 1370 (EXECUTE command denied), which the probe cannot tell from a present
#   one; with it, 1305 (FUNCTION does not exist). The database holds no routine, so a probe
#   never finds a stored function there, and `sys` (which has stored functions named like
#   built-ins of other servers) is never the probe's default database (carry R-C1-1).
CREATE DATABASE IF NOT EXISTS hv_catalog_probe;
CREATE USER 'hv_catalog'@'%' IDENTIFIED BY 'hv_test_pw';
GRANT SELECT ON mysql.* TO 'hv_catalog'@'%';
GRANT SELECT ON performance_schema.* TO 'hv_catalog'@'%';
GRANT SELECT, SHOW VIEW, EXECUTE ON sys.* TO 'hv_catalog'@'%';
GRANT EXECUTE ON hv_catalog_probe.* TO 'hv_catalog'@'%';

# - `hv_fixture`: test set-up only (through `hacivat-testkit`), never product code. Two jobs:
#   * `SET GLOBAL time_zone`: MySQL needs SYSTEM_VARIABLES_ADMIN (the narrow replacement of
#     SUPER); MariaDB has no such privilege and needs SUPER. The statement is chosen at run
#     time because neither server parses the other's privilege name.
#   * create, grant to, revoke from and drop `hvm4_*` test accounts: CREATE USER (global; it
#     cannot be limited to a name pattern, so test code names every account `hvm4_*`), and
#     GRANT OPTION with the privileges it grants, on `hvm4_*` databases only.
CREATE USER 'hv_fixture'@'%' IDENTIFIED BY 'hv_test_pw';
GRANT CREATE USER ON *.* TO 'hv_fixture'@'%';
GRANT ALL PRIVILEGES ON `hvm4\_%`.* TO 'hv_fixture'@'%' WITH GRANT OPTION;
SET @hv_m4_grant := IF(VERSION() LIKE '%MariaDB%',
    'GRANT SUPER ON *.* TO ''hv_fixture''@''%''',
    'GRANT SYSTEM_VARIABLES_ADMIN ON *.* TO ''hv_fixture''@''%''');
PREPARE hv_m4_grant FROM @hv_m4_grant;
EXECUTE hv_m4_grant;
DEALLOCATE PREPARE hv_m4_grant;
SET @hv_m4_grant := NULL;

FLUSH PRIVILEGES;

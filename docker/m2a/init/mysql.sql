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
# chosen from the MySQL 8.0/8.4 and MariaDB manuals; ADR 0004 section m4-t01c records why each
# is the least that does its job. No comment line below holds a quote character: the client
# reads this file in DELIMITER mode further down.
#
# - hv_test owns every database whose name starts with hvm4_ (each M4 test creates its own and
#   drops it). The backslash makes the underscore literal in a grant database pattern.
GRANT ALL PRIVILEGES ON `hvm4\_%`.* TO 'hv_test'@'%';

# - hv_catalog: the system-catalog reader of the snapshot generator (HV-INTRO-16). It reads the
#   system schemas (information_schema shows an object only to an account with a privilege on
#   it) and writes nothing. No grant on any fixture database. Note: SELECT on mysql.* includes
#   the password hashes of mysql.user (and global_priv on MariaDB); the accounts here are
#   throwaway test accounts (R-M4-43).
#   hv_catalog_probe is an empty database where it holds EXECUTE only: the default database of
#   the built-in-function probe, which prepares SELECT name() for each candidate name. An
#   unqualified name that is not a built-in names a stored function of the default database
#   (MySQL manual, Function Name Parsing and Resolution), and the server checks EXECUTE before
#   it looks the function up: without the privilege a missing function answers 1370 (EXECUTE
#   command denied), which the probe cannot tell from a present one; with it, 1305 (FUNCTION
#   does not exist). The database holds no routine, so a probe never finds a stored function
#   there; sys (whose stored functions carry names that are built-ins on some servers only) is
#   never the probe database (carry R-C1-1).
CREATE DATABASE IF NOT EXISTS hv_catalog_probe;
CREATE USER 'hv_catalog'@'%' IDENTIFIED BY 'hv_test_pw';
GRANT SELECT ON mysql.* TO 'hv_catalog'@'%';
GRANT SELECT ON performance_schema.* TO 'hv_catalog'@'%';
GRANT SELECT, SHOW VIEW, EXECUTE ON sys.* TO 'hv_catalog'@'%';
GRANT EXECUTE ON hv_catalog_probe.* TO 'hv_catalog'@'%';

# - hv_fixture: test set-up only (through hacivat-testkit), never product code. It holds EXECUTE
#   on hv_fixture_admin and nothing else, so it can do exactly two jobs, through the procedures
#   there (ruling I1 of the T1c review):
#   * set an allowed global variable (time_zone only): set_global(name, value);
#   * create, grant to, revoke from and drop hvm4_* accounts, with privileges from an allowlist
#     on hvm4_* databases (or one of their tables) only: create_user, drop_user,
#     change_grant.
#   The procedures are SQL SECURITY DEFINER; they refuse any other name, database, table,
#   privilege or variable before they build a statement, and they quote with QUOTE() (strings)
#   or accept only [a-z0-9_] (identifiers). CREATE USER, GRANT, REVOKE, DROP USER and SET can
#   be prepared (MySQL 8.4 manual, SQL Syntax Permitted in Prepared Statements; MariaDB from
#   10.6.2, PREPARE Statement: all statements), and a stored procedure may use PREPARE.
#   Parameters are ascii with ascii_bin, so comparisons and REGEXP are exact and case-sensitive.
#
# - hv_fixture_definer (localhost, ACCOUNT LOCK): the procedures run with its privileges. It can
#   never sign in: MySQL 8.4 manual, Account Locking: locking does not affect the ability to
#   execute stored programs that name the locked account as DEFINER; MariaDB, Account Locking:
#   a lock refuses new client connections. Its privileges are what the two jobs need:
#   * CREATE USER (global; MySQL: enables CREATE/DROP/RENAME/ALTER USER; MariaDB likewise). On
#     MySQL it cannot touch accounts with SYSTEM_USER (root); the procedures limit it to hvm4_*
#     on both servers;
#   * ALL on hvm4_* databases WITH GRANT OPTION: a grantor must hold each privilege it grants
#     plus GRANT OPTION (MySQL, GRANT OPTION; MariaDB, GRANT);
#   * the global-variable privilege: SYSTEM_VARIABLES_ADMIN on MySQL (System Variable
#     Privileges), SUPER on MariaDB, which has no narrower one (MariaDB, GRANT: SUPER sets
#     global system variables). Neither server parses the other privilege name, so the
#     statement is chosen from VERSION() and prepared.
CREATE DATABASE IF NOT EXISTS hv_fixture_admin;
CREATE USER 'hv_fixture_definer'@'localhost' ACCOUNT LOCK;
GRANT CREATE USER ON *.* TO 'hv_fixture_definer'@'localhost';
GRANT ALL PRIVILEGES ON `hvm4\_%`.* TO 'hv_fixture_definer'@'localhost' WITH GRANT OPTION;
SET @hv_m4_grant := IF(VERSION() LIKE '%MariaDB%',
    'GRANT SUPER ON *.* TO ''hv_fixture_definer''@''localhost''',
    'GRANT SYSTEM_VARIABLES_ADMIN ON *.* TO ''hv_fixture_definer''@''localhost''');
PREPARE hv_m4_grant FROM @hv_m4_grant;
EXECUTE hv_m4_grant;
DEALLOCATE PREPARE hv_m4_grant;
SET @hv_m4_grant := NULL;

DELIMITER //

CREATE DEFINER = 'hv_fixture_definer'@'localhost'
PROCEDURE hv_fixture_admin.set_global(
    IN p_name VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin,
    IN p_value VARCHAR(64))
SQL SECURITY DEFINER
BEGIN
    IF p_name IS NULL OR p_value IS NULL OR p_name NOT IN ('time_zone') THEN
        SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'hv_fixture_admin: variable refused';
    END IF;
    SET @hv_fixture_sql := CONCAT('SET GLOBAL ', p_name, ' = ', QUOTE(p_value));
    PREPARE hv_fixture_stmt FROM @hv_fixture_sql;
    SET @hv_fixture_sql := NULL;
    EXECUTE hv_fixture_stmt;
    DEALLOCATE PREPARE hv_fixture_stmt;
END//

CREATE DEFINER = 'hv_fixture_definer'@'localhost'
PROCEDURE hv_fixture_admin.create_user(
    IN p_user VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin,
    IN p_password VARCHAR(64))
SQL SECURITY DEFINER
BEGIN
    IF p_user IS NULL OR p_password IS NULL OR p_user NOT REGEXP '^hvm4_[a-z0-9_]{1,27}$' THEN
        SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'hv_fixture_admin: account refused';
    END IF;
    SET @hv_fixture_sql := CONCAT('CREATE USER ', QUOTE(p_user), '@''%'' IDENTIFIED BY ',
        QUOTE(p_password));
    PREPARE hv_fixture_stmt FROM @hv_fixture_sql;
    SET @hv_fixture_sql := NULL;
    EXECUTE hv_fixture_stmt;
    DEALLOCATE PREPARE hv_fixture_stmt;
END//

CREATE DEFINER = 'hv_fixture_definer'@'localhost'
PROCEDURE hv_fixture_admin.drop_user(
    IN p_user VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin)
SQL SECURITY DEFINER
BEGIN
    IF p_user IS NULL OR p_user NOT REGEXP '^hvm4_[a-z0-9_]{1,27}$' THEN
        SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'hv_fixture_admin: account refused';
    END IF;
    SET @hv_fixture_sql := CONCAT('DROP USER IF EXISTS ', QUOTE(p_user), '@''%''');
    PREPARE hv_fixture_stmt FROM @hv_fixture_sql;
    SET @hv_fixture_sql := NULL;
    EXECUTE hv_fixture_stmt;
    DEALLOCATE PREPARE hv_fixture_stmt;
END//

# GRANT (p_grant = 1) or REVOKE (p_grant = 0) one allowlisted privilege on database p_db (or
# its table p_table when it is not NULL) to or from the hvm4_* account p_user.
CREATE DEFINER = 'hv_fixture_definer'@'localhost'
PROCEDURE hv_fixture_admin.change_grant(
    IN p_grant BOOLEAN,
    IN p_privilege VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin,
    IN p_db VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin,
    IN p_table VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin,
    IN p_user VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin)
SQL SECURITY DEFINER
BEGIN
    IF p_grant IS NULL
        OR p_user IS NULL OR p_user NOT REGEXP '^hvm4_[a-z0-9_]{1,27}$'
        OR p_db IS NULL OR p_db NOT REGEXP '^hvm4_[a-z0-9_]{1,59}$'
        OR (p_table IS NOT NULL AND p_table NOT REGEXP '^[a-z0-9_]{1,64}$')
        OR p_privilege IS NULL OR p_privilege NOT IN ('SELECT', 'INSERT', 'UPDATE', 'DELETE',
            'EXECUTE', 'SHOW VIEW', 'TRIGGER', 'REFERENCES', 'CREATE', 'DROP', 'ALTER',
            'INDEX', 'CREATE VIEW', 'CREATE ROUTINE', 'ALTER ROUTINE', 'EVENT') THEN
        SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'hv_fixture_admin: grant refused';
    END IF;
    SET @hv_fixture_sql := CONCAT(
        IF(p_grant, 'GRANT ', 'REVOKE '), p_privilege,
        ' ON `', p_db, '`.', IF(p_table IS NULL, '*', CONCAT('`', p_table, '`')),
        IF(p_grant, ' TO ', ' FROM '), QUOTE(p_user), '@''%''');
    PREPARE hv_fixture_stmt FROM @hv_fixture_sql;
    SET @hv_fixture_sql := NULL;
    EXECUTE hv_fixture_stmt;
    DEALLOCATE PREPARE hv_fixture_stmt;
END//

DELIMITER ;

CREATE USER 'hv_fixture'@'%' IDENTIFIED BY 'hv_test_pw';
GRANT EXECUTE ON hv_fixture_admin.* TO 'hv_fixture'@'%';

FLUSH PRIVILEGES;

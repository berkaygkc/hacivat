-- mysql84-x509 only (T13 carry, m2b-fix-f2), after init/mysql.sql: `hv_x509` signs in only with
-- a client certificate the server's `ssl_ca` (the test CA `mysql-x509-certs` generated) trusts,
-- plus the throwaway test password. Read-only grants, like `hv_ro`.
CREATE USER 'hv_x509'@'%' IDENTIFIED BY 'hv_test_pw' REQUIRE X509;
GRANT SELECT, SHOW VIEW ON hv_test.* TO 'hv_x509'@'%';

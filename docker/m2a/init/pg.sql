-- M2a fixtures on every PostgreSQL server (R17): the extra databases the database picker
-- and the explorer list. Runs once, at initdb, as the `hv_test` superuser.
CREATE DATABASE orders OWNER hv_test;
CREATE DATABASE orders_archive OWNER hv_test;
